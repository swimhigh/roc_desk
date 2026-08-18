use tauri::State;
use uuid::Uuid;

use crate::error::AppError;
use crate::log::{IndexStats, LogQuery, LogSearchResult};
use crate::log::remote::{search_live, LiveSearchResult};
use crate::state::AppState;

/// 模式 B：本地索引搜索（DESIGN.md §3.4.2），查 FTS5。
#[tauri::command]
pub async fn log_search_index(state: State<'_, AppState>, query: LogQuery) -> Result<Vec<LogSearchResult>, AppError> {
    state.log_engine.search(&query)
}

/// 模式 A：远程实时搜索，通过 SSH 跑 `rg`/`grep`（DESIGN.md §3.4.2）。
#[tauri::command]
pub async fn log_search_live(
    state: State<'_, AppState>,
    profile_id: Uuid,
    pattern: String,
    path: String,
    is_regex: bool,
) -> Result<Vec<LiveSearchResult>, AppError> {
    let session = state
        .ssh_pool
        .get_or_connect(profile_id)
        .await?;
    search_live(&session, &pattern, &path, is_regex).await
}

/// 把远程日志文件下载并导入本地 FTS5 索引（DESIGN.md §3.4.1）。
#[tauri::command]
pub async fn log_import_file(
    state: State<'_, AppState>,
    profile_id: Uuid,
    remote_path: String,
    host_name: String,
) -> Result<usize, AppError> {
    let file_ops = state.ssh_pool.get_file_ops(profile_id).await?;
    state
        .log_importer
        .import_remote_file(&file_ops, &remote_path, &host_name)
        .await
}

/// 本地工作区的日志文件直接导入 FTS5 索引，不用经过 SSH/SFTP（DESIGN.md §3.4.1）。
#[tauri::command]
pub async fn log_import_local_file(
    state: State<'_, AppState>,
    path: String,
    host_name: String,
) -> Result<usize, AppError> {
    state.log_importer.import_local_file(&path, &host_name)
}

#[tauri::command]
pub async fn log_index_stats(state: State<'_, AppState>) -> Result<IndexStats, AppError> {
    state.log_engine.index_stats()
}

#[tauri::command]
pub async fn log_index_clear(state: State<'_, AppState>, older_than_days: i64) -> Result<usize, AppError> {
    state.log_engine.clear_older_than(older_than_days)
}
