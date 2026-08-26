use std::sync::LazyLock;

use futures_util::StreamExt;
use regex::Regex;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

use super::providers::AiProvider;
use crate::error::AppError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

static AWS_KEY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"AKIA[0-9A-Z]{16}").unwrap());
static PRIVATE_KEY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)-----BEGIN [A-Z ]*PRIVATE KEY-----.*?-----END [A-Z ]*PRIVATE KEY-----").unwrap()
});
static PASSWORD_KV: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i)(password|passwd|pwd|secret|token)\s*[:=]\s*['"]?[^\s'"]+"#).unwrap());
static SEARCH_ITEM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<item>(.*?)</item>").unwrap());
static SEARCH_TITLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<title>(.*?)</title>").unwrap());
static SEARCH_LINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<link>(.*?)</link>").unwrap());
static SEARCH_DESCRIPTION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)<description>(.*?)</description>").unwrap());
static XML_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]+>").unwrap());

/// 发送前的轻量脱敏 pass（DESIGN.md §3.6"数据出境风险"）：只覆盖高置信度的几类
/// 敏感信息——AWS Access Key、PEM 私钥块、`password=xxx` 形式的键值对——不追求
/// 识别所有可能的密钥格式，那需要一个专门的密钥扫描器，超出这里的范围。
pub fn redact(text: &str) -> String {
    let text = AWS_KEY.replace_all(text, "[REDACTED_AWS_KEY]");
    let text = PRIVATE_KEY.replace_all(&text, "[REDACTED_PRIVATE_KEY]");
    let text = PASSWORD_KV.replace_all(&text, "$1=[REDACTED]");
    text.into_owned()
}

/// AI 对话客户端（DESIGN.md §3.6）：OpenAI 兼容的 `/chat/completions` SSE 流式协议，
/// 豆包/DeepSeek/通义千问/本地 Ollama 都走同一条路径。
pub struct AiChatClient {
    client: reqwest::Client,
}

impl AiChatClient {
    pub fn new() -> Self {
        Self { client: reqwest::Client::new() }
    }

    /// 增量文本通过 `ai:chat-chunk` 事件推给前端，结束发 `ai:chat-done`，出错发
    /// `ai:chat-error`——和 `ssh:data`/`ssh:status` 同一套事件驱动模式
    /// （CODE_DESIGN.md §七），命令本身立即返回，不阻塞在整个对话流上。
    pub async fn stream_chat(
        &self,
        provider: &AiProvider,
        api_key: Option<&str>,
        messages: &[ChatMessage],
        redact_enabled: bool,
        web_search_enabled: bool,
        app_handle: AppHandle,
        request_id: Uuid,
    ) {
        let result = self
            .run_stream(
                provider,
                api_key,
                messages,
                redact_enabled,
                web_search_enabled,
                &app_handle,
                request_id,
            )
            .await;
        match result {
            Ok(()) => {
                let _ = app_handle.emit("ai:chat-done", serde_json::json!({ "requestId": request_id }));
            }
            Err(e) => {
                let _ = app_handle.emit(
                    "ai:chat-error",
                    serde_json::json!({ "requestId": request_id, "message": e.to_string() }),
                );
            }
        }
    }

