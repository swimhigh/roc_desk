use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{oneshot, Mutex};

use super::client::McpTransport;
use super::McpServer;
use crate::error::AppError;

/// 一次 MCP 调用最多等这么久——MCP 服务器是机器对机器的进程/连接，不像命令
/// 确认弹窗那样合理地无限期等真人操作，卡住了应该报错而不是把整个工具循环
/// 一起拖死。
const CALL_TIMEOUT: Duration = Duration::from_secs(30);

/// stdio 传输：起一个子进程，MCP 消息是换行分隔的 JSON-RPC 2.0（`Content-Length`
/// 帧头是给 LSP 用的，MCP stdio 规范用的是更简单的 NDJSON）。写走一把锁保护的
/// `ChildStdin`（同一时刻只有一次写入在跑，避免多个并发工具调用把请求行拼到一起）；
/// 读是一个独立后台 task，按响应里的 `id` 分发给等待中的调用者——和
/// `CommandConfirmRegistry` 的 pending-map 是同一个模式，只是这里的 key 是数字
/// 请求 id 不是 `Uuid`。
pub struct StdioTransport {
    stdin: Mutex<ChildStdin>,
    pending: Arc<Mutex<HashMap<i64, oneshot::Sender<Value>>>>,
    next_id: AtomicI64,
    /// 只是为了让子进程跟这个结构体的生命周期绑在一起（`kill_on_drop(true)`
    /// 已经在 spawn 时设置，drop 这个字段就会杀掉子进程），从不主动读写它。
    _child: Child,
}

impl StdioTransport {
    pub async fn spawn(server: &McpServer) -> Result<Self, AppError> {
        let command = server
            .command
            .clone()
            .ok_or_else(|| AppError::Internal(format!("MCP 服务器 {} 未配置可执行命令", server.name)))?;

        let mut cmd = Command::new(&command);
        cmd.args(&server.args)
            .envs(&server.env)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);

        let mut child = cmd
            .spawn()
            .map_err(|e| AppError::Internal(format!("启动 MCP 服务器 {} 失败：{e}", server.name)))?;

        let stdin = child.stdin.take().ok_or_else(|| AppError::Internal("MCP 子进程没有 stdin".into()))?;
        let stdout = child.stdout.take().ok_or_else(|| AppError::Internal("MCP 子进程没有 stdout".into()))?;

        let pending: Arc<Mutex<HashMap<i64, oneshot::Sender<Value>>>> = Arc::new(Mutex::new(HashMap::new()));
        let reader_pending = pending.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            loop {
                match lines.next_line().await {
                    Ok(Some(line)) => {
                        let line = line.trim();
                        if line.is_empty() {
                            continue;
                        }
                        let Ok(value) = serde_json::from_str::<Value>(line) else { continue };
                        // 只处理带 `id` 的响应（请求的回执）；服务端主动发来的通知
                        // （没有 `id`，比如进度提示）目前不消费，直接丢弃。
                        let Some(id) = value.get("id").and_then(|v| v.as_i64()) else { continue };
                        if let Some(tx) = reader_pending.lock().await.remove(&id) {
                            let payload = if let Some(err) = value.get("error") {
                                json!({ "__mcp_error__": true, "detail": err })
                            } else {
                                value.get("result").cloned().unwrap_or(Value::Null)
                            };
                            let _ = tx.send(payload);
                        }
                    }
                    _ => break, // EOF 或读取失败：子进程大概率已经退出，停止读取循环
                }
            }
        });

        Ok(Self { stdin: Mutex::new(stdin), pending, next_id: AtomicI64::new(1), _child: child })
    }

    async fn write_line(&self, value: &Value) -> Result<(), AppError> {
        let mut line = serde_json::to_string(value).map_err(|e| AppError::Internal(e.to_string()))?;
        line.push('\n');
        let mut stdin = self.stdin.lock().await;
        stdin.write_all(line.as_bytes()).await.map_err(AppError::from)?;
        stdin.flush().await.map_err(AppError::from)
    }
}

#[async_trait]
impl McpTransport for StdioTransport {
    async fn call(&self, method: &str, params: Value) -> Result<Value, AppError> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);

        let request = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        if let Err(e) = self.write_line(&request).await {
            self.pending.lock().await.remove(&id);
            return Err(e);
        }

        let result = tokio::time::timeout(CALL_TIMEOUT, rx)
            .await
            .map_err(|_| AppError::Connection(format!("MCP 调用 {method} 超时")))?
            .map_err(|_| AppError::Internal("MCP 子进程提前退出，未收到响应".into()))?;

        if result.get("__mcp_error__").is_some() {
            return Err(AppError::Internal(format!("MCP 调用 {method} 失败：{}", result["detail"])));
        }
        Ok(result)
    }

    async fn notify(&self, method: &str, params: Value) -> Result<(), AppError> {
        self.write_line(&json!({ "jsonrpc": "2.0", "method": method, "params": params })).await
    }
}
