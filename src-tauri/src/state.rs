use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};

use tokio::sync::{Mutex, RwLock};
use uuid::Uuid;

use crate::ai::{AiChatClient, AiProviderManager, AiRuntime};
use crate::coding::{ChangeStore, CodingSession, CommandConfirmRegistry, PendingInjection, QuestionRegistry};
use crate::credential::CredentialStore;
use crate::db::repo::audit_log_repo::AuditLogRepo;
use crate::db::repo::browser_history_repo::BrowserHistoryRepo;
use crate::db::repo::coding_history_repo::CodingHistoryRepo;
use crate::db::repo::ai_evidence_repo::AiEvidenceRepo;
use crate::db::repo::permission_rules_repo::PermissionRulesRepo;
use crate::db::repo::sql_agent_history_repo::SqlAgentHistoryRepo;
use crate::db::DbPool;
use crate::log::{LogImporter, LogSearchEngine};
use crate::mcp::McpServerManager;
use crate::sql::agent::SqlAgentSession;
use crate::sql::ai_assistant::SqlAiAssistant;
use crate::symbols::SymbolIndex;
use crate::workspace::{WorkspaceHandle, WorkspaceManager};

/// 应用状态聚合（CODE_DESIGN.md §3.1）。
///
/// Coding Session、MCP、Lua 等字段会在对应模块实现时加入；目前覆盖 Phase 1/2/3
/// 落地的部分：工作区/文件操作 + 连接管理 + SSH + 日志搜索 + AI 问答。
pub struct AppState {
    pub db: DbPool,
    pub credential_store: Arc<dyn CredentialStore>,
    // 连接管理/SSH/SFTP/Agent/RDP/传输日志已经迁到 roc_desk_ssh
    // （roc_desk_ssh::RocDeskSshAppState，单独 manage，见 lib.rs::run）——迁移前
    // 这里是 `connection_manager`/`connection_group_manager`/`ssh_pool`/
    // `rdp_sessions`/`trust_prompts`/`agent_pool`/`agent_trust_prompts`/
    // `cancelled_transfers`/`transfer_log` 九个字段。`commands/coding.rs`/
    // `commands/sql.rs`/`commands/log_search.rs` 里仍然需要 `ssh_pool`/
    // `agent_pool`/`connection_manager` 的命令函数改成额外取一份
    // `State<'_, roc_desk_ssh::RocDeskSshAppState>`（和 SQL 迁移时的处理方式
    // 一致，见 commands/sql.rs 顶部注释）。`workspace_manager`（下面这个字段）
    // 内部自己持有一份 `ssh_pool`/`agent_pool`/`connection_manager`，构造时从
    // `RocDeskSshAppState` 里克隆，不需要 `AppState` 自己再重复放一份。
    pub workspace_manager: Arc<WorkspaceManager>,
    /// 当前窗口内已打开的工作区句柄，key 为 WorkspaceProfile.id
    pub workspaces: Arc<RwLock<HashMap<Uuid, WorkspaceHandle>>>,
    /// 工作模块首页卡片"添加过哪些工作区"关联表（2026-09 需求，见
    /// `db::repo::workspace_module_links_repo` 文档）。
    pub workspace_module_links: Arc<crate::db::repo::workspace_module_links_repo::WorkspaceModuleLinksRepo>,
    pub log_engine: Arc<LogSearchEngine>,
    pub log_importer: Arc<LogImporter>,
    pub ai_provider_manager: Arc<AiProviderManager>,
    pub ai_runtime: Arc<AiRuntime>,
    pub ai_chat_client: Arc<AiChatClient>,
    /// AI 编程助手会话，key 为 workspace id——每个工作区自动绑定最多一个活跃会话
    /// （DESIGN.md §3.8.1"自动绑定当前工作区"），不需要额外的 session_id 概念。
    pub coding_sessions: Arc<RwLock<HashMap<Uuid, Arc<Mutex<CodingSession>>>>>,
    /// 文件改动（Diff/Accept/Undo/Redo）状态，和 `coding_sessions` 平级、同样以
    /// workspace_id 为 key，但故意拆成独立的锁——不能让"应用/拒绝某个文件改动"
    /// 卡在等一个可能跑一两分钟的 AI 对话轮次释放 `coding_sessions` 里那把锁
    /// （2026-09 用户真实反馈，详见 `coding::ChangeStore` 的文档注释）。
    pub coding_changes: Arc<RwLock<HashMap<Uuid, Arc<Mutex<ChangeStore>>>>>,
    pub command_confirms: CommandConfirmRegistry,
    pub audit_log: Arc<AuditLogRepo>,
    pub coding_history: Arc<CodingHistoryRepo>,
    pub ai_evidence: Arc<AiEvidenceRepo>,
    pub browser_history: Arc<BrowserHistoryRepo>,
    /// 当前正在跑的 Explorer 全文搜索请求 id（`fs_search_stream`）——新搜索开始时
    /// 覆盖这个值，正在跑的旧搜索每处理一个目录/文件都会发现自己的 request_id 已经
    /// 不是"当前"这个了，尽快中止，不会几次搜索并发抢 CPU/IO（见 fsops::search_stream
    /// 的 `should_cancel` 钩子）。用 `std::sync::Mutex` 而不是 `tokio::sync::Mutex`——
    /// 只是拿锁读写一个值，不跨 `.await`，标准库的锁足够，不需要 tokio 版本的开销。
    pub active_search: Arc<StdMutex<Option<Uuid>>>,
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
    /// 冷启动命令行参数解析出的模块角色（`--mode`/`--open`，见
    /// `commands::launcher::get_launch_context`）——不像 `pending_open_paths`
    /// 那样是一次性事件，这两个值在整个进程生命周期内固定不变，前端可以随时查询。
    pub launch_mode: Option<String>,
    pub launch_open: Option<String>,
    /// AI 编程助手"停止"按钮（2026-09 用户反馈：中转过载时一轮对话能卡一两
    /// 分钟，之前完全没有办法主动打断）用的取消信号，key 为 workspace id——
    /// 和 `active_search`/`cancelled_transfers` 是同一种"不跟长任务抢同一把锁"
    /// 的模式：`coding_send_message` 持有 `coding_sessions` 里那把锁一直到整
    /// 轮对话结束，`coding_cancel_turn` 如果也去抢那把锁会被同样卡住，所以
    /// 取消信号必须存在一个独立的地方。用 `tokio_util::sync::CancellationToken`
    /// 而不是像那两个一样用纯轮询的 `HashSet`——AI 对话轮次的瓶颈是单次可能
    /// 卡很久的网络 await（HTTP 请求/`codex_engine::run_turn`），不是天然逐条
    /// 处理的循环，`CancellationToken::cancelled()` 能配合 `tokio::select!`
    /// 精确打断这次 await，不需要轮询。
    pub coding_cancel_tokens:
        Arc<StdMutex<HashMap<Uuid, tokio_util::sync::CancellationToken>>>,
    /// AI 处理期间用户又发了一条新消息——不等当前这一轮工具循环彻底跑完，key 为
    /// workspace id，和 `coding_cancel_tokens` 同一种"不跟长任务抢同一把锁"模式：
    /// `coding_send_message` 从进入到返回一直独占持有 `coding_sessions` 那把锁，
    /// 插话命令如果也去抢那把锁只能等整轮结束才能把消息塞进去，跟直接不让插话
    /// 没区别。`CodingSession::send_message` 内部工具循环每轮迭代开头（和检查
    /// `cancel_token` 同一个位置）会读这张表、把攒到的消息追加进对话上下文，供
    /// 下一次模型请求看到——不是打断当前这次请求，是"下一轮工具调用前生效"
    /// （2026-09 用户反馈：AI 处理期间按 Enter，输入框内容被清空但消息没真正
    /// 发出去）。
    pub coding_pending_injections: Arc<StdMutex<HashMap<Uuid, Vec<PendingInjection>>>>,
    /// "转到定义/声明"用的符号索引（2026-09-16 需求），key 为 workspace id——和
    /// `coding_sessions`/`coding_changes` 同一种"按工作区一份"的模式。轻量正则
    /// 扫描器，不是真正的语言语义分析，见 `symbols` 模块文档。
    pub symbol_indexes: Arc<RwLock<HashMap<Uuid, SymbolIndex>>>,

