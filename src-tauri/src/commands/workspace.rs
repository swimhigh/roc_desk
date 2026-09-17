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
    let handle = state
        .workspace_manager
        .open_remote(connection_id, &remote_path)
        .await?;
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
    state
        .workspace_manager
        .update_last_sftp_paths(id, &local_path, &remote_path)
}

/// 工作模块首页/选择页展示的"这个模块添加过的工作区"列表（2026-09 需求，见
/// `db::repo::workspace_module_links_repo` 文档）——和 `workspace_list_recent`
/// 不是一回事：后者是"系统里所有打开过的工作区"，供"从已有工作区中选择"这类
/// 挑选场景使用；这个是"当前模块卡片该显示哪些"，默认是空的，要显式
/// `workspace_add_module_link` 才会出现。
#[tauri::command]
pub async fn workspace_list_for_module(
    state: State<'_, AppState>,
    module: String,
    limit: Option<usize>,
) -> Result<Vec<WorkspaceProfile>, AppError> {
    state
        .workspace_module_links
        .list_for_module(&module, limit.unwrap_or(100))
}

#[tauri::command]
pub async fn workspace_add_module_link(
    state: State<'_, AppState>,
    id: Uuid,
    module: String,
) -> Result<(), AppError> {
    state.workspace_module_links.add(id, &module)
}

/// 从某个模块的列表里移除——只解除关联，不删除工作区本身（同一个工作区可能还
/// 关联着其它模块，也可能用户只是想让这张卡片清净一点）。彻底忘记一个工作区
/// 仍然走 `workspace_remove_recent`。
#[tauri::command]
pub async fn workspace_remove_module_link(
    state: State<'_, AppState>,
    id: Uuid,
    module: String,
) -> Result<(), AppError> {
    state.workspace_module_links.remove(id, &module)
}
