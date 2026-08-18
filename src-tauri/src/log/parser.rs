use std::sync::LazyLock;

use regex::Regex;

/// 日志行解析：从一行原始文本里尽力抽取时间戳和级别（DESIGN.md §3.4）。
/// 覆盖常见格式即可（ISO8601、nginx 风格 `2026/08/17 03:21:05`），解析不出来
/// 就留空——搜索/展示都不强依赖这两个字段，只是有的话体验更好。
static TIMESTAMP_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\d{4}[-/]\d{2}[-/]\d{2}[T ]\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:?\d{2})?").unwrap()
});

static LEVEL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b(TRACE|DEBUG|INFO|WARN(?:ING)?|ERROR|FATAL|CRITICAL)\b").unwrap());

pub struct ParsedLine {
    pub timestamp: Option<String>,
    pub level: Option<String>,
}

pub fn parse_log_line(line: &str) -> ParsedLine {
    let timestamp = TIMESTAMP_RE.find(line).map(|m| m.as_str().to_string());
    let level = LEVEL_RE.find(line).map(|m| m.as_str().to_uppercase());
    ParsedLine { timestamp, level }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_iso8601_timestamp_and_level() {
        let parsed = parse_log_line("2026-08-17T10:22:08Z ERROR upstream timed out");
        assert_eq!(parsed.timestamp.as_deref(), Some("2026-08-17T10:22:08Z"));
        assert_eq!(parsed.level.as_deref(), Some("ERROR"));
    }

    #[test]
    fn extracts_nginx_style_timestamp() {
        let parsed = parse_log_line("2026/08/17 03:21:05 [error] upstream timed out");
        assert_eq!(parsed.timestamp.as_deref(), Some("2026/08/17 03:21:05"));
        assert_eq!(parsed.level.as_deref(), Some("ERROR"));
    }

    #[test]
    fn handles_lines_without_recognizable_fields() {
        let parsed = parse_log_line("just some plain text");
        assert!(parsed.timestamp.is_none());
        assert!(parsed.level.is_none());
    }
}