    // 数据源 CRUD/会话管理/查询执行/工作区标签页/导出导入这些字段已迁到
    // roc_desk_sql（roc_desk_sql::SqlAppState，单独 manage，见 lib.rs::run）——
    // 迁移前这里是 `sql_data_source_service`/`sql_session_manager`/
    // `sql_query_history`/`sql_workspace_tabs`/`sql_workspace_cache`/
    // `sql_executor`/`sql_transfer_manager` 七个字段。AI 面板（下面
    // `sql_ai_assistant`/`sql_changes`）和 SQL Agent 依赖 `roc_desk_core`
    // 还没有的 `crate::ai`/`crate::agent_llm`，所以没有一起迁，留在宿主的
    // 命令函数改成额外取一份 `State<'_, roc_desk_sql::SqlAppState>`
    // （见 commands/sql.rs、commands/sql_agent.rs）。
    /// AI 对 SQL 标签页的文件改动状态，和 `coding_changes` 同样的
    /// "按 id 一份、独立加锁"模式，但 key 是 data_source_id 而不是
    /// workspace_id——两套状态机除了都复用 `ChangeStore` 类型外互不相干。
    pub sql_changes: Arc<RwLock<HashMap<Uuid, Arc<Mutex<ChangeStore>>>>>,
    pub sql_ai_assistant: Arc<SqlAiAssistant>,

