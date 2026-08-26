pub mod client;
pub mod http;
pub mod stdio;

use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::credential::CredentialStore;
use crate::db::repo::mcp_servers_repo::McpServersRepo;
use crate::error::AppError;

pub use client::{McpClient, McpTool};

/// MCP 服务器配置（`0012_mcp_servers.sql`），对齐 OpenCode 的 `mcp` 配置块，但只
/// 支持两种传输：本地 stdio 子进程、远程 HTTP（Streamable HTTP 的单次 JSON 响应
/// 模式，不支持 SSE 长连接——见 `mcp/http.rs` 顶部注释里的范围裁剪说明）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpTransportKind {
    Stdio,
    Http,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServer {
    pub id: Uuid,
    pub name: String,
    pub transport: McpTransportKind,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    pub url: Option<String>,
    pub headers: HashMap<String, String>,
    pub auth_token_ref: Option<String>,
    pub enabled: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct McpServerInput {
    pub name: String,
    pub transport: McpTransportKind,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    pub url: Option<String>,
    pub headers: HashMap<String, String>,
    /// 前端只在用户真的填了新 token 时才传值；留空表示"沿用已保存的那份"——
    /// 和 `AiProviderManager` 的 `api_key` 语义完全一致。
    pub auth_token: Option<String>,
    pub enabled: bool,
}

fn credential_key(id: Uuid) -> String {
    format!("mcp:{id}:auth_token")
}

/// MCP 服务器的增删改查 + 懒连接缓存（DESIGN.md/REQUIREMENTS.md §3.7 补上的
/// "未实现：MCP 客户端"）。长期持有在 `AppState` 里，和 `SshConnectionPool` 是
/// 同一种"跨工作区/跨会话共用连接"的模式——stdio 子进程一旦起来没必要每个
/// 会话各起一份，HTTP 连接本身也无状态、天然可共用。
pub struct McpServerManager {
    repo: Arc<McpServersRepo>,
    credential_store: Arc<dyn CredentialStore>,
    clients: RwLock<HashMap<Uuid, Arc<McpClient>>>,
}

impl McpServerManager {
    pub fn new(repo: Arc<McpServersRepo>, credential_store: Arc<dyn CredentialStore>) -> Self {
        Self { repo, credential_store, clients: RwLock::new(HashMap::new()) }
    }

    pub async fn create(&self, input: McpServerInput) -> Result<McpServer, AppError> {
        let id = Uuid::new_v4();
        let auth_token_ref = match &input.auth_token {
            Some(token) if !token.is_empty() => {
                let key = credential_key(id);
                self.credential_store.set(&key, token).await?;
                Some(key)
            }
            _ => None,
        };
        let server = McpServer {
            id,
            name: input.name,
            transport: input.transport,
            command: input.command,
            args: input.args,
            env: input.env,
            url: input.url,
            headers: input.headers,
            auth_token_ref,
            enabled: input.enabled,
            created_at: Utc::now().to_rfc3339(),
        };
        self.repo.create(&server)?;
        Ok(server)
    }

    pub async fn update(&self, id: Uuid, input: McpServerInput) -> Result<McpServer, AppError> {
        let existing = self.repo.get(id)?.ok_or_else(|| AppError::NotFound(format!("mcp server not found: {id}")))?;
        let auth_token_ref = match &input.auth_token {
            Some(token) if !token.is_empty() => {
                let key = existing.auth_token_ref.clone().unwrap_or_else(|| credential_key(id));
                self.credential_store.set(&key, token).await?;
                Some(key)
            }
            _ => existing.auth_token_ref,
        };
        let server = McpServer {
            id,
            name: input.name,
            transport: input.transport,
            command: input.command,
            args: input.args,
            env: input.env,
            url: input.url,
            headers: input.headers,
            auth_token_ref,
            enabled: input.enabled,
            created_at: existing.created_at,
        };
        self.repo.update(&server)?;
        self.clients.write().await.remove(&id);
        Ok(server)
    }

    pub async fn delete(&self, id: Uuid) -> Result<(), AppError> {
        if let Some(existing) = self.repo.get(id)? {
            if let Some(key) = existing.auth_token_ref {
                self.credential_store.delete(&key).await?;
            }
        }
        self.repo.delete(id)?;
        self.clients.write().await.remove(&id);
        Ok(())
    }

    pub fn list(&self) -> Result<Vec<McpServer>, AppError> {
        self.repo.list()
    }

    /// 已启用的服务器列表（工具循环拼装 `tools` 数组时用，未启用的服务器不出现
    /// 在模型可调用的工具里，也就不会被懒连接）。
    pub fn list_enabled(&self) -> Result<Vec<McpServer>, AppError> {
        Ok(self.list()?.into_iter().filter(|s| s.enabled).collect())
    }

    /// 懒连接：第一次用到某个服务器时才真正起子进程/建立 HTTP 配置并跑
    /// `initialize` 握手，后续复用同一个 `McpClient`（含它内部缓存的 `tools/list`
    /// 结果）。stdio 子进程的生命周期跟着这个 `Arc<McpClient>`——`mcp/stdio.rs`
    /// spawn 时显式设置了 `kill_on_drop(true)`（tokio 默认是 false，不显式设置
    /// 子进程会在应用退出后变成孤儿进程），`Arc` 引用计数归零时子进程一起被杀掉。
    pub async fn get_or_connect(&self, server_id: Uuid) -> Result<Arc<McpClient>, AppError> {
        if let Some(client) = self.clients.read().await.get(&server_id).cloned() {
            return Ok(client);
        }
        let server = self.repo.get(server_id)?.ok_or_else(|| AppError::NotFound(format!("mcp server not found: {server_id}")))?;
        let auth_token = match &server.auth_token_ref {
            Some(key) => self.credential_store.get(key).await?,
            None => None,
        };
        let client = Arc::new(McpClient::connect(&server, auth_token.as_deref()).await?);
        self.clients.write().await.insert(server_id, client.clone());
        Ok(client)
    }
}
