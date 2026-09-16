use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::AppError;
use crate::fsops::FileOps;

const SKILLS_DIR: &str = ".rock_desk/skills";

/// 一个技能的元信息（不含正文，正文按需通过 `skill` 工具加载），供系统提示词
/// 里列"可用技能：name — description"这一行，模型据此决定要不要加载哪个。也是
/// `skill_list` 命令直接吐给前端的类型（Skills 管理弹窗的"查看"用它渲染列表），
/// 字段名保持 snake_case 和仓库里其它 Tauri command 返回类型的约定一致。
#[derive(Debug, Clone, Serialize)]
pub struct SkillMeta {
    pub name: String,
    pub description: String,
    /// 技能目录相对工作区根目录的路径，如 `.rock_desk/skills/demo`。
    pub dir: String,
}

/// 最小 YAML frontmatter 解析：`---` 分隔的头部逐行按 `key: value` 拆，不支持
/// 嵌套结构/多行值——`SKILL.md` 的 frontmatter 只用到 `name`/`description` 两个
/// 标量字段，不值得为此引入一个完整的 yaml crate。返回 `(字段表, 正文)`；
/// 找不到规范的 `---` 包裹头部时，整份内容当正文、字段表为空。
pub fn parse_frontmatter(text: &str) -> (HashMap<String, String>, &str) {
    let Some(rest) = text.strip_prefix("---") else {
        return (HashMap::new(), text);
    };
    let rest = rest.strip_prefix('\n').unwrap_or(rest);
    let Some(end) = rest.find("\n---") else {
        return (HashMap::new(), text);
    };
    let header = &rest[..end];
    let body = rest[end + 4..]
        .strip_prefix('\n')
        .unwrap_or(&rest[end + 4..]);

    let mut fields = HashMap::new();
    for line in header.lines() {
        if let Some((key, value)) = line.split_once(':') {
            fields.insert(
                key.trim().to_string(),
                value.trim().trim_matches('"').to_string(),
            );
        }
    }
    (fields, body)
}

/// 扫描 `<workspace_root>/.rock_desk/skills/*/SKILL.md`，解析每份的 frontmatter。
/// 单个技能目录解析失败（没有 SKILL.md、frontmatter 缺字段等）跳过，不影响其它
/// 技能被正常发现——和 `discover_skills` 的调用方（`coding_start`）一样，这是
/// "锦上添花"的能力，不应该因为一个格式错误的技能目录阻断整个会话启动。
pub async fn discover_skills(file_ops: &dyn FileOps, root: &str) -> Vec<SkillMeta> {
    let skills_root = format!("{}/{SKILLS_DIR}", root.trim_end_matches(['/', '\\']));
    let Ok(entries) = file_ops.list_dir(&skills_root).await else {
        return Vec::new();
    };

    let mut skills = Vec::new();
    for entry in entries.into_iter().filter(|e| e.is_dir) {
        let skill_md_path = format!("{}/SKILL.md", entry.path);
        let Ok(content) = file_ops.read_file(&skill_md_path).await else {
            continue;
        };
        let (fields, _body) = parse_frontmatter(&content.text);
        let name = fields
            .get("name")
            .cloned()
            .unwrap_or_else(|| entry.name.clone());
        let description = fields.get("description").cloned().unwrap_or_default();
        skills.push(SkillMeta {
            name,
            description,
            dir: entry.path.clone(),
        });
    }
    skills
}

/// `skill` 工具的执行体：按名称找到对应技能目录，读 `SKILL.md` 正文（frontmatter
/// 之后的部分）返回给模型。
pub async fn load_skill_body(
    file_ops: &dyn FileOps,
    skills: &[SkillMeta],
    name: &str,
) -> Result<String, AppError> {
    let meta = skills
        .iter()
        .find(|s| s.name == name)
        .ok_or_else(|| AppError::NotFound(format!("未找到技能：{name}")))?;
    let content = file_ops
        .read_file(&format!("{}/SKILL.md", meta.dir))
        .await?;
    let (_fields, body) = parse_frontmatter(&content.text);
    Ok(body.to_string())
}

/// `skill_import` 命令用来判断"这是压缩包还是已经解压好的文件夹"——只看扩展名，
/// 不嗅探文件头（导入这个动作本身就是用户主动选的文件，犯不上防对着一个改了后缀
/// 的假压缩包做额外校验）。
pub fn is_archive_path(path: &str) -> bool {
    let lower = path.to_lowercase();
    lower.ends_with(".zip") || lower.ends_with(".tar.gz") || lower.ends_with(".tgz")
}

