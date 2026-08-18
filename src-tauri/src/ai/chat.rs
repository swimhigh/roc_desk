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
        app_handle: AppHandle,
        request_id: Uuid,
    ) {
        let result = self
            .run_stream(provider, api_key, messages, redact_enabled, &app_handle, request_id)
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
        app_handle: &AppHandle,
        request_id: Uuid,
    ) -> Result<(), AppError> {
        // 只对云端 Provider 做脱敏——本地 Ollama 不出网，脱敏反而会污染用户本想让
        // 模型原样看到的日志/代码内容（DESIGN.md §3.6"每个 Provider 需标注本地/云端"）。
        let outgoing: Vec<ChatMessage> = if redact_enabled && !provider.is_local {
            messages
                .iter()
                .map(|m| ChatMessage { role: m.role.clone(), content: redact(&m.content) })
                .collect()
        } else {
            messages.to_vec()
        };

        let url = format!("{}/chat/completions", provider.api_base.trim_end_matches('/'));
        let mut req = self.client.post(&url).json(&serde_json::json!({
            "model": provider.model,
            "messages": outgoing,
            "stream": true,
        }));
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
}
