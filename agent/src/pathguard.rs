//! `allowed_roots` 路径访问范围限制（AGENT_DESIGN.md §五）：配置了白名单时，所有
//! 涉及路径的请求先校验目标路径是否落在白名单某一项之内。用词法规范化（不要求
//! 路径已存在——`create_dir`/`write_file` 经常是给一个还不存在的新路径）而不是
//! `fs::canonicalize`，防止 `..` 绕过。

use std::path::{Component, Path, PathBuf};

use roc_desk_protocol::ErrorCode;

/// 把路径按分隔符拆开、消化掉 `.`/`..`（`..` 在到达根之后不会继续往上跳出根），
/// 不接触文件系统——这是唯一能对"可能还不存在的路径"做的规范化方式。
pub fn lexical_normalize(path: &str) -> PathBuf {
    let mut out = PathBuf::new();
    for component in Path::new(path).components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn normalize_for_compare(path: &Path) -> String {
    path.to_string_lossy().replace('/', "\\").trim_end_matches('\\').to_lowercase()
}

/// `allowed_roots` 为空表示不限制。命中范围外返回 `OutsideAllowedRoots`。
pub fn check_allowed(path: &str, allowed_roots: &[String]) -> Result<PathBuf, ErrorCode> {
    let normalized = lexical_normalize(path);
    if allowed_roots.is_empty() {
        return Ok(normalized);
    }
    let compare = normalize_for_compare(&normalized);
    let allowed = allowed_roots.iter().any(|root| {
        compare == *root || compare.starts_with(&format!("{root}\\"))
    });
    if allowed {
        Ok(normalized)
    } else {
        Err(ErrorCode::OutsideAllowedRoots)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_allowlist_allows_everything() {
        assert!(check_allowed(r"C:\Windows\System32", &[]).is_ok());
    }

    #[test]
    fn blocks_outside_root() {
        let roots = vec![r"d:\projects".to_string()];
        assert!(check_allowed(r"C:\Windows", &roots).is_err());
    }

    #[test]
    fn allows_inside_root_case_insensitive() {
        let roots = vec![r"d:\projects".to_string()];
        assert!(check_allowed(r"D:\Projects\app\src", &roots).is_ok());
    }

    #[test]
    fn dotdot_cannot_escape_root() {
        let roots = vec![r"d:\projects".to_string()];
        // 词法规范化后 `..` 会把路径提回到 D:\projects 之外的 D:\，应当被拒绝。
        assert!(check_allowed(r"D:\projects\..\..\Windows", &roots).is_err());
    }
}
