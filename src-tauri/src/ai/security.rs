use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;
use sha2::{Digest, Sha256};

static SECRET_PATTERNS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    vec![
        (Regex::new(r"AKIA[0-9A-Z]{16}").unwrap(), "[REDACTED_AWS_ACCESS_KEY]"),
        (Regex::new(r"ASIA[0-9A-Z]{16}").unwrap(), "[REDACTED_AWS_STS_KEY]"),
        (Regex::new(r"(?s)-----BEGIN [A-Z ]*PRIVATE KEY-----.*?-----END [A-Z ]*PRIVATE KEY-----").unwrap(), "[REDACTED_PRIVATE_KEY]"),
        (Regex::new(r"eyJ[A-Za-z0-9_-]{8,}\.eyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}").unwrap(), "[REDACTED_JWT]"),
        (Regex::new(r"gh[pousr]_[A-Za-z0-9]{20,}").unwrap(), "[REDACTED_GITHUB_TOKEN]"),
        (Regex::new(r"glpat-[A-Za-z0-9_-]{20,}").unwrap(), "[REDACTED_GITLAB_TOKEN]"),
        (Regex::new(r"AIza[0-9A-Za-z_-]{30,}").unwrap(), "[REDACTED_GOOGLE_API_KEY]"),
        (Regex::new(r"(?i)(authorization\s*:\s*(?:bearer|basic)\s+)[^\s,;]+" ).unwrap(), "$1[REDACTED]"),
        (Regex::new(r"(?i)(cookie\s*:\s*)[^\r\n]+" ).unwrap(), "$1[REDACTED]"),
        (Regex::new(r#"(?i)((?:password|passwd|pwd|secret|token|api[_-]?key|access[_-]?key|secret[_-]?key|client[_-]?secret)\s*[\"']?\s*[:=]\s*[\"']?)[^\s\"',;}]+"#).unwrap(), "$1[REDACTED]"),
        (Regex::new(r"(?i)(https?://[^\s/:@]+:)[^\s/@]+@" ).unwrap(), "$1[REDACTED]@"),
        (Regex::new(r"(?i)([?&](?:token|access_token|api_key|key|secret|signature)=)[^&#\s]+" ).unwrap(), "$1[REDACTED]"),
    ]
});

pub fn redact(text: &str) -> String {
    SECRET_PATTERNS
        .iter()
        .fold(text.to_owned(), |value, (pattern, replacement)| {
            pattern.replace_all(&value, *replacement).into_owned()
        })
}

pub fn redact_json(value: &mut Value) {
    match value {
        Value::String(text) => *text = redact(text),
        Value::Array(values) => values.iter_mut().for_each(redact_json),
        Value::Object(values) => values.values_mut().for_each(redact_json),
        _ => {}
    }
}

pub fn audit_metadata(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    format!("bytes={}, sha256={:x}", text.len(), digest)
}

pub fn model_safe_error(error: &crate::error::AppError) -> String {
    let redacted = redact(&error.to_string());
    let shortened: String = redacted.chars().take(800).collect();
    format!("工具执行失败：{shortened}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_common_secrets() {
        let input = "Authorization: Bearer abc.def.ghi\npassword: hunter2\nghp_abcdefghijklmnopqrstuvwxyz123456";
        let output = redact(input);
        assert!(!output.contains("hunter2"));
        assert!(!output.contains("abcdefghijklmnopqrstuvwxyz123456"));
        assert!(!output.contains("abc.def.ghi"));
    }

    #[test]
    fn redacts_nested_json() {
        let mut value = serde_json::json!({"content": ["token=hello-world", {"value": "AKIA1234567890ABCDEF"}]});
        redact_json(&mut value);
        let encoded = value.to_string();
        assert!(!encoded.contains("hello-world"));
        assert!(!encoded.contains("AKIA1234567890ABCDEF"));
    }
}
