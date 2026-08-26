use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use futures_util::StreamExt;
use serde_json::{json, Value};
use tokio::sync::Mutex;

use super::client::McpTransport;
use super::McpServer;
use crate::error::AppError;

const CALL_TIMEOUT: Duration = Duration::from_secs(30);

/// HTTP 传输：MCP"Streamable HTTP"规范。**2026-08-26 用一台真实的内网 MCP 服务器
/// （FastMCP/uvicorn 技术栈）联调后发现最初"只支持 application/json 单次响应，
/// SSE 一律报错"的范围裁剪是错的——这类服务器即使只返回一条消息，也总是用
/// `Content-Type: text/event-stream` 包一层（`event: message\ndata: {...}\n\n`），
/// 不是"流式才用 SSE"，而是"默认就用 SSE 框架，哪怕只有一帧"。真要跳过 SSE 解析，
/// 大概率会连不上大多数真实部署的 MCP HTTP 服务器，所以这里补上了按 `\n\n` 拆帧、
/// 取 `data:` 字段解析 JSON-RPC 消息的最小 SSE 解析（不是完整 EventSource 实现，
/// 不处理 `id:`/`retry:`/多行 `data:` 拼接等 SSE 边角特性，够解出 MCP 用到的这一种
/// 帧格式）。收到第一条 `id` 匹配本次请求的消息就返回，不需要等连接关闭——服务器
/// 通常在发完结果后很快主动关闭这条流。
///
/// 规范里握手响应可能带 `Mcp-Session-Id` 响应头，之后的请求要原样带回去，
/// 否则一些服务器实现会直接拒绝——这里做了最小支持：记住第一次拿到的
/// session id，后续请求都带上。
pub struct HttpTransport {
    client: reqwest::Client,
    url: String,
    headers: HashMap<String, String>,
    auth_token: Option<String>,
    session_id: Mutex<Option<String>>,
    next_id: AtomicI64,
}

impl HttpTransport {
    pub fn new(server: &McpServer, auth_token: Option<&str>) -> Self {
        Self {
            client: reqwest::Client::new(),
            url: server.url.clone().unwrap_or_default(),
            headers: server.headers.clone(),
            auth_token: auth_token.map(str::to_string),
            session_id: Mutex::new(None),
            next_id: AtomicI64::new(1),
        }
    }

    async fn post(&self, body: Value, expect_response: bool, expected_id: Option<i64>) -> Result<Option<Value>, AppError> {
        let mut req = self
            .client
            .post(&self.url)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header(reqwest::header::ACCEPT, "application/json, text/event-stream")
            .json(&body);
        for (key, value) in &self.headers {
            req = req.header(key.as_str(), value.as_str());
        }
        if let Some(token) = &self.auth_token {
            req = req.bearer_auth(token);
        }
        if let Some(session) = self.session_id.lock().await.clone() {
            req = req.header("Mcp-Session-Id", session);
        }

        let resp = tokio::time::timeout(CALL_TIMEOUT, req.send())
            .await
            .map_err(|_| AppError::Connection("MCP HTTP 请求超时".into()))??;

        if let Some(session) = resp.headers().get("mcp-session-id").and_then(|v| v.to_str().ok()) {
            *self.session_id.lock().await = Some(session.to_string());
        }

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(AppError::Connection(format!("MCP HTTP {status}: {text}")));
        }
        if !expect_response {
            return Ok(None);
        }

        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();
        if content_type.starts_with("text/event-stream") {
            return tokio::time::timeout(CALL_TIMEOUT, Self::read_sse_response(resp, expected_id))
                .await
                .map_err(|_| AppError::Connection("MCP SSE 响应超时".into()))?;
        }

        let value: Value = resp.json().await.map_err(AppError::from)?;
        Ok(Some(value))
    }

    /// 按空行拆帧、取每帧里的 `data:` 字段解析成 JSON——只认带 `id` 且和本次请求
    /// `id` 匹配的帧当作最终结果；没有 `id` 的帧（服务端主动推送的日志/进度通知）
    /// 忽略，继续等下一帧。读到匹配帧就提前返回、丢弃 `resp`（连接自然被
    /// reqwest/hyper 关闭），不强求等服务器主动结束这条流。
    async fn read_sse_response(resp: reqwest::Response, expected_id: Option<i64>) -> Result<Option<Value>, AppError> {
        let mut stream = resp.bytes_stream();
        let mut buf = String::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(AppError::from)?;
            buf.push_str(&String::from_utf8_lossy(&chunk));
            // 实测这台服务器（uvicorn/FastMCP）用的是 `\r\n\r\n` 分隔帧，不是裸
            // `\n\n`——先统一换行符再找空行，否则 `\r\n\r\n` 里两个 `\n` 中间隔着
            // 一个 `\r`，永远匹配不上，SSE 响应会一直等到超时。
            buf = buf.replace("\r\n", "\n");
            while let Some(pos) = buf.find("\n\n") {
                let frame = buf[..pos].to_string();
                buf.drain(..pos + 2);
                for line in frame.lines() {
                    let Some(data) = line.strip_prefix("data:") else { continue };
                    let Ok(value) = serde_json::from_str::<Value>(data.trim()) else { continue };
                    let frame_id = value.get("id").and_then(|v| v.as_i64());
                    if frame_id.is_some() && (expected_id.is_none() || frame_id == expected_id) {
                        return Ok(Some(value));
                    }
                }
            }
        }
        Err(AppError::Connection("MCP SSE 响应流结束但未收到匹配的响应帧".into()))
    }
}

#[async_trait]
impl McpTransport for HttpTransport {
    async fn call(&self, method: &str, params: Value) -> Result<Value, AppError> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let body = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let value = self
            .post(body, true, Some(id))
            .await?
            .ok_or_else(|| AppError::Internal("MCP 服务器未返回结果".into()))?;
        if let Some(err) = value.get("error") {
            return Err(AppError::Internal(format!("MCP 调用 {method} 失败：{err}")));
        }
        Ok(value.get("result").cloned().unwrap_or(Value::Null))
    }

    async fn notify(&self, method: &str, params: Value) -> Result<(), AppError> {
        let body = json!({ "jsonrpc": "2.0", "method": method, "params": params });
        self.post(body, false, None).await?;
        Ok(())
    }
}
