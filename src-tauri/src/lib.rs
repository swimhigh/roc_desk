pub mod ai;
pub mod browser;
pub mod coding;
pub mod commands;
pub mod connection;
pub mod credential;
pub mod db;
pub mod error;
pub mod fsops;
pub mod log;
pub mod mcp;
pub mod pty;
pub mod rdp;
pub mod ssh;
pub mod state;
pub mod workspace;

use std::collections::HashMap;
use std::sync::Arc;

use tauri::Manager;
use tokio::sync::RwLock;

use ai::{AiChatClient, AiProviderManager};
use coding::CommandConfirmRegistry;
use connection::{ConnectionGroupManager, ConnectionManager};
use credential::KeyringStore;
use db::repo::ai_providers_repo::AiProvidersRepo;
use db::repo::audit_log_repo::AuditLogRepo;
use db::repo::browser_history_repo::BrowserHistoryRepo;
use db::repo::connection_groups_repo::ConnectionGroupsRepo;
use db::repo::connections_repo::ConnectionsRepo;
use db::repo::coding_history_repo::CodingHistoryRepo;
use db::repo::known_hosts_repo::KnownHostsRepo;
use db::repo::mcp_servers_repo::McpServersRepo;
use db::repo::permission_rules_repo::PermissionRulesRepo;
use db::repo::workspace_repo::WorkspaceRepo;
use log::{LogImporter, LogSearchEngine};
use mcp::McpServerManager;
use pty::LocalPtyManager;
use rdp::RdpSessionManager;
use ssh::{KnownHostsVerifier, SshConnectionPool, TrustPromptRegistry};
use state::AppState;
use workspace::WorkspaceManager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt::init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // 数据库/日志缓存等可变内容放在 exe 同级的 data 子目录，不用系统 AppData——
            // 便于整个安装目录整体迁移/备份，也符合便携部署的预期（不打包进 exe 本身，
            // 是运行时在旁边生成的独立目录）。
            let exe_dir = std::env::current_exe()?
                .parent()
                .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::Other, "无法定位可执行文件所在目录"))?
                .to_path_buf();
            // 统一应用缓存目录：数据库、日志缓存、编程助手历史等都放在这里。
            // 使用项目约定的隐藏目录名，避免缓存散落在各个模块或临时目录中。
            let mut app_data_dir = exe_dir.join(".rock_desk");
            let legacy_data_dir = exe_dir.join("data");
            if !app_data_dir.exists() && legacy_data_dir.exists() {
                // 一次性迁移旧版本数据，保留连接、Provider、日志索引等已有配置。
                if let Err(error) = std::fs::rename(&legacy_data_dir, &app_data_dir) {
                    tracing::warn!(%error, "failed to migrate data directory to .rock_desk; using legacy data directory for this run");
                    app_data_dir = legacy_data_dir;
                }
            }
            std::fs::create_dir_all(&app_data_dir)?;
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
                    db::migrate::migrate_legacy_data(&db_path, &conn, &["connection_groups", "connections", "known_hosts"])?;
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
            let connection_manager = Arc::new(ConnectionManager::new(connections_repo, credential_store.clone()));
            let connection_groups_repo = Arc::new(ConnectionGroupsRepo::new(sessions_pool.clone()));
            let connection_group_manager = Arc::new(ConnectionGroupManager::new(connection_groups_repo));

            let known_hosts_repo = Arc::new(KnownHostsRepo::new(sessions_pool.clone()));
            let trust_prompts = TrustPromptRegistry::default();
            let verifier = Arc::new(KnownHostsVerifier::new(
                known_hosts_repo,
                trust_prompts.clone(),
                app.handle().clone(),
            ));
            let ssh_pool = Arc::new(SshConnectionPool::new(connection_manager.clone(), verifier));
            let rdp_sessions = Arc::new(RdpSessionManager::new(connection_manager.clone()));

            let workspace_repo = Arc::new(WorkspaceRepo::new(workspaces_pool.clone()));
            let workspace_manager = Arc::new(WorkspaceManager::new(
                workspace_repo,
                connection_manager.clone(),
                ssh_pool.clone(),
                workspaces_dir,
            ));

            let log_engine = Arc::new(LogSearchEngine::new(pool.clone()));
            let log_importer = Arc::new(LogImporter::new(log_engine.clone(), app_data_dir.join("log_cache")));

            let ai_providers_repo = Arc::new(AiProvidersRepo::new(pool.clone()));
            let ai_provider_manager = Arc::new(AiProviderManager::new(ai_providers_repo, credential_store.clone()));
            let ai_chat_client = Arc::new(AiChatClient::new());

            let audit_log = Arc::new(AuditLogRepo::new(pool.clone()));
            let coding_history = Arc::new(CodingHistoryRepo::new(pool.clone()));
            let browser_history = Arc::new(BrowserHistoryRepo::new(pool.clone()));
            let permission_rules = Arc::new(PermissionRulesRepo::new(pool.clone()));
            let mcp_servers_repo = Arc::new(McpServersRepo::new(pool.clone()));
            let mcp_manager = Arc::new(McpServerManager::new(mcp_servers_repo, credential_store.clone()));

            app.manage(AppState {
                db: pool,
                credential_store,
                connection_manager,
                connection_group_manager,
                ssh_pool,
                rdp_sessions,
                trust_prompts,
                workspace_manager,
                workspaces: Arc::new(RwLock::new(HashMap::new())),
                log_engine,
                log_importer,
                ai_provider_manager,
                ai_chat_client,
                coding_sessions: Arc::new(RwLock::new(HashMap::new())),
                command_confirms: CommandConfirmRegistry::default(),
                audit_log,
                coding_history,
                local_pty: Arc::new(LocalPtyManager::default()),
                browser_history,
                active_search: Arc::new(std::sync::Mutex::new(None)),
                permission_rules,
                question_confirms: coding::QuestionRegistry::default(),
                mcp_manager,
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::workspace::workspace_list_recent,
            commands::workspace::workspace_open_local,
            commands::workspace::workspace_open_remote,
            commands::workspace::workspace_remove_recent,
            commands::workspace::workspace_close,
            commands::fs::fs_list_dir,
            commands::fs::fs_read_file,
            commands::fs::fs_write_file,
            commands::fs::fs_read_file_with_encoding,
            commands::fs::fs_write_file_with_encoding,
            commands::fs::fs_supported_encodings,
            commands::fs::fs_delete,
            commands::fs::fs_rename,
            commands::fs::fs_copy,
            commands::fs::fs_search_stream,
            commands::fs::fs_replace,
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
            commands::rdp::rdp_connect,
            commands::rdp::rdp_set_bounds,
            commands::rdp::rdp_hide,
            commands::rdp::rdp_show,
            commands::rdp::rdp_disconnect,
            commands::rdp::rdp_status,
            commands::sftp::sftp_list_dir,
            commands::sftp::sftp_read_file,
            commands::sftp::sftp_write_file,
            commands::sftp::sftp_download,
            commands::sftp::sftp_upload,
            commands::sftp::sftp_delete,
            commands::sftp::sftp_rename,
            commands::sftp::sftp_download_entry,
            commands::sftp::sftp_upload_entry,
            commands::local_fs::local_list_dir,
            commands::local_fs::local_home_dir,
            commands::log_search::log_search_index,
            commands::log_search::log_search_live,
            commands::log_search::log_import_file,
            commands::log_search::log_import_local_file,
            commands::log_search::log_index_stats,
            commands::log_search::log_index_clear,
            commands::ai::ai_provider_list,
            commands::ai::ai_provider_create,
            commands::ai::ai_provider_update,
            commands::ai::ai_provider_delete,
            commands::ai::ai_chat_send,
            commands::coding::coding_start,
            commands::coding::coding_new_session,
            commands::coding::coding_close,
            commands::coding::coding_set_mode,
            commands::coding::coding_set_provider,
            commands::coding::coding_set_auto_allow_readonly,
            commands::coding::coding_set_auto_git_commit,
            commands::coding::coding_send_message,
            commands::coding::coding_optimize_prompt,
            commands::coding::coding_accept_change,
            commands::coding::coding_reject_change,
            commands::coding::coding_undo_change,
            commands::coding::coding_redo_change,
            commands::coding::coding_confirm_command,
            commands::coding::coding_history_list,
            commands::coding::coding_history_get,
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
