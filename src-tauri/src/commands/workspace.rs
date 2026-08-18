use tauri::State;
use uuid::Uuid;

use crate::error::AppError;
use crate::state::AppState;
use crate::workspace::WorkspaceProfile;

/// 工作区选择页的"最近打开"列表（DESIGN.md §3.1.1）。
#[tauri::command]
pub async fn workspace_list_recent(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> Result<Vec<WorkspaceProfile>, AppError> {
    state.workspace_manager.list_recent(limit.unwrap_or(20))
}

/// 打开本地文件夹作为工作区；`path` 由前端原生目录选择器（tauri-plugin-dialog）给出。
#[tauri::command]
pub async fn workspace_open_local(
    state: State<'_, AppState>,
    path: String,
) -> Result<WorkspaceProfile, AppError> {
    let handle = state.workspace_manager.open_local(&path)?;
    let profile = handle.profile.clone();
    state.workspaces.write().await.insert(profile.id, handle);
    Ok(profile)
}

/// 连接远程主机并选择目录后调用；内部经连接池建连（含主机指纹校验），
/// 成功后打开一个绑定该主机的工作区（DESIGN.md §3.1.1）。
#[tauri::command]
pub async fn workspace_open_remote(
    state: State<'_, AppState>,
    connection_id: Uuid,
    remote_path: String,
) -> Result<WorkspaceProfile, AppError> {
    let handle = state.workspace_manager.open_remote(connection_id, &remote_path).await?;
    let profile = handle.profile.clone();
    state.workspaces.write().await.insert(profile.id, handle);
    Ok(profile)
}

#[tauri::command]
pub async fn workspace_remove_recent(state: State<'_, AppState>, id: Uuid) -> Result<(), AppError> {
    state.workspace_manager.remove_from_recent(id)
}

#[tauri::command]
pub async fn workspace_close(state: State<'_, AppState>, id: Uuid) -> Result<(), AppError> {
    state.workspaces.write().await.remove(&id);
    Ok(())
}