    // --- SQL Agent（2026-09 用户要求：AI 工具要和"工作区"编程助手一样是真正
    // 的多轮 Agent，而不是单次生成/解释/优化那种一次性请求，见 `sql::agent`）。
    // 字段命名/结构和上面 `coding_*` 系列一一对应，key 从 workspace_id 换成
    // data_source_id，复用同一批基础设施类型（`CommandConfirmRegistry`/
    // `QuestionRegistry`）而不是另建一套。
    /// SQL Agent 会话，key 为 data_source_id——一个数据源最多一个活跃会话，
    /// 和 `coding_sessions` 按 workspace_id 一份是同一种模式。
    pub sql_agent_sessions: Arc<RwLock<HashMap<Uuid, Arc<Mutex<SqlAgentSession>>>>>,
    /// `run_query` 遇到需要确认的语句（UPDATE/DELETE/DDL）时用来等前端弹窗
    /// 结果，和 `command_confirms` 是同一个类型、但故意是独立的一份注册表——
    /// 两边的 session_id 空间（workspace_id vs data_source_id 生成的会话 id）
    /// 不重叠，分开存避免出现"以为是同一个请求"的误用。
    pub sql_agent_confirms: CommandConfirmRegistry,
    /// `question` 工具的等待注册表，同上，和 `question_confirms` 类型相同、
    /// 实例独立。
    pub sql_agent_questions: QuestionRegistry,
    pub sql_agent_history: Arc<SqlAgentHistoryRepo>,
    /// "停止"按钮用的取消信号，key 为 data_source_id，和 `coding_cancel_tokens`
    /// 同样的独立锁模式（不能卡在等 `sql_agent_sessions` 那把锁）。
    pub sql_agent_cancel_tokens: Arc<StdMutex<HashMap<Uuid, tokio_util::sync::CancellationToken>>>,

    // HTTP 桌面已迁到 roc_desk-http（roc_desk_http::HttpAppState，单独 manage，
    // 见 lib.rs::run），不再是 AppState 的字段。
}