    async fn run_stream(
        &self,
        provider: &AiProvider,
        api_key: Option<&str>,
        messages: &[ChatMessage],
        redact_enabled: bool,
        web_search_enabled: bool,
        app_handle: &AppHandle,
        request_id: Uuid,
    ) -> Result<(), AppError> {
        // 只对云端 Provider 做脱敏——本地 Ollama 不出网，脱敏反而会污染用户本想让
        // 模型原样看到的日志/代码内容（DESIGN.md §3.6"每个 Provider 需标注本地/云端"）。
        let mut outgoing: Vec<ChatMessage> = if redact_enabled && !provider.is_local {
            messages
                .iter()
                .map(|m| ChatMessage { role: m.role.clone(), content: redact(&m.content) })
                .collect()
        } else {
            messages.to_vec()
        };

        if web_search_enabled {
            let mut recent_user_messages = outgoing
                .iter()
                .rev()
                .filter(|message| message.role == "user")
                .take(3)
                .map(|message| message.content.as_str())
                .collect::<Vec<_>>();
            recent_user_messages.reverse();
            if !recent_user_messages.is_empty() {
                // Include recent user turns so follow-ups such as “它的财报呢” retain
                // the company/topic from the previous question.
                let query = recent_user_messages.join("\n");
                let query = query.chars().rev().take(800).collect::<String>().chars().rev().collect::<String>();
                let context = self.search_web(&redact(&query)).await?;
                outgoing.insert(0, ChatMessage {
                    role: "system".into(),
                    content: format!(
                        "以下是应用刚从互联网检索到的资料。你已经获得了真实的联网检索结果，不要声称自己没有搜索工具。回答时优先利用相关资料；资料不足时明确说明。引用事实时附上对应 URL。\n\n{context}"
                    ),
                });
            }
        }

        let url = format!("{}/chat/completions", provider.api_base.trim_end_matches('/'));
        let body = serde_json::json!({
            "model": provider.model,
            "messages": outgoing,
            "stream": true,
        });
        let mut req = self.client.post(&url).json(&body);
        if let Some(key) = api_key {
            req = req.bearer_auth(key);
        }

        let resp = req.send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Connection(format!("HTTP {status}: {body}")));
        }

        let mut stream = resp.bytes_stream();
        let mut buf = String::new();
        while let Some(chunk) = stream.next().await {
            let bytes = chunk?;
            buf.push_str(&String::from_utf8_lossy(&bytes));

            // SSE 帧以空行分隔（OpenAI 兼容协议：`data: {...}` / 结束帧 `data: [DONE]`）。
            while let Some(pos) = buf.find("\n\n") {
                let frame = buf[..pos].to_string();
                buf.drain(..pos + 2);
                for line in frame.lines() {
                    let Some(data) = line.strip_prefix("data:") else { continue };
                    let data = data.trim();
                    if data == "[DONE]" {
                        return Ok(());
                    }
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(data) {
                        if let Some(delta) = v["choices"][0]["delta"]["content"].as_str() {
                            let _ = app_handle.emit(
                                "ai:chat-chunk",
                                serde_json::json!({ "requestId": request_id, "delta": delta }),
                            );
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// 用 Bing RSS 获取轻量、无脚本的搜索结果。搜索失败时由调用方静默退回普通对话，
    /// 不让临时网络问题阻断 AI 问答。
    async fn search_web(&self, query: &str) -> Result<String, AppError> {
        search_web_results(&self.client, query).await
    }
}

/// 供统一 AI工具的 function-calling 使用的互联网搜索入口。这里与旧版问答面板
/// 共用 Bing RSS 抓取逻辑，避免 Coding Agent 退化成“模型自己声称无法联网”。
pub async fn search_web_results(client: &reqwest::Client, query: &str) -> Result<String, AppError> {
        let mut queries = vec![query.to_string()];
        // Bing RSS sometimes tokenizes this Chinese company name as only “建”.
        // Retry with its English name so the search toggle returns useful data.
        if query.contains("建滔") {
            queries.push(format!("{query} Kingboard Holdings"));
            queries.push("Kingboard Holdings annual report financial results".into());
        }
        let mut results = Vec::new();
        for candidate in queries {
            let url = reqwest::Url::parse_with_params(
                "https://www.bing.com/search",
                &[("q", candidate.as_str()), ("format", "rss"), ("setlang", "zh-CN")],
            )
            .map_err(|e| AppError::Connection(e.to_string()))?;
            let xml = client
                .get(url)
                .header(reqwest::header::USER_AGENT, "roc_desk/1.0 (AI web search)")
                .send()
                .await?
                .error_for_status()?
                .text()
                .await?;
            for item in SEARCH_ITEM.captures_iter(&xml).take(5) {
                let Some(item) = item.get(1).map(|m| m.as_str()) else { continue };
                let Some(title) = SEARCH_TITLE.captures(item).and_then(|c| c.get(1)).map(|m| xml_text(m.as_str())) else { continue };
                let Some(link) = SEARCH_LINK.captures(item).and_then(|c| c.get(1)).map(|m| xml_text(m.as_str())) else { continue };
                let description = SEARCH_DESCRIPTION.captures(item).and_then(|c| c.get(1)).map(|v| xml_text(v.as_str())).unwrap_or_default();
                if !results.iter().any(|line: &String| line.contains(&link)) {
                    results.push(format!("- {title}\n  {link}\n  {description}"));
                }
            }
            if results.len() >= 5 { break; }
        }
        let results = results.into_iter().take(8).collect::<Vec<_>>();
        if results.is_empty() {
            return Err(AppError::Connection("互联网搜索未返回结果".into()));
        }
        Ok(results.join("\n"))
}

fn xml_text(value: &str) -> String {
    XML_TAG
        .replace_all(value, "")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .trim()
        .to_string()
}
