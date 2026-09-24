pub mod agent;
pub mod agent_llm;
pub mod ai;
pub mod browser;
pub mod coding;
pub mod commands;
pub mod connection;
pub mod credential;
pub mod db;
pub mod fsops;
pub mod log;
pub mod mcp;
pub mod pty;
pub mod rdp;
pub mod sql;
pub mod ssh;
pub mod state;
pub mod symbols;
pub mod windows_context_menu;
pub mod workspace;

// Compatibility re-export while modules are migrated into roc_desk_core.
// New tool crates should import `roc_desk_core::error::AppError` directly.
pub use roc_desk_core::error;

use std::collections::HashMap;
use std::sync::Arc;

use tauri::{Emitter, Manager};
use tokio::sync::RwLock;

use agent::{AgentCertVerifier, AgentConnectionPool, AgentTrustPromptRegistry};
use ai::{AiChatClient, AiProviderManager, AiRuntime};
use coding::CommandConfirmRegistry;
use connection::{ConnectionGroupManager, ConnectionManager};
use credential::KeyringStore;
use db::repo::agent_known_hosts_repo::AgentKnownHostsRepo;
use db::repo::ai_providers_repo::AiProvidersRepo;
use db::repo::audit_log_repo::AuditLogRepo;
use db::repo::browser_history_repo::BrowserHistoryRepo;
use db::repo::coding_history_repo::CodingHistoryRepo;
use db::repo::ai_evidence_repo::AiEvidenceRepo;
use db::repo::connection_groups_repo::ConnectionGroupsRepo;
use db::repo::connections_repo::ConnectionsRepo;
use db::repo::known_hosts_repo::KnownHostsRepo;
use db::repo::mcp_servers_repo::McpServersRepo;
use db::repo::permission_rules_repo::PermissionRulesRepo;
use db::repo::sql_agent_history_repo::SqlAgentHistoryRepo;
use db::repo::transfer_log_repo::TransferLogRepo;
use db::repo::workspace_module_links_repo::WorkspaceModuleLinksRepo;
use db::repo::workspace_repo::WorkspaceRepo;
use log::{LogImporter, LogSearchEngine};
use mcp::McpServerManager;
use pty::LocalPtyManager;
use rdp::RdpSessionManager;
use sql::ai_assistant::SqlAiAssistant;
use ssh::{KnownHostsVerifier, SshConnectionPool, TrustPromptRegistry};
use state::AppState;
use workspace::WorkspaceManager;

/// 数据库/日志等可变内容放在 exe 同级的 `.rock_desk` 子目录，不用系统 AppData——
/// 便于整个安装目录整体迁移/备份，也符合便携部署的预期（不打包进 exe 本身，是运行时
/// 在旁边生成的独立目录）。从 `run()` 顶层提出来（而不是留在 `.setup()` 闭包里）是
/// 因为 `init_logging` 也要用这个目录，必须在 `tauri::Builder` 跑起来之前就拿到手，
/// 这样从进程启动的第一行代码开始所有日志/panic 都能落盘，不留"最初这段时间出的错
/// 没记录"的空档。
fn resolve_app_data_dir() -> std::io::Result<std::path::PathBuf> {
    let exe_dir = std::env::current_exe()?
        .parent()
        .ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::Other, "无法定位可执行文件所在目录")
        })?
        .to_path_buf();
    let mut app_data_dir = exe_dir.join(".rock_desk");
    let legacy_data_dir = exe_dir.join("data");
    if !app_data_dir.exists() && legacy_data_dir.exists() {
        // 一次性迁移旧版本数据，保留连接、Provider、日志索引等已有配置。这一步还在
        // tracing 初始化之前，迁移失败时没有日志系统可用，只能眼下 eprintln 一句
        // （windowed 发布版没有控制台，这行基本只有开发模式下能看见，但不影响后面
        // 用 legacy_data_dir 兜底继续跑）。
        if let Err(error) = std::fs::rename(&legacy_data_dir, &app_data_dir) {
            eprintln!("failed to migrate data directory to .rock_desk: {error}; using legacy data directory for this run");
            app_data_dir = legacy_data_dir;
        }
    }
    std::fs::create_dir_all(&app_data_dir)?;
    Ok(app_data_dir)
}

