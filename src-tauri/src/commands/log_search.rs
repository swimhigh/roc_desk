use tauri::{AppHandle, Emitter, State};
use uuid::Uuid;

use crate::error::AppError;
use crate::log::remote::{search_live, LiveSearchResult};
use crate::log::{IndexStats, LogImportOutcome, LogQuery, LogSearchResult};
use crate::state::AppState;

/// 模式 B：本地索引搜索（DESIGN.md §3.4.2），查 FTS5。
#[tauri::command]
pub async fn log_search_index(
    state: State<'_, AppState>,
    query: LogQuery,
) -> Result<Vec<LogSearchResult>, AppError> {
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
    let session = state.ssh_pool.get_or_connect(profile_id).await?;
    search_live(&session, &pattern, &path, is_regex).await
}

/// 把一批远程日志文件/目录下载并导入本地 FTS5 索引（DESIGN.md §3.4.1）。
/// `paths` 可以混着传文件和目录——目录只有 `recursive` 为真时才会被递归展开，
/// 否则报错（不猜用户是想导入目录下的哪个文件）。展开出的文件列表逐个下载+
/// 导入，每个文件开始前发一次 `log:import-progress` 事件（`requestId` 由调用方
/// 生成，用来在前端过滤出这一次导入自己的进度，不与其它并发的导入串台）——
/// 2026-09 用户反馈"只能一次导入一个文件，体验不好，想支持多选/整个目录"。
#[tauri::command]
pub async fn log_import_remote_paths(
    state: State<'_, AppState>,
    app_handle: AppHandle,
    profile_id: Uuid,
    paths: Vec<String>,
    recursive: bool,
    host_name: String,
    request_id: Uuid,
) -> Result<LogImportOutcome, AppError> {
    let file_ops = state.ssh_pool.get_file_ops(profile_id).await?;
    state
        .log_importer
        .import_remote_paths(&file_ops, &paths, &host_name, recursive, |path, done, total| {
            let _ = app_handle.emit(
                "log:import-progress",
                serde_json::json!({ "requestId": request_id, "path": path, "done": done, "total": total }),
            );
        })
        .await
}

/// 本地工作区的日志文件/目录直接导入 FTS5 索引，不用经过 SSH/SFTP
/// （DESIGN.md §3.4.1），批量语义和 `log_import_remote_paths` 对称。
#[tauri::command]
pub async fn log_import_local_paths(
    state: State<'_, AppState>,
    app_handle: AppHandle,
    paths: Vec<String>,
    recursive: bool,
    host_name: String,
    request_id: Uuid,
) -> Result<LogImportOutcome, AppError> {
    state
        .log_importer
        .import_local_paths(&paths, &host_name, recursive, |path, done, total| {
            let _ = app_handle.emit(
                "log:import-progress",
                serde_json::json!({ "requestId": request_id, "path": path, "done": done, "total": total }),
            );
        })
}

#[tauri::command]
pub async fn log_index_stats(state: State<'_, AppState>) -> Result<IndexStats, AppError> {
    state.log_engine.index_stats()
}

#[tauri::command]
pub async fn log_index_clear(
    state: State<'_, AppState>,
    older_than_days: i64,
) -> Result<usize, AppError> {
    state.log_engine.clear_older_than(older_than_days)
}
