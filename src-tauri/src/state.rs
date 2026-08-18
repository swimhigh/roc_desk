use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};

use tokio::sync::{Mutex, RwLock};
use uuid::Uuid;

use crate::ai::{AiChatClient, AiProviderManager};
use crate::coding::{CodingSession, CommandConfirmRegistry};
use crate::connection::ConnectionManager;
use crate::credential::CredentialStore;
use crate::db::repo::audit_log_repo::AuditLogRepo;
use crate::db::repo::browser_history_repo::BrowserHistoryRepo;
use crate::db::DbPool;
use crate::log::{LogImporter, LogSearchEngine};
use crate::pty::LocalPtyManager;
use crate::ssh::{SshConnectionPool, TrustPromptRegistry};
use crate::workspace::{WorkspaceHandle, WorkspaceManager};

/// 应用状态聚合（CODE_DESIGN.md §3.1）。
///
/// Coding Session、MCP、Lua 等字段会在对应模块实现时加入；目前覆盖 Phase 1/2/3
/// 落地的部分：工作区/文件操作 + 连接管理 + SSH + 日志搜索 + AI 问答。
pub struct AppState {
    pub db: DbPool,
    pub credential_store: Arc<dyn CredentialStore>,
    pub connection_manager: Arc<ConnectionManager>,
    pub ssh_pool: Arc<SshConnectionPool>,
    pub trust_prompts: TrustPromptRegistry,
    pub workspace_manager: Arc<WorkspaceManager>,
    /// 当前窗口内已打开的工作区句柄，key 为 WorkspaceProfile.id
    pub workspaces: Arc<RwLock<HashMap<Uuid, WorkspaceHandle>>>,
    pub log_engine: Arc<LogSearchEngine>,
    pub log_importer: Arc<LogImporter>,
    pub ai_provider_manager: Arc<AiProviderManager>,
    pub ai_chat_client: Arc<AiChatClient>,
    /// AI 编程助手会话，key 为 workspace id——每个工作区自动绑定最多一个活跃会话
    /// （DESIGN.md §3.8.1"自动绑定当前工作区"），不需要额外的 session_id 概念。
    pub coding_sessions: Arc<RwLock<HashMap<Uuid, Arc<Mutex<CodingSession>>>>>,
    pub command_confirms: CommandConfirmRegistry,
    pub audit_log: Arc<AuditLogRepo>,
    pub local_pty: Arc<LocalPtyManager>,
    pub browser_history: Arc<BrowserHistoryRepo>,
    /// 当前正在跑的 Explorer 全文搜索请求 id（`fs_search_stream`）——新搜索开始时
    /// 覆盖这个值，正在跑的旧搜索每处理一个目录/文件都会发现自己的 request_id 已经
    /// 不是"当前"这个了，尽快中止，不会几次搜索并发抢 CPU/IO（见 fsops::search_stream
    /// 的 `should_cancel` 钩子）。用 `std::sync::Mutex` 而不是 `tokio::sync::Mutex`——
    /// 只是拿锁读写一个值，不跨 `.await`，标准库的锁足够，不需要 tokio 版本的开销。
    pub active_search: Arc<StdMutex<Option<Uuid>>>,
}