/// 从命令行参数里挑出"看起来像文件路径"的那些——Windows"打开方式"/双击已关联
/// 文件时，资源管理器会把文件的完整路径直接拼进 argv（`roc_desk.exe "C:\a\b.txt"`），
/// 不带任何前缀标记；这里只排除看起来像 flag 的参数（`-` 开头，目前程序没有
/// 任何命令行 flag，纯粹是防御性过滤，避免以后加了 flag 却被这里当成路径打开）。
/// 调用方（冷启动 / `tauri-plugin-single-instance` 转发）各自负责跳过 argv[0]。
fn extract_open_paths(args: &[String]) -> Vec<String> {
    args.iter()
        .filter(|a| !a.starts_with('-'))
        .cloned()
        .collect()
}

/// 从 `--key=value` 形式的参数里取值（`docs/HOME_MODES_DESIGN.md` §3.5 的
/// `--mode`/`--open`）。用 `starts_with`/切片而不是引入一个完整的 CLI 解析 crate——
/// 这个程序目前只有这两个内部启动参数，不面向终端用户暴露 `--help` 之类的东西，
/// 犯不上为此加依赖。
fn parse_arg_value(args: &[String], key: &str) -> Option<String> {
    let prefix = format!("--{key}=");
    args.iter()
        .find_map(|a| a.strip_prefix(prefix.as_str()).map(|v| v.to_string()))
}

/// 日志落盘（同时保留 stdout，开发模式下 `cargo tauri dev` 挂着控制台还是照常能看）。
/// 用阻塞写入的 `RollingFileAppender`（不是 `tracing_appender::non_blocking` 那种
/// 后台线程+缓冲的写法）——目的就是崩溃时这一条日志已经真的落盘了，不会因为进程
/// 退出得太快、后台刷盘线程没来得及跑完而把最关键的这条丢掉。日志量本来就不大
/// （用户操作触发的业务事件，不是逐字节的高频日志），阻塞写入的开销可以忽略。
fn init_logging(log_dir: &std::path::Path) {
    let _ = std::fs::create_dir_all(log_dir);
    let file_writer = tracing_appender::rolling::daily(log_dir, "roc_desk.log");
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    use tracing_subscriber::prelude::*;
    tracing_subscriber::registry()
        .with(env_filter)
        .with(tracing_subscriber::fmt::layer())
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(file_writer)
                .with_ansi(false),
        )
        .init();
}

