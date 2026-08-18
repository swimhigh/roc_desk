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
pub mod pty;
pub mod ssh;
pub mod state;
pub mod workspace;

use std::collections::HashMap;
use std::sync::Arc;

use tauri::Manager;
use tokio::sync::RwLock;

use ai::{AiChatClient, AiProviderManager};
use coding::CommandConfirmRegistry;
use connection::ConnectionManager;
use credential::KeyringStore;
use db::repo::ai_providers_repo::AiProvidersRepo;
use db::repo::audit_log_repo::AuditLogRepo;
use db::repo::browser_history_repo::BrowserHistoryRepo;
use db::repo::connections_repo::ConnectionsRepo;
use db::repo::known_hosts_repo::KnownHostsRepo;
use db::repo::workspace_repo::WorkspaceRepo;
use log::{LogImporter, LogSearchEngine};
use pty::LocalPtyManager;
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
            let app_data_dir = exe_dir.join("data");
            std::fs::create_dir_all(&app_data_dir)?;
            let db_path = app_data_dir.join("roc_desk.db");

            let pool = db::pool::create_pool(&db_path)?;
            {
                let conn = pool.get()?;
                db::migrate::run_migrations(&conn)?;
            }

            let credential_store: Arc<dyn credential::CredentialStore> = Arc::new(KeyringStore);

            let connections_repo = Arc::new(ConnectionsRepo::new(pool.clone()));
            let connection_manager = Arc::new(ConnectionManager::new(connections_repo, credential_store.clone()));

            let known_hosts_repo = Arc::new(KnownHostsRepo::new(pool.clone()));
            let trust_prompts = TrustPromptRegistry::default();
            let verifier = Arc::new(KnownHostsVerifier::new(
                known_hosts_repo,
                trust_prompts.clone(),
                app.handle().clone(),
            ));
            let ssh_pool = Arc::new(SshConnectionPool::new(connection_manager.clone(), verifier));

            let workspace_repo = Arc::new(WorkspaceRepo::new(pool.clone()));
            let workspace_manager = Arc::new(WorkspaceManager::new(
                workspace_repo,
                connection_manager.clone(),
                ssh_pool.clone(),
            ));

            let log_engine = Arc::new(LogSearchEngine::new(pool.clone()));
            let log_importer = Arc::new(LogImporter::new(log_engine.clone(), app_data_dir.join("log_cache")));

            let ai_providers_repo = Arc::new(AiProvidersRepo::new(pool.clone()));
            let ai_provider_manager = Arc::new(AiProviderManager::new(ai_providers_repo, credential_store.clone()));
            let ai_chat_client = Arc::new(AiChatClient::new());

            let audit_log = Arc::new(AuditLogRepo::new(pool.clone()));
            let browser_history = Arc::new(BrowserHistoryRepo::new(pool.clone()));

            app.manage(AppState {
                db: pool,
                credential_store,
                connection_manager,
                ssh_pool,
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
                local_pty: Arc::new(LocalPtyManager::default()),
                browser_history,
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
            commands::fs::fs_search,
            commands::fs::fs_replace,
            commands::connection::connection_list,
            commands::connection::connection_create,
            commands::connection::connection_update,
            commands::connection::connection_delete,
            commands::ssh::ssh_connect,
            commands::ssh::ssh_open_shell,
            commands::ssh::ssh_write,
            commands::ssh::ssh_resize,
            commands::ssh::ssh_disconnect,
            commands::ssh::ssh_close_channel,
            commands::ssh::ssh_confirm_host_key,
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
            commands::coding::coding_set_mode,
            commands::coding::coding_set_auto_allow_readonly,
            commands::coding::coding_set_auto_git_commit,
            commands::coding::coding_send_message,
            commands::coding::coding_accept_change,
            commands::coding::coding_reject_change,
            commands::coding::coding_undo_change,
            commands::coding::coding_redo_change,
            commands::coding::coding_confirm_command,
            commands::pty::pty_open,
            commands::pty::pty_write,
            commands::pty::pty_resize,
            commands::pty::pty_close,
            commands::browser::browser_open,
            commands::browser::browser_set_bounds,
            commands::browser::browser_hide,
            commands::browser::browser_show,
            commands::browser::browser_close,
            commands::browser::browser_history_list,
            commands::browser::browser_history_remove,
            commands::browser::browser_history_clear,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
