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

/// 修改"最近工作区"里一条记录的目录（用户反馈：目录配错了之前只能删除重加）。
#[tauri::command]
pub async fn workspace_update_path(
    state: State<'_, AppState>,
    id: Uuid,
    new_path: String,
) -> Result<WorkspaceProfile, AppError> {
    state.workspace_manager.update_path(id, &new_path).await
}

#[tauri::command]
pub async fn workspace_close(state: State<'_, AppState>, id: Uuid) -> Result<(), AppError> {
    state.workspaces.write().await.remove(&id);
    Ok(())
}

/// SFTP/Agent 双栏浏览器每次导航都调一次（用户需求："下次启动工作区中的SFTP或
/// 文件传输时，直接定位到最后记住的目录"）——写完就地生效，不需要返回最新的
/// `WorkspaceProfile`：这次打开期间前端手上那份 `current` 就算过期也无所谓，
/// 下次真正重新打开这个工作区时 `workspace_open_local`/`workspace_open_remote`
/// 会取到最新值，中途没有谁会去读这两个字段。
#[tauri::command]
pub async fn workspace_update_last_sftp_paths(
    state: State<'_, AppState>,
    id: Uuid,
    local_path: String,
    remote_path: String,
) -> Result<(), AppError> {
    state.workspace_manager.update_last_sftp_paths(id, &local_path, &remote_path)
}
