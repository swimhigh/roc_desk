use async_trait::async_trait;
use serde_json::{json, Value};

use super::stdio::StdioTransport;
use super::http::HttpTransport;
use super::{McpServer, McpTransportKind};
use crate::error::AppError;

/// 一次 MCP JSON-RPC 2.0 往返（`initialize`/`tools/list`/`tools/call`），stdio 和
/// HTTP 两种传输各自实现。`notify` 对应 JSON-RPC 通知（没有 `id`，不等待响应），
/// 目前只用在握手后的 `notifications/initialized`。
#[async_trait]
pub trait McpTransport: Send + Sync {
    async fn call(&self, method: &str, params: Value) -> Result<Value, AppError>;
    async fn notify(&self, method: &str, params: Value) -> Result<(), AppError>;
}

#[derive(Debug, Clone)]
pub struct McpTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

/// 单个 MCP 服务器的已建立连接：握手完成、工具列表已缓存。`McpServerManager`
/// 负责懒创建并长期持有（见 `mcp/mod.rs`），这里只管协议本身。
pub struct McpClient {
    transport: Box<dyn McpTransport>,
    pub tools: Vec<McpTool>,
}

impl McpClient {
    pub async fn connect(server: &McpServer, auth_token: Option<&str>) -> Result<Self, AppError> {
        let transport: Box<dyn McpTransport> = match server.transport {
            McpTransportKind::Stdio => Box::new(StdioTransport::spawn(server).await?),
            McpTransportKind::Http => Box::new(HttpTransport::new(server, auth_token)),
        };

        let init_params = json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "roc_desk", "version": env!("CARGO_PKG_VERSION") },
        });
        transport.call("initialize", init_params).await?;
        // MCP 握手要求客户端在收到 initialize 响应后发一条 `notifications/initialized`
        // 通知；个别实现对这条通知处理得比较随意（不回任何东西也不算错），失败了
        // 不阻断后续 tools/list——真正要紧的是双方已经握过手。
        let _ = transport.notify("notifications/initialized", json!({})).await;

        let tools = Self::fetch_tools(transport.as_ref()).await?;
        Ok(Self { transport, tools })
    }

    async fn fetch_tools(transport: &dyn McpTransport) -> Result<Vec<McpTool>, AppError> {
        // 不处理分页（`nextCursor`）——个人开发者接入的 MCP 服务器工具数量通常在
        // 个位数到几十个，真的需要分页的场景留到有真实需求时再补。
        let result = transport.call("tools/list", json!({})).await?;
        let raw_tools = result["tools"].as_array().cloned().unwrap_or_default();
        Ok(raw_tools
            .into_iter()
            .filter_map(|t| {
                Some(McpTool {
                    name: t["name"].as_str()?.to_string(),
                    description: t["description"].as_str().unwrap_or_default().to_string(),
                    input_schema: t
                        .get("inputSchema")
                        .cloned()
                        .unwrap_or_else(|| json!({ "type": "object", "properties": {} })),
                })
            })
            .collect())
    }

    pub async fn call_tool(&self, name: &str, arguments: Value) -> Result<String, AppError> {
        let result = self.transport.call("tools/call", json!({ "name": name, "arguments": arguments })).await?;
        let text = mcp_content_to_text(&result);
        if result["isError"].as_bool().unwrap_or(false) {
            return Err(AppError::Internal(if text.is_empty() { "MCP 工具执行失败".to_string() } else { text }));
        }
        Ok(text)
    }
}

/// MCP 工具结果统一是 `{ content: [{type: "text", text: "..."}, ...], isError? }`
/// 这种结构化形式（也可能有 image/resource 类型的 content，暂不处理，模型工具
/// 结果本来就只能是文本）——这里只抽取文本部分拼起来喂给模型。
fn mcp_content_to_text(result: &Value) -> String {
    let Some(items) = result["content"].as_array() else { return result.to_string() };
    items
        .iter()
        .filter_map(|item| item["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}
