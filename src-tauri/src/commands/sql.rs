use std::sync::Arc;

use tauri::{AppHandle, State};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::coding::{ChangeStore, CodingTarget, FileChange, FileSyncInfo};
use crate::error::AppError;
use crate::fsops::local::LocalFileOps;
use crate::fsops::FileOps;
use crate::state::AppState;
use roc_desk_sql::SqlAppState;

// ---------------------------------------------------------------------------
// AI 面板（方案 §8、§4.4）——生成/优化/修复走 ChangeStore 确认闸门，
// 解释直接返回文本。
//
// 数据源/查询历史/工作区标签页/标签页文件缓存这些字段已经迁移进
// `roc_desk_sql::SqlAppState`（见 lib.rs 里对 `roc_desk_sql::cmd::*` 的
// 注册），但 AI 面板本身（依赖 `crate::ai`/`crate::agent_llm`/
// `crate::coding::ChangeStore`，还没有 `roc_desk_core` 对应实现）留在宿主——
// 所以这里的函数需要同时拿 `State<AppState>`（AI provider/ChangeStore 注册表）
// 和 `State<SqlAppState>`（标签页文件路径/缓存目录），Tauri 原生支持一个
// 命令函数取多个不同类型的 `State<T>`，不需要额外机制。
// ---------------------------------------------------------------------------

async fn get_or_create_sql_change_store(
    state: &State<'_, AppState>,
    sql_state: &State<'_, SqlAppState>,
    data_source_id: Uuid,
) -> Arc<Mutex<ChangeStore>> {
    if let Some(store) = state.sql_changes.read().await.get(&data_source_id) {
        return store.clone();
    }
    let workspace_root = sql_state
        .workspace_cache
        .data_source_root(data_source_id)
        .to_string_lossy()
        .to_string();
    let file_ops: Arc<dyn FileOps> = Arc::new(LocalFileOps);
    let store = Arc::new(Mutex::new(ChangeStore::new(
        data_source_id,
        workspace_root,
        CodingTarget::Local,
        file_ops,
        false,
    )));
    state
        .sql_changes
        .write()
        .await
        .insert(data_source_id, store.clone());
    store
}

async fn stage_ai_result(
    state: &State<'_, AppState>,
    sql_state: &State<'_, SqlAppState>,
    app_handle: &AppHandle,
    data_source_id: Uuid,
    tab_id: Uuid,
    new_sql: String,
) -> Result<FileChange, AppError> {
    let tab = sql_state
        .workspace_tabs
        .get(tab_id)?
        .ok_or_else(|| AppError::NotFound(format!("tab not found: {tab_id}")))?;
    let abs_path = sql_state
        .workspace_cache
        .guard_path(data_source_id, &tab.file_path)?
        .to_string_lossy()
        .to_string();
    let store = get_or_create_sql_change_store(state, sql_state, data_source_id).await;
    let mut guard = store.lock().await;
    let (change, _sync) = guard
        .stage(
            &abs_path,
            new_sql,
            Uuid::new_v4(),
            &state.ssh_pool,
            &state.agent_pool,
            app_handle,
        )
        .await?;
    Ok(change)
}

#[tauri::command]
pub async fn sql_ai_generate(
    state: State<'_, AppState>,
    sql_state: State<'_, SqlAppState>,
    app_handle: AppHandle,
    data_source_id: Uuid,
    tab_id: Uuid,
    provider_id: Uuid,
    instruction: String,
    schema_context: String,
    current_sql: String,
) -> Result<FileChange, AppError> {
    let sql = state
        .sql_ai_assistant
        .generate_sql(provider_id, &instruction, &schema_context, &current_sql)
        .await?;
    stage_ai_result(&state, &sql_state, &app_handle, data_source_id, tab_id, sql).await
}

#[tauri::command]
pub async fn sql_ai_explain(
    state: State<'_, AppState>,
    provider_id: Uuid,
    sql: String,
    schema_context: String,
) -> Result<String, AppError> {
    state.sql_ai_assistant.explain_sql(provider_id, &sql, &schema_context).await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn sql_ai_optimize(
    state: State<'_, AppState>,
    sql_state: State<'_, SqlAppState>,
    app_handle: AppHandle,
    data_source_id: Uuid,
    tab_id: Uuid,
    provider_id: Uuid,
    sql: String,
    explain_output: Option<String>,
    schema_context: String,
) -> Result<FileChange, AppError> {
    let optimized = state
        .sql_ai_assistant
        .optimize_sql(provider_id, &sql, explain_output.as_deref(), &schema_context)
        .await?;
    stage_ai_result(&state, &sql_state, &app_handle, data_source_id, tab_id, optimized).await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn sql_ai_fix_error(
    state: State<'_, AppState>,
    sql_state: State<'_, SqlAppState>,
    app_handle: AppHandle,
    data_source_id: Uuid,
    tab_id: Uuid,
    provider_id: Uuid,
    sql: String,
    error_message: String,
    schema_context: String,
) -> Result<FileChange, AppError> {
    let fixed = state
        .sql_ai_assistant
        .fix_error(provider_id, &sql, &error_message, &schema_context)
        .await?;
    stage_ai_result(&state, &sql_state, &app_handle, data_source_id, tab_id, fixed).await
}

#[tauri::command]
pub async fn sql_accept_change(
    state: State<'_, AppState>,
    sql_state: State<'_, SqlAppState>,
    app_handle: AppHandle,
    data_source_id: Uuid,
    change_id: Uuid,
) -> Result<FileSyncInfo, AppError> {
    let store = get_or_create_sql_change_store(&state, &sql_state, data_source_id).await;
    let mut guard = store.lock().await;
    guard
        .accept(change_id, &state.ssh_pool, &state.agent_pool, &app_handle)
        .await
}

#[tauri::command]
pub async fn sql_reject_change(
    state: State<'_, AppState>,
    sql_state: State<'_, SqlAppState>,
    data_source_id: Uuid,
    change_id: Uuid,
) -> Result<(), AppError> {
    let store = get_or_create_sql_change_store(&state, &sql_state, data_source_id).await;
    let mut guard = store.lock().await;
    guard.reject(change_id)
}

#[tauri::command]
pub async fn sql_undo_change(
    state: State<'_, AppState>,
    sql_state: State<'_, SqlAppState>,
    data_source_id: Uuid,
    change_id: Uuid,
) -> Result<FileSyncInfo, AppError> {
    let store = get_or_create_sql_change_store(&state, &sql_state, data_source_id).await;
    let mut guard = store.lock().await;
    guard.undo(change_id).await
}

#[tauri::command]
pub async fn sql_revert_turn(
    state: State<'_, AppState>,
    sql_state: State<'_, SqlAppState>,
    data_source_id: Uuid,
    turn_id: Uuid,
) -> Result<Vec<FileSyncInfo>, AppError> {
    let store = get_or_create_sql_change_store(&state, &sql_state, data_source_id).await;
    let mut guard = store.lock().await;
    guard.revert_turn(turn_id).await
}
