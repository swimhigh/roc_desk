use std::net::IpAddr;
use std::sync::LazyLock;
use std::time::Duration;

use futures_util::StreamExt;
use regex::Regex;

use crate::error::AppError;

const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;
const MAX_OUTPUT_CHARS: usize = 8000;
const FETCH_TIMEOUT: Duration = Duration::from_secs(15);

// Rust 的 regex crate 不支持反向引用，`<script>`/`<style>` 各自单独匹配，
// 不能像 PCRE 那样用 `<(script|style)>...</\1>` 一个正则搞定。
static SCRIPT_BLOCK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?is)<script[^>]*>.*?</script>").unwrap());
static STYLE_BLOCK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?is)<style[^>]*>.*?</style>").unwrap());
static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<[^>]+>").unwrap());
static BLANK_LINES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[ \t]*\n(?:[ \t]*\n)+").unwrap());

/// `webfetch` 工具（`coding/tools.rs` 的 `ToolCall::WebFetch`）：抓取一个 URL 并
/// 转成模型能读的纯文本。只做"够用"级别的 HTML→文本转换（正则剥标签），不引入
/// html5ever/scraper 这类完整解析器——参考项目里 `ai/chat.rs` 处理 Bing RSS 结果
/// 时是同一个思路，不追求还原排版，只求信息可读。
pub async fn fetch_url(client: &reqwest::Client, url: &str) -> Result<String, AppError> {
    let parsed = reqwest::Url::parse(url).map_err(|e| AppError::Internal(format!("无效的 URL：{e}")))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(AppError::Internal("只支持 http/https URL".into()));
    }
    let host = parsed.host_str().ok_or_else(|| AppError::Internal("URL 缺少主机名".into()))?;
    if is_internal_host(host) {
        // 网页内容本身可能包含"访问 http://169.254.169.254/... 获取更多信息"这类
        // 提示注入，模型如果照做会让 webfetch 变成打内网/云元数据端点的跳板——
        // 这里在请求发出前就拒绝，而不是信任模型自己判断该不该访问。
        return Err(AppError::PermissionDenied(format!("出于安全考虑，拒绝访问内网/本地地址：{host}")));
    }

    let resp = tokio::time::timeout(
        FETCH_TIMEOUT,
        client.get(parsed).header(reqwest::header::USER_AGENT, "roc_desk/1.0 (AI webfetch)").send(),
    )
    .await
    .map_err(|_| AppError::Connection("webfetch 请求超时".into()))??;

    if !resp.status().is_success() {
        return Err(AppError::Connection(format!("HTTP {}", resp.status())));
    }
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();

    let mut body = Vec::new();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(AppError::from)?;
        if body.len() >= MAX_BODY_BYTES {
            break;
        }
        body.extend_from_slice(&chunk);
    }
    let text = String::from_utf8_lossy(&body).into_owned();
    let extracted = if content_type.contains("html") { html_to_text(&text) } else { text };
    Ok(extracted.chars().take(MAX_OUTPUT_CHARS).collect())
}

fn is_internal_host(host: &str) -> bool {
    let host = host.trim_matches(|c| c == '[' || c == ']');
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(v4)) => v4.is_loopback() || v4.is_private() || v4.is_link_local() || v4.is_unspecified(),
        // fc00::/7 是 IPv6 的唯一本地地址（ULA）范围，标准库目前没有 `is_unique_local` 稳定 API。
        Ok(IpAddr::V6(v6)) => v6.is_loopback() || v6.is_unspecified() || (v6.segments()[0] & 0xfe00) == 0xfc00,
        Err(_) => false,
    }
}

fn html_to_text(html: &str) -> String {
    let no_script = SCRIPT_BLOCK.replace_all(html, "");
    let no_style = STYLE_BLOCK.replace_all(&no_script, "");
    let no_tags = TAG.replace_all(&no_style, "\n");
    let decoded = decode_entities(&no_tags);
    BLANK_LINES.replace_all(decoded.trim(), "\n\n").to_string()
}

fn decode_entities(text: &str) -> String {
    text.replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_loopback_and_private_hosts() {
        assert!(is_internal_host("localhost"));
        assert!(is_internal_host("127.0.0.1"));
        assert!(is_internal_host("192.168.1.1"));
        assert!(is_internal_host("10.0.0.5"));
        assert!(is_internal_host("169.254.169.254"));
        assert!(!is_internal_host("example.com"));
        assert!(!is_internal_host("93.184.216.34"));
    }

    #[test]
    fn strips_script_and_tags() {
        let html = "<html><head><style>.a{}</style></head><body><script>evil()</script><p>Hello <b>World</b></p></body></html>";
        let text = html_to_text(html);
        assert!(text.contains("Hello"));
        assert!(text.contains("World"));
        assert!(!text.contains("evil"));
        assert!(!text.contains('<'));
    }
}