/// 解压一个 Skills 压缩包（.zip/.tar.gz/.tgz）到 `dest_dir`（调用方负责准备一个
/// 全新的空目录，通常是临时目录，用完即删）。RAR 故意不支持——2026-09 用户决定：
/// 没有靠谱的纯 Rust RAR 解压库，要支持只能调用系统装的 WinRAR/7-Zip 或打包一份
/// unrar 可执行文件进便携版，而 Skills 包从来不会用 RAR 分发（GitHub/Claude
/// Skills 市场只出 zip/tar.gz），不值得为此新增外部依赖。
///
/// 返回压缩包里实际含 `SKILL.md` 的那个目录——很多压缩包（尤其是 GitHub 生成的
/// "Download ZIP"）会在里面多包一层同名文件夹（`my-skill-main/SKILL.md`，不是直接
/// `SKILL.md` 落在压缩包根目录），这里做一次探测：压缩包根目录直接有 `SKILL.md`
/// 就用根目录；没有的话找根目录下唯一一个自己含 `SKILL.md` 的子目录；两种都找不到
/// 或者根目录下有不止一个这样的子目录（分不清导入哪个）就报错，不瞎猜。
pub fn extract_skill_archive(archive_path: &Path, dest_dir: &Path) -> Result<PathBuf, AppError> {
    let lower = archive_path.to_string_lossy().to_lowercase();
    if lower.ends_with(".zip") {
        extract_zip(archive_path, dest_dir)?;
    } else if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        extract_tar_gz(archive_path, dest_dir)?;
    } else {
        return Err(AppError::Internal(format!(
            "不支持的压缩包格式（只支持 .zip/.tar.gz/.tgz）：{}",
            archive_path.display()
        )));
    }
    find_skill_root(dest_dir)
}

fn extract_zip(archive_path: &Path, dest_dir: &Path) -> Result<(), AppError> {
    let file = std::fs::File::open(archive_path)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| AppError::Internal(format!("zip 压缩包解析失败：{e}")))?;
    archive
        .extract(dest_dir)
        .map_err(|e| AppError::Internal(format!("zip 压缩包解压失败：{e}")))?;
    Ok(())
}

fn extract_tar_gz(archive_path: &Path, dest_dir: &Path) -> Result<(), AppError> {
    let file = std::fs::File::open(archive_path)?;
    let gz = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(gz);
    archive
        .unpack(dest_dir)
        .map_err(|e| AppError::Internal(format!("tar.gz 压缩包解压失败：{e}")))?;
    Ok(())
}

fn find_skill_root(dest_dir: &Path) -> Result<PathBuf, AppError> {
    if dest_dir.join("SKILL.md").is_file() {
        return Ok(dest_dir.to_path_buf());
    }
    let mut candidates = Vec::new();
    for entry in std::fs::read_dir(dest_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() && path.join("SKILL.md").is_file() {
            candidates.push(path);
        }
    }
    match candidates.len() {
        1 => Ok(candidates.remove(0)),
        0 => Err(AppError::Internal(
            "压缩包里没有找到 SKILL.md，不是一个合法的技能包".into(),
        )),
        _ => Err(AppError::Internal(
            "压缩包里有多个含 SKILL.md 的目录，无法确定要导入哪一个".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_frontmatter_and_body() {
        let text = "---\nname: demo\ndescription: 一个示例技能\n---\n\n这里是正文\n第二行\n";
        let (fields, body) = parse_frontmatter(text);
        assert_eq!(fields.get("name").map(String::as_str), Some("demo"));
        assert_eq!(
            fields.get("description").map(String::as_str),
            Some("一个示例技能")
        );
        assert!(body.contains("这里是正文"));
    }

    #[test]
    fn missing_frontmatter_returns_whole_text_as_body() {
        let text = "没有 frontmatter 的普通文本";
        let (fields, body) = parse_frontmatter(text);
        assert!(fields.is_empty());
        assert_eq!(body, text);
    }

    #[test]
    fn recognizes_archive_extensions() {
        assert!(is_archive_path("demo.zip"));
        assert!(is_archive_path("DEMO.ZIP"));
        assert!(is_archive_path("demo.tar.gz"));
        assert!(is_archive_path("demo.tgz"));
        assert!(!is_archive_path("demo"));
        assert!(!is_archive_path("demo.rar"));
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("roc_desk-skills-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn finds_skill_root_at_top_level() {
        let dir = temp_dir("root-level");
        std::fs::write(dir.join("SKILL.md"), "---\nname: demo\n---\nbody").unwrap();
        let found = find_skill_root(&dir).unwrap();
        assert_eq!(found, dir);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn finds_skill_root_one_level_nested() {
        let dir = temp_dir("nested");
        let inner = dir.join("my-skill-main");
        std::fs::create_dir_all(&inner).unwrap();
        std::fs::write(inner.join("SKILL.md"), "---\nname: demo\n---\nbody").unwrap();
        let found = find_skill_root(&dir).unwrap();
        assert_eq!(found, inner);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn errors_when_no_skill_md_found() {
        let dir = temp_dir("empty");
        std::fs::write(dir.join("readme.txt"), "nothing here").unwrap();
        assert!(find_skill_root(&dir).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