/// 之前完全没有 panic hook：任何线程 panic 都是"无声消失"——windowed 发布版没有
/// 控制台，默认 panic hook 打印到 stderr 等于打进了黑洞，用户看到的就是"程序突然
/// 不见了"，什么线索都留不下（2026-08-28 用户反馈）。这里不改变 panic 本身的行为
/// （该 unwind 还是 unwind，该终止哪个线程还是终止哪个线程），只是在默认处理之前
/// 先把完整信息记进日志文件。`Backtrace::force_capture` 无视 `RUST_BACKTRACE`
/// 环境变量强制抓栈——打包给用户的 exe 没人会去设这个环境变量，不强制就等于没有。
fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "unknown".to_string());
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "非字符串 panic payload".to_string());
        let backtrace = std::backtrace::Backtrace::force_capture();
        tracing::error!(target: "panic", %location, %message, %backtrace, "程序发生 panic");
        default_hook(info);
    }));
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app_data_dir = match resolve_app_data_dir() {
        Ok(dir) => dir,
        Err(error) => {
            // 连数据目录都定位不了，日志系统也没法初始化——这是极端边缘情况
            // （比如 current_exe() 失败），只能眼下退出，没有更好的兜底。
            eprintln!("无法初始化数据目录，程序退出: {error}");
            return;
        }
    };
    init_logging(&app_data_dir.join("logs"));
    install_panic_hook();

    // 首页/工作模块架构（`docs/HOME_MODES_DESIGN.md`）：不带 `--mode` 启动的是首页
    // （启动器）进程，带了 `--mode` 的是首页 spawn 出来的某个模块窗口（或者用户自己
    // 拿命令行/快捷方式直接指定）。`argv[0]` 是可执行文件路径本身，要跳过。
    let cli_args: Vec<String> = std::env::args().skip(1).collect();
    let launch_mode = parse_arg_value(&cli_args, "mode");
    let launch_open = parse_arg_value(&cli_args, "open");
    let is_launcher = launch_mode.is_none();

    let mut builder = tauri::Builder::default();
    if is_launcher {
        // `single-instance` 只对首页角色生效——它天生就应该是全局唯一的一个窗口。
        // 模块窗口故意不注册这个插件：不然首页 spawn 出来开工作区/编辑器等模块的
        // 子进程一启动就会撞上这同一个全局互斥体，被首页"吞掉"（转发参数、聚焦
        // 首页、自己退出），完全没法达到"点一个模块另开一个真进程真窗口"的效果
        // （见 `docs/HOME_MODES_DESIGN.md` §2.2）。副作用是"同一个模块窗口的固定
        // 快捷方式被手滑连点两次"不会去重，直接开两个窗口——这在设计文档里是
        // 明确接受的取舍（§3.7：那类去重是可选便利，不是本次要交付的核心诉求）。
        //
        // 必须是第一个注册的插件（官方文档要求）：拦截"已有实例在跑，这次启动应该
        // 转发给它"的情况，发生在窗口/其它插件初始化之前。回调里把新进程 argv 里的
        // 文件路径（Windows"打开方式"/双击已关联文件，2026-09-03 需求）转发给已有
        // 窗口，而不是真的再起一个进程实例；模块窗口点"返回首页"重新拉起一个不带
        // `--mode` 的进程时，也是走这同一条路径被转发/聚焦（`commands::launcher::
        // spawn_module_window` 的文档注释里有完整说明）。
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            let paths = extract_open_paths(argv.get(1..).unwrap_or(&[]));
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
            if !paths.is_empty() {
                let _ = app.emit("open-file-paths", paths);
            }
        }));
    }

    builder
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(move |app| {
            // 首页默认最大化窗口（`docs/HOME_MODES_DESIGN.md` §3.4："类似桌面首页"
            // 的定位，不是一个可以随便缩小的工具面板）；模块窗口保持 `tauri.conf.json`
            // 里配置的默认尺寸，不强制最大化。
            //
            // `tauri.conf.json` 里这个窗口声明了 `"visible": false`——窗口创建时
            // 先按声明的 1200x760 尺寸建好但不显示，这里先 `maximize()` 再 `show()`，
            // 用户才不会看到"先冒出一个小窗口、紧接着跳变成最大化"这一下（真实反馈，
            // 2026-09-04："明显看到了从一个固定窗口再到最大化窗口的一个过程"）——
            // 早先直接对已经显示的窗口调 `maximize()`，从 WebView2 完成首帧渲染到
            // `maximize()` 真正生效之间那一小段时间窗口就是按小尺寸可见的。
            if let Some(window) = app.get_webview_window("main") {
                if is_launcher {
                    let _ = window.maximize();
                }
                let _ = window.show();
            }

            let db_path = app_data_dir.join("roc_desk.db");

            let pool = db::pool::create_pool(&db_path)?;
            {
                let conn = pool.get()?;
                db::migrate::run_main_migrations(&conn)?;
            }

            // 会话（SSH/RDP 连接档案 + 分组 + known_hosts）和工作区数据分别存独立的
            // 数据库文件，不再和其它业务数据挤在同一个 roc_desk.db 里（用户 2026-08-25
            // 需求："SESSION和工作区的数据存不同文件夹里"）——两者生命周期和使用场景
            // 不同：前者是"服务器通讯录"，后者是"某次打开过的项目目录"，分开存也方便
            // 以后单独备份/同步其中一份。`*_db_is_new` 必须在 `create_pool` 之前算，
            // 拿到第一个连接就会把文件建出来，晚一步判断永远是 false。
            let sessions_dir = app_data_dir.join("sessions");
            std::fs::create_dir_all(&sessions_dir)?;
            let sessions_db_path = sessions_dir.join("sessions.db");
            let sessions_db_is_new = !sessions_db_path.exists();
            let sessions_pool = db::pool::create_pool(&sessions_db_path)?;
            {
                let conn = sessions_pool.get()?;
                db::migrate::run_sessions_migrations(&conn)?;
                if sessions_db_is_new {
                    db::migrate::migrate_legacy_data(
                        &db_path,
                        &conn,
                        &["connection_groups", "connections", "known_hosts"],
                    )?;
                }
            }

            let workspaces_dir = app_data_dir.join("workspaces");
            std::fs::create_dir_all(&workspaces_dir)?;
            let workspaces_db_path = workspaces_dir.join("workspaces.db");
            let workspaces_db_is_new = !workspaces_db_path.exists();
            let workspaces_pool = db::pool::create_pool(&workspaces_db_path)?;
            {
                let conn = workspaces_pool.get()?;
                db::migrate::run_workspaces_migrations(&conn)?;
                if workspaces_db_is_new {
                    db::migrate::migrate_legacy_data(&db_path, &conn, &["workspaces"])?;
                }
            }

            let credential_store: Arc<dyn credential::CredentialStore> = Arc::new(KeyringStore);

            let connections_repo = Arc::new(ConnectionsRepo::new(sessions_pool.clone()));
            let connection_manager = Arc::new(ConnectionManager::new(
                connections_repo,
                credential_store.clone(),
            ));
            let connection_groups_repo = Arc::new(ConnectionGroupsRepo::new(sessions_pool.clone()));
            let connection_group_manager =
                Arc::new(ConnectionGroupManager::new(connection_groups_repo));

            let known_hosts_repo = Arc::new(KnownHostsRepo::new(sessions_pool.clone()));
            let trust_prompts = TrustPromptRegistry::default();
            let verifier = Arc::new(KnownHostsVerifier::new(
                known_hosts_repo,
                trust_prompts.clone(),
                app.handle().clone(),
            ));
            let ssh_pool = Arc::new(SshConnectionPool::new(connection_manager.clone(), verifier));
            let rdp_sessions = Arc::new(RdpSessionManager::new(connection_manager.clone()));

            let agent_known_hosts_repo = Arc::new(AgentKnownHostsRepo::new(sessions_pool.clone()));
            let agent_trust_prompts = AgentTrustPromptRegistry::default();
            let agent_cert_verifier = Arc::new(AgentCertVerifier::new(
                agent_known_hosts_repo,
                agent_trust_prompts.clone(),
                app.handle().clone(),
            ));
            let agent_pool = Arc::new(AgentConnectionPool::new(
                connection_manager.clone(),
                agent_cert_verifier,
            ));

            let workspace_repo = Arc::new(WorkspaceRepo::new(workspaces_pool.clone()));
            let workspace_manager = Arc::new(WorkspaceManager::new(
                workspace_repo,
                connection_manager.clone(),
                ssh_pool.clone(),
                agent_pool.clone(),
                workspaces_dir,
            ));

            let workspace_module_links = Arc::new(WorkspaceModuleLinksRepo::new(workspaces_pool.clone()));

            // HTTP 桌面（roc_desk-http，见 docs/MULTI_REPO_SPLIT_PROGRESS.md）：
            // 自带一份 SQLite 文件（请求历史/标签页元数据），不再挂在
            // `workspaces_pool` 上——命令层已经从 workspace_id 改成 root: String，
            // 不需要 FK 到 `workspaces(id)`。
            let http_app_state = roc_desk_http::HttpAppState::new(&app_data_dir.join("http_desk.db"))
                .expect("初始化 HTTP 桌面存储失败");

            let log_engine = Arc::new(LogSearchEngine::new(pool.clone()));
            let log_importer = Arc::new(LogImporter::new(
                log_engine.clone(),
                app_data_dir.join("log_cache"),
            ));

            let ai_providers_repo = Arc::new(AiProvidersRepo::new(pool.clone()));
            let ai_provider_manager = Arc::new(AiProviderManager::new(
                ai_providers_repo,
                credential_store.clone(),
            ));
            let ai_runtime = Arc::new(AiRuntime::new());
            let ai_chat_client = Arc::new(AiChatClient::from_runtime(&ai_runtime));

            let audit_log = Arc::new(AuditLogRepo::new(pool.clone()));
            let coding_history = Arc::new(CodingHistoryRepo::new(pool.clone()));
            let ai_evidence = Arc::new(AiEvidenceRepo::new(pool.clone()));
            let browser_history = Arc::new(BrowserHistoryRepo::new(pool.clone()));
            let permission_rules = Arc::new(PermissionRulesRepo::new(pool.clone()));
            let mcp_servers_repo = Arc::new(McpServersRepo::new(pool.clone()));
            let mcp_manager = Arc::new(McpServerManager::new(
                mcp_servers_repo,
                credential_store.clone(),
            ));
            let transfer_log = Arc::new(TransferLogRepo::new(pool.clone()));

            // SQL 桌面模块（docs/SQL_DESKTOP_PLAN.md）。数据源 CRUD/会话管理/查询
            // 执行/工作区标签页/导出导入已经迁到 roc_desk_sql（见
            // docs/MULTI_REPO_SPLIT_PROGRESS.md "SQL 工作台"一节）——
            // `roc_desk_sql::SqlAppState::new` 指向和主库同一个 `db_path`（不是
            // 单独一个文件），因为 AI 面板/SQL Agent 这些还没迁走的宿主代码
            // 需要看到和 SQL 工作台面板一样的数据源/查询历史，两者必须是同一份
            // 数据；`0001_sql_desktop` 这个 crate 内部的迁移名和宿主自己的
            // `0020_sql_desktop` 建的是同一套表（已核对过 schema 完全一致），
            // 先手动把这条迁移记进 `schema_migrations`，避免 `SqlAppState::new`
            // 对着已经建过表的库重新跑一遍 CREATE TABLE 报错。
            {
                let conn = pool.get()?;
                conn.execute_batch(
                    "CREATE TABLE IF NOT EXISTS schema_migrations (
                        name TEXT PRIMARY KEY,
                        applied_at TEXT NOT NULL DEFAULT (datetime('now'))
                    );
                    INSERT OR IGNORE INTO schema_migrations (name) VALUES ('0001_sql_desktop');",
                )?;
            }
            let sql_app_state = roc_desk_sql::SqlAppState::new(&db_path, app_data_dir.clone())
                .expect("初始化 SQL 工作台存储失败");
            let sql_ai_assistant = Arc::new(SqlAiAssistant::new(
                ai_chat_client.clone(),
                ai_provider_manager.clone(),
            ));
            let sql_agent_history = Arc::new(SqlAgentHistoryRepo::new(pool.clone()));

            app.manage(AppState {
                db: pool,
                credential_store,
                connection_manager,
                connection_group_manager,
                ssh_pool,
                rdp_sessions,
                trust_prompts,
                agent_pool,
                agent_trust_prompts,
                workspace_manager,
                workspaces: Arc::new(RwLock::new(HashMap::new())),
                workspace_module_links,
                log_engine,
                log_importer,
                ai_provider_manager,
                ai_runtime,
                ai_chat_client,
                coding_sessions: Arc::new(RwLock::new(HashMap::new())),
                coding_changes: Arc::new(RwLock::new(HashMap::new())),
                command_confirms: CommandConfirmRegistry::default(),
                audit_log,
                coding_history,
                ai_evidence,
                local_pty: Arc::new(LocalPtyManager::default()),
                browser_history,
                active_search: Arc::new(std::sync::Mutex::new(None)),
                cancelled_transfers: Arc::new(std::sync::Mutex::new(
                    std::collections::HashSet::new(),
                )),
                transfer_log,
                permission_rules,
                question_confirms: coding::QuestionRegistry::default(),
                mcp_manager,
                pending_open_paths: std::sync::Mutex::new(extract_open_paths(
                    &std::env::args().skip(1).collect::<Vec<_>>(),
                )),
                launch_mode: launch_mode.clone(),
                launch_open: launch_open.clone(),
                coding_cancel_tokens: Arc::new(std::sync::Mutex::new(
                    std::collections::HashMap::new(),
                )),
                coding_pending_injections: Arc::new(std::sync::Mutex::new(
                    std::collections::HashMap::new(),
                )),
                symbol_indexes: Arc::new(RwLock::new(HashMap::new())),
                sql_changes: Arc::new(RwLock::new(HashMap::new())),
                sql_ai_assistant,
                sql_agent_sessions: Arc::new(RwLock::new(HashMap::new())),
                sql_agent_confirms: CommandConfirmRegistry::default(),
                sql_agent_questions: coding::QuestionRegistry::default(),
                sql_agent_history,
                sql_agent_cancel_tokens: Arc::new(std::sync::Mutex::new(
                    std::collections::HashMap::new(),
                )),
            });
            app.manage(http_app_state);
            app.manage(sql_app_state);

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            windows_context_menu::show_windows_context_menu,
            commands::launcher::get_launch_context,
            commands::launcher::spawn_module_window,
            commands::workspace::workspace_list_recent,
            commands::workspace::workspace_open_local,
            commands::workspace::workspace_open_remote,
            commands::workspace::workspace_remove_recent,
            commands::workspace::workspace_update_path,
            commands::workspace::workspace_close,
            commands::workspace::workspace_update_last_sftp_paths,
            commands::workspace::workspace_list_for_module,
            commands::workspace::workspace_add_module_link,
            commands::workspace::workspace_remove_module_link,
            commands::fs::fs_list_dir,
            commands::fs::fs_read_file,
            commands::fs::fs_write_file,
            commands::fs::fs_read_file_with_encoding,
            commands::fs::fs_write_file_with_encoding,
            commands::fs::fs_supported_encodings,
            commands::fs::fs_delete,
            commands::fs::fs_rename,
            commands::fs::fs_copy,
            commands::fs::fs_create_dir,
            commands::fs::fs_search_stream,
            commands::fs::fs_search_cancel,
            commands::fs::fs_replace,
            commands::fs::fs_read_binary_preview,
            commands::fs::fs_open_externally,
            commands::fs::fs_convert_legacy_office_to_pdf,
            commands::fs::fs_inspect_binary,
            commands::fs::fs_peek_is_binary,
            commands::fs::fs_inspect_jar,
            roc_desk_editor::ocr::editor_ocr_image,
            commands::symbols::symbols_build_index,
            commands::symbols::symbols_go_to_definition,
            commands::symbols::symbols_reindex_file,
            commands::connection::connection_list,
            commands::connection::connection_create,
            commands::connection::connection_update,
            commands::connection::connection_delete,
            commands::connection_group::connection_group_list,
            commands::connection_group::connection_group_create,
            commands::connection_group::connection_group_update,
            commands::connection_group::connection_group_delete,
            commands::ssh::ssh_connect,
            commands::ssh::ssh_open_shell,
            commands::ssh::ssh_write,
            commands::ssh::ssh_resize,
            commands::ssh::ssh_disconnect,
            commands::ssh::ssh_close_channel,
            commands::ssh::ssh_host_stats,
            commands::ssh::ssh_confirm_host_key,
            commands::agent::agent_connect,
            commands::agent::agent_disconnect,
            commands::agent::agent_confirm_cert,
            commands::agent::agent_test_connection,
            commands::agent::agent_list_dir,
            commands::agent::agent_list_roots,
            commands::agent::agent_open_shell,
            commands::agent::agent_write,
            commands::agent::agent_resize,
            commands::agent::agent_close_channel,
            commands::agent::agent_read_file,
            commands::agent::agent_write_file,
            commands::agent::agent_delete,
            commands::agent::agent_create_dir,
            commands::agent::agent_rename,
            commands::agent::agent_download,
            commands::agent::agent_upload,
            commands::agent::agent_download_entry,
            commands::agent::agent_upload_entry,
            commands::rdp::rdp_connect,
            commands::rdp::rdp_set_bounds,
            commands::rdp::rdp_hide,
            commands::rdp::rdp_show,
            commands::rdp::rdp_disconnect,
            commands::rdp::rdp_status,
            commands::sftp::sftp_list_dir,
            commands::sftp::sftp_read_file,
            commands::sftp::sftp_read_binary_preview,
            commands::sftp::sftp_open_externally,
            commands::sftp::sftp_convert_legacy_office_to_pdf,
            commands::sftp::sftp_inspect_binary,
            commands::sftp::sftp_peek_is_binary,
            commands::sftp::sftp_inspect_jar,
            commands::sftp::sftp_write_file,
            commands::sftp::sftp_download,
            commands::sftp::sftp_upload,
            commands::sftp::sftp_delete,
            commands::sftp::sftp_create_dir,
            commands::sftp::sftp_rename,
            commands::sftp::sftp_download_entry,
            commands::sftp::sftp_upload_entry,
            roc_desk_explorer::cmd::local_list_dir,
            roc_desk_explorer::cmd::local_home_dir,
            roc_desk_explorer::cmd::local_list_drives,
            roc_desk_explorer::cmd::local_is_dir,
            roc_desk_explorer::cmd::local_read_file,
            roc_desk_explorer::cmd::local_write_file,
            roc_desk_explorer::cmd::local_read_file_with_encoding,
            roc_desk_explorer::cmd::local_write_file_with_encoding,
            roc_desk_explorer::cmd::local_read_binary_preview,
            roc_desk_explorer::cmd::local_open_externally,
            roc_desk_explorer::cmd::local_convert_legacy_office_to_pdf,
            roc_desk_explorer::cmd::local_inspect_binary,
            roc_desk_explorer::cmd::local_peek_is_binary,
            roc_desk_explorer::cmd::local_inspect_jar,
            roc_desk_explorer::cmd::local_delete,
            roc_desk_explorer::cmd::local_rename,
            roc_desk_explorer::cmd::local_copy,
            roc_desk_explorer::cmd::local_create_dir,
            roc_desk_explorer::cmd::local_move,
            commands::local_fs::take_pending_open_paths,
            commands::transfer::transfer_cancel,
            commands::transfer::transfer_log_list,
            commands::transfer::transfer_log_clear,
            commands::log_search::log_search_index,
            commands::log_search::log_search_live,
            commands::log_search::log_import_remote_paths,
            commands::log_search::log_import_local_paths,
            commands::log_search::log_index_stats,
            commands::log_search::log_index_clear,
            commands::ai::ai_provider_list,
            commands::ai::ai_provider_create,
            commands::ai::ai_provider_update,
            commands::ai::ai_provider_delete,
            commands::ai::ai_provider_list_models,
            commands::ai::ai_chat_send,
            commands::ai::ai_chat_cancel,
            commands::coding::coding_start,
            commands::coding::coding_new_session,
            commands::coding::coding_close,
            commands::coding::coding_set_mode,
            commands::coding::coding_set_provider,
            commands::coding::coding_set_auto_allow_readonly,
            commands::coding::coding_set_auto_git_commit,
            commands::coding::coding_set_full_auto,
            commands::coding::coding_set_auto_apply_changes,
            commands::coding::coding_send_message,
            commands::coding::coding_inject_message,
            commands::coding::coding_cancel_turn,
            commands::coding::coding_optimize_prompt,
            commands::coding::coding_accept_change,
            commands::coding::coding_reject_change,
            commands::coding::coding_undo_change,
            commands::coding::coding_redo_change,
            commands::coding::coding_revert_turn,
            commands::coding::coding_confirm_command,
            commands::coding::coding_history_list,
            commands::coding::coding_history_get,
            commands::coding::coding_history_resume,
            commands::coding::coding_history_save,
            commands::coding::coding_history_rename,
            commands::coding::coding_history_delete,
            commands::coding::coding_answer_question,
            commands::coding::permission_rule_list,
            commands::coding::permission_rule_create,
            commands::coding::permission_rule_delete,
            commands::coding::mcp_server_list,
            commands::coding::mcp_server_create,
            commands::coding::mcp_server_update,
            commands::coding::mcp_server_delete,
            commands::coding::skill_list,
            commands::coding::skill_import,
            commands::coding::skill_delete,
            commands::pty::pty_open,
            commands::pty::pty_write,
            commands::pty::pty_resize,
            commands::pty::pty_close,
            commands::browser::browser_open,
            commands::browser::browser_set_bounds,
            commands::browser::browser_hide,
            commands::browser::browser_show,
            commands::browser::browser_close,
            commands::browser::browser_close_all,
            commands::browser::browser_history_list,
            commands::browser::browser_history_remove,
            commands::browser::browser_history_clear,
            commands::diagnostics::log_frontend_error,
            roc_desk_sql::cmd::sql_list_data_sources,
            roc_desk_sql::cmd::sql_save_data_source,
            roc_desk_sql::cmd::sql_delete_data_source,
            roc_desk_sql::cmd::sql_test_connection,
            roc_desk_sql::cmd::sql_open_session,
            roc_desk_sql::cmd::sql_close_session,
            roc_desk_sql::cmd::sql_list_databases,
            roc_desk_sql::cmd::sql_current_database,
            roc_desk_sql::cmd::sql_switch_database,
            roc_desk_sql::cmd::sql_list_objects,
            roc_desk_sql::cmd::sql_preview_template,
            roc_desk_sql::cmd::sql_describe_object,
            roc_desk_sql::cmd::sql_table_page,
            roc_desk_sql::cmd::sql_table_row_count,
            roc_desk_sql::cmd::sql_table_update_cell,
            roc_desk_sql::cmd::sql_table_delete_row,
            roc_desk_sql::cmd::sql_table_insert_row,
            roc_desk_sql::cmd::sql_generate_alter_table,
            roc_desk_sql::cmd::sql_export_start,
            roc_desk_sql::cmd::sql_export_poll,
            roc_desk_sql::cmd::sql_export_cancel,
            roc_desk_sql::cmd::sql_import_start,
            roc_desk_sql::cmd::sql_import_poll,
            roc_desk_sql::cmd::sql_import_cancel,
            roc_desk_sql::cmd::sql_write_text_file,
            commands::sql_agent::sql_agent_start,
            commands::sql_agent::sql_agent_new_session,
            commands::sql_agent::sql_agent_close,
            commands::sql_agent::sql_agent_set_provider,
            commands::sql_agent::sql_agent_send_message,
            commands::sql_agent::sql_agent_cancel_turn,
            commands::sql_agent::sql_agent_resolve_confirm,
            commands::sql_agent::sql_agent_answer_question,
            commands::sql_agent::sql_agent_history_list,
            commands::sql_agent::sql_agent_history_get,
            commands::sql_agent::sql_agent_history_save,
            commands::sql_agent::sql_agent_history_resume,
            commands::sql_agent::sql_agent_history_rename,
            commands::sql_agent::sql_agent_history_delete,
            roc_desk_sql::cmd::sql_execute,
            roc_desk_sql::cmd::sql_poll_query,
            roc_desk_sql::cmd::sql_cancel,
            roc_desk_sql::cmd::sql_confirm_write,
            roc_desk_sql::cmd::sql_rollback_write,
            roc_desk_sql::cmd::sql_explain,
            roc_desk_sql::cmd::sql_query_history,
            roc_desk_sql::cmd::sql_workspace_tabs_list,
            roc_desk_sql::cmd::sql_workspace_tab_create,
            roc_desk_sql::cmd::sql_workspace_tab_delete,
            roc_desk_sql::cmd::sql_workspace_tab_update_meta,
            roc_desk_sql::cmd::sql_tab_read_content,
            roc_desk_sql::cmd::sql_tab_write_content,
            commands::sql::sql_ai_generate,
            commands::sql::sql_ai_explain,
            commands::sql::sql_ai_optimize,
            commands::sql::sql_ai_fix_error,
            commands::sql::sql_accept_change,
            commands::sql::sql_reject_change,
            commands::sql::sql_undo_change,
            commands::sql::sql_revert_turn,
            roc_desk_http::cmd::http_list_collections,
            roc_desk_http::cmd::http_create_collection,
            roc_desk_http::cmd::http_rename_collection,
            roc_desk_http::cmd::http_delete_collection,
            roc_desk_http::cmd::http_list_requests,
            roc_desk_http::cmd::http_get_request,
            roc_desk_http::cmd::http_create_request,
            roc_desk_http::cmd::http_save_request,
            roc_desk_http::cmd::http_delete_request,
            roc_desk_http::cmd::http_list_environments,
            roc_desk_http::cmd::http_save_environment,
            roc_desk_http::cmd::http_delete_environment,
            roc_desk_http::cmd::http_get_global_variables,
            roc_desk_http::cmd::http_save_global_variables,
            roc_desk_http::cmd::http_get_collection_meta,
            roc_desk_http::cmd::http_save_collection_meta,
            roc_desk_http::cmd::http_send_request,
            roc_desk_http::cmd::http_import_curl,
            roc_desk_http::cmd::http_import_postman,
            roc_desk_http::cmd::http_import_openapi,
            roc_desk_http::cmd::http_export_postman,
            roc_desk_http::cmd::http_list_history,
            roc_desk_http::cmd::http_get_history_detail,
            roc_desk_http::cmd::http_delete_history,
            roc_desk_http::cmd::http_clear_history,
            roc_desk_http::cmd::http_list_tabs,
            roc_desk_http::cmd::http_open_tab,
            roc_desk_http::cmd::http_close_tab,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            // RDP 内嵌窗口从 `SetParent` 子窗口改成属主窗口之后（见 `rdp/mod.rs`
            // 顶部"代价"一节），Windows 不再在主窗口销毁时自动级联关掉它——退出时
            // 必须显式收尾，否则 wfreerdp.exe 连带它的窗口会变成孤儿留在桌面上。
            if let tauri::RunEvent::ExitRequested { .. } = event {
                if let Some(state) = app_handle.try_state::<AppState>() {
                    let _ = state.rdp_sessions.disconnect_all();
                }
            }
        });
}
