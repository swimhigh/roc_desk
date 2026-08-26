use std::collections::HashMap;

use crate::error::AppError;
use crate::fsops::FileOps;

const SKILLS_DIR: &str = ".rock_desk/skills";

/// 一个技能的元信息（不含正文，正文按需通过 `skill` 工具加载），供系统提示词
/// 里列"可用技能：name — description"这一行，模型据此决定要不要加载哪个。
#[derive(Debug, Clone)]
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
    let Some(rest) = text.strip_prefix("---") else { return (HashMap::new(), text) };
    let rest = rest.strip_prefix('\n').unwrap_or(rest);
    let Some(end) = rest.find("\n---") else { return (HashMap::new(), text) };
    let header = &rest[..end];
    let body = rest[end + 4..].strip_prefix('\n').unwrap_or(&rest[end + 4..]);

    let mut fields = HashMap::new();
    for line in header.lines() {
        if let Some((key, value)) = line.split_once(':') {
            fields.insert(key.trim().to_string(), value.trim().trim_matches('"').to_string());
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
    let Ok(entries) = file_ops.list_dir(&skills_root).await else { return Vec::new() };

    let mut skills = Vec::new();
    for entry in entries.into_iter().filter(|e| e.is_dir) {
        let skill_md_path = format!("{}/SKILL.md", entry.path);
        let Ok(content) = file_ops.read_file(&skill_md_path).await else { continue };
        let (fields, _body) = parse_frontmatter(&content.text);
        let name = fields.get("name").cloned().unwrap_or_else(|| entry.name.clone());
        let description = fields.get("description").cloned().unwrap_or_default();
        skills.push(SkillMeta { name, description, dir: entry.path.clone() });
    }
    skills
}

/// `skill` 工具的执行体：按名称找到对应技能目录，读 `SKILL.md` 正文（frontmatter
/// 之后的部分）返回给模型。
pub async fn load_skill_body(file_ops: &dyn FileOps, skills: &[SkillMeta], name: &str) -> Result<String, AppError> {
    let meta = skills
        .iter()
        .find(|s| s.name == name)
        .ok_or_else(|| AppError::NotFound(format!("未找到技能：{name}")))?;
    let content = file_ops.read_file(&format!("{}/SKILL.md", meta.dir)).await?;
    let (_fields, body) = parse_frontmatter(&content.text);
    Ok(body.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_frontmatter_and_body() {
        let text = "---\nname: demo\ndescription: 一个示例技能\n---\n\n这里是正文\n第二行\n";
        let (fields, body) = parse_frontmatter(text);
        assert_eq!(fields.get("name").map(String::as_str), Some("demo"));
        assert_eq!(fields.get("description").map(String::as_str), Some("一个示例技能"));
        assert!(body.contains("这里是正文"));
    }

    #[test]
    fn missing_frontmatter_returns_whole_text_as_body() {
        let text = "没有 frontmatter 的普通文本";
        let (fields, body) = parse_frontmatter(text);
        assert!(fields.is_empty());
        assert_eq!(body, text);
    }
}
