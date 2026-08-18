use serde::{Deserialize, Serialize};

use super::parser::parse_log_line;
use crate::error::AppError;
use crate::ssh::session::SshSession;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveSearchResult {
    pub file_path: String,
    pub line_number: i64,
    pub timestamp: Option<String>,
    pub log_level: Option<String>,
    pub line: String,
}

/// 单引号安全转义：把 `'` 换成 `'\''`，包在单引号里传给远程 shell，
/// 防止搜索关键词里的 shell 特殊字符（`;`、`` ` ``、`$(...)` 等）被解释执行
/// （命令注入是 OWASP Top 10 之一，这里的输入直接来自用户的搜索框，必须转义）。
pub(crate) fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// 模式 A：远程实时搜索（DESIGN.md §3.4.2），通过 SSH 执行 `rg`，没有就退回 `grep -rn`。
pub async fn search_live(
    session: &SshSession,
    pattern: &str,
    path: &str,
    is_regex: bool,
) -> Result<Vec<LiveSearchResult>, AppError> {
    let quoted_pattern = shell_quote(pattern);
    let quoted_path = shell_quote(path);

    let rg_flags = if is_regex { "" } else { "-F" };
    let cmd = format!(
        "rg --line-number --no-heading {rg_flags} -- {quoted_pattern} {quoted_path} 2>/dev/null \
         || grep -rn {} -- {quoted_pattern} {quoted_path} 2>/dev/null",
        if is_regex { "-E" } else { "-F" }
    );

    let output = session.exec(&cmd).await?;
    Ok(parse_grep_output(&output))
}

/// 解析 `rg`/`grep` 的 `path:line:content` 输出格式。
fn parse_grep_output(output: &str) -> Vec<LiveSearchResult> {
    output
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, ':');
            let file_path = parts.next()?.to_string();
            let line_number: i64 = parts.next()?.parse().ok()?;
            let content = parts.next()?.to_string();
            let parsed = parse_log_line(&content);
            Some(LiveSearchResult {
                file_path,
                line_number,
                timestamp: parsed.timestamp,
                log_level: parsed.level,
                line: content,
            })
        })
        .take(500)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_embedded_single_quotes() {
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
    }

    #[test]
    fn parses_ripgrep_style_output() {
        let out = "/var/log/app.log:42:2026-08-17 ERROR boom\n/var/log/app.log:43:next line";
        let results = parse_grep_output(out);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].file_path, "/var/log/app.log");
        assert_eq!(results[0].line_number, 42);
        assert_eq!(results[0].log_level.as_deref(), Some("ERROR"));
    }
}
