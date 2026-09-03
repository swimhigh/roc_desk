use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex as StdMutex};

use tokio::sync::{Mutex, RwLock};
use uuid::Uuid;

use crate::agent::{AgentConnectionPool, AgentTrustPromptRegistry};
use crate::ai::{AiChatClient, AiProviderManager};
use crate::coding::{CodingSession, CommandConfirmRegistry, QuestionRegistry};
use crate::connection::{ConnectionGroupManager, ConnectionManager};
use crate::credential::CredentialStore;
use crate::db::repo::audit_log_repo::AuditLogRepo;
use crate::db::repo::coding_history_repo::CodingHistoryRepo;
use crate::db::repo::browser_history_repo::BrowserHistoryRepo;
use crate::db::repo::permission_rules_repo::PermissionRulesRepo;
use crate::db::DbPool;
use crate::log::{LogImporter, LogSearchEngine};
use crate::mcp::McpServerManager;
use crate::pty::LocalPtyManager;
use crate::rdp::RdpSessionManager;
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
    pub connection_group_manager: Arc<ConnectionGroupManager>,
    pub ssh_pool: Arc<SshConnectionPool>,
    pub rdp_sessions: Arc<RdpSessionManager>,
    pub trust_prompts: TrustPromptRegistry,
    /// 远程 Windows Agent 连接池（AGENT_DESIGN.md），和 `ssh_pool` 是同一种"连接池"
    /// 模式——`WorkspaceManager`/`CodingSession` 按连接档案的 `protocol` 字段决定
    /// 用这个还是 `ssh_pool`。
    pub agent_pool: Arc<AgentConnectionPool>,
    /// Agent TLS 证书指纹 TOFU 弹窗的等待注册表，和 `trust_prompts`（SSH 主机指纹）
    /// 是两条独立的信任链条，故意不合用一张表（见 `agent::handshake` 模块文档）。
    pub agent_trust_prompts: AgentTrustPromptRegistry,
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
    pub coding_history: Arc<CodingHistoryRepo>,
    pub local_pty: Arc<LocalPtyManager>,
    pub browser_history: Arc<BrowserHistoryRepo>,
    /// 当前正在跑的 Explorer 全文搜索请求 id（`fs_search_stream`）——新搜索开始时
    /// 覆盖这个值，正在跑的旧搜索每处理一个目录/文件都会发现自己的 request_id 已经
    /// 不是"当前"这个了，尽快中止，不会几次搜索并发抢 CPU/IO（见 fsops::search_stream
    /// 的 `should_cancel` 钩子）。用 `std::sync::Mutex` 而不是 `tokio::sync::Mutex`——
    /// 只是拿锁读写一个值，不跨 `.await`，标准库的锁足够，不需要 tokio 版本的开销。
    pub active_search: Arc<StdMutex<Option<Uuid>>>,
    /// SFTP/Agent 双栏浏览器"停止传输"用（2026-09-01 用户反馈：拖拽/上传下载
    /// 大文件夹时没有办法主动停下来，只能干等传完）——和 `active_search` 是同一个
    /// 取消模式，区别是这里可能同时有多个传输各自跑在不同的 `request_id` 下（比如
    /// 两个不同的浏览器面板各自拖了一次），所以用 `HashSet` 记"被取消的" id 集合，
    /// 不是单个"当前活跃的" id：递归复制（`fsops::copy_between`/
    /// `fsops::remote::upload_recursive`/`download_recursive`）每处理完一个文件都
    /// 检查一次自己的 `request_id` 是否在这个集合里，在就尽快中止。取消/正常结束/
    /// 出错都要记得从集合里移除对应 id，不然会无限增长。
    pub cancelled_transfers: Arc<StdMutex<HashSet<Uuid>>>,
    /// 传输日志（用户 2026-09-01 需求："传输日志需要记录，并可在界面上查询追溯"）。
    pub transfer_log: Arc<crate::db::repo::transfer_log_repo::TransferLogRepo>,
    /// 权限规则引擎的持久化层（REQUIREMENTS.md §3.7 权限引擎升级）——`CodingSession`
    /// 不持有它，`send_message` 每次都现取一份最新规则，见 `coding/permission.rs`。
    pub permission_rules: Arc<PermissionRulesRepo>,
    /// `question` 工具的等待注册表，和 `command_confirms` 是同一套 oneshot 模式。
    pub question_confirms: QuestionRegistry,
    /// MCP 服务器配置 + 懒连接缓存，长期持有、跨工作区/会话共用（和 `ssh_pool`
    /// 是同一种"连接池"模式）。
    pub mcp_manager: Arc<McpServerManager>,
    /// 冷启动时命令行参数里带的文件路径（Windows"打开方式"/双击已关联文件，
    /// 2026-09-03 需求）——此时前端 JS 还没跑起来，没法直接 emit 事件给它，
    /// 先存这里，前端 App.tsx 挂载后调 `take_pending_open_paths` 取走并清空。
    /// 已运行实例收到第二次启动的 argv 转发（见 `lib.rs` 的
    /// `tauri_plugin_single_instance::init`）不走这个字段，直接 emit 事件，
    /// 因为那种情况前端肯定已经跑起来了。
    pub pending_open_paths: StdMutex<Vec<String>>,
}
