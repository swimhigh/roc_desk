use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, State};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::coding::{CodingMode, CodingSession, CodingTarget, FileChange};
use crate::error::AppError;
use crate::state::AppState;
use crate::workspace::WorkspaceKind;

#[derive(Serialize)]
pub struct CodingSessionInfo {
    pub id: Uuid,
    pub mode: CodingMode,
    pub target: CodingTarget,
    pub auto_allow_readonly: bool,
    pub git_repo: bool,
    pub auto_git_commit: bool,
    pub changes: Vec<FileChange>,
}

async fn session_info(session: &CodingSession) -> CodingSessionInfo {
    CodingSessionInfo {
        id: session.id,
        mode: session.mode,
        target: session.target.clone(),
        auto_allow_readonly: session.auto_allow_readonly,
        git_repo: session.git_repo,
        auto_git_commit: session.auto_git_commit,
        changes: session.changes.clone(),
    }
}

/// 自动绑定当前工作区打开（或复用已有的）编程助手会话（DESIGN.md §3.8.1）。
#[tauri::command]
pub async fn coding_start(
    state: State<'_, AppState>,
    workspace_id: Uuid,
    provider_id: Uuid,
) -> Result<CodingSessionInfo, AppError> {
    if let Some(existing) = state.coding_sessions.read().await.get(&workspace_id) {
        return Ok(session_info(&*existing.lock().await).await);
    }

    let workspaces = state.workspaces.read().await;
    let handle = workspaces
        .get(&workspace_id)
        .ok_or_else(|| AppError::NotFound(format!("workspace not opened: {workspace_id}")))?;
    let profile = handle.profile.clone();
    let file_ops = handle.file_ops.clone();
    drop(workspaces);

    let target = match profile.kind {
        WorkspaceKind::Local => CodingTarget::Local,
        WorkspaceKind::Remote => CodingTarget::Remote {
            connection_id: profile
                .connection_id
                .ok_or_else(|| AppError::Internal("remote workspace missing connection_id".into()))?,
            host_label: profile.display_name.clone(),
        },
    };

    let mut session = CodingSession::new(workspace_id, profile.root_path.clone(), target, provider_id, file_ops);
    // 只在会话开始时探测一次是否在 Git 仓库里——探测本身要跑一条命令，不值得每次
    // 查询会话状态都重新问一遍；工作区在会话生命周期内也不会突然从"不是仓库"
    // 变成"是仓库"（真发生了，用户重开一次工作区/编程会话就能重新探测到）。
    session.git_repo = crate::coding::git_ops::is_git_repo(&session.target, &session.workspace_root, &state.ssh_pool).await;
    let info = session_info(&session).await;
    state
        .coding_sessions
        .write()
        .await
        .insert(workspace_id, Arc::new(Mutex::new(session)));
    Ok(info)
}

#[tauri::command]
pub async fn coding_set_mode(state: State<'_, AppState>, workspace_id: Uuid, mode: CodingMode) -> Result<(), AppError> {
    let session = get_session(&state, workspace_id).await?;
    session.lock().await.mode = mode;
    Ok(())
}

#[tauri::command]
pub async fn coding_set_auto_allow_readonly(
    state: State<'_, AppState>,
    workspace_id: Uuid,
    enabled: bool,
) -> Result<(), AppError> {
    let session = get_session(&state, workspace_id).await?;
    session.lock().await.auto_allow_readonly = enabled;
    Ok(())
}

/// 开关"每次 Accept 一条变更就自动 git add + commit"（DESIGN.md §3.8.2）；
/// `session.git_repo` 为 false（工作区根目录不是 Git 仓库）时前端应该把这个开关
/// disable 掉，这里不重复做校验——就算开了也不会有任何效果（`accept_change` 里
/// `auto_git_commit && git_repo` 才会真的去跑 git 命令）。
#[tauri::command]
pub async fn coding_set_auto_git_commit(
    state: State<'_, AppState>,
    workspace_id: Uuid,
    enabled: bool,
) -> Result<(), AppError> {
    let session = get_session(&state, workspace_id).await?;
    session.lock().await.auto_git_commit = enabled;
    Ok(())
}

#[tauri::command]
pub async fn coding_send_message(
    state: State<'_, AppState>,
    app_handle: AppHandle,
    workspace_id: Uuid,
    text: String,
) -> Result<String, AppError> {
    let session = get_session(&state, workspace_id).await?;
    let mut session = session.lock().await;
    session
        .send_message(
            &text,
            &state.ai_provider_manager,
            &state.ssh_pool,
            &state.audit_log,
            &state.command_confirms,
            &app_handle,
        )
        .await
}

#[tauri::command]
pub async fn coding_accept_change(
    state: State<'_, AppState>,
    app_handle: AppHandle,
    workspace_id: Uuid,
    change_id: Uuid,
) -> Result<(), AppError> {
    let session = get_session(&state, workspace_id).await?;
    let mut guard = session.lock().await;
    guard.accept_change(change_id, &state.ssh_pool, &app_handle).await
}

#[tauri::command]
pub async fn coding_reject_change(state: State<'_, AppState>, workspace_id: Uuid, change_id: Uuid) -> Result<(), AppError> {
    let session = get_session(&state, workspace_id).await?;
    let mut guard = session.lock().await;
    guard.reject_change(change_id)
}

#[tauri::command]
pub async fn coding_undo_change(state: State<'_, AppState>, workspace_id: Uuid, change_id: Uuid) -> Result<(), AppError> {
    let session = get_session(&state, workspace_id).await?;
    let mut guard = session.lock().await;
    guard.undo_change(change_id).await
}

#[tauri::command]
pub async fn coding_redo_change(state: State<'_, AppState>, workspace_id: Uuid) -> Result<Option<Uuid>, AppError> {
    let session = get_session(&state, workspace_id).await?;
    let mut guard = session.lock().await;
    guard.redo_change().await
}

/// 响应 `coding:command-confirm-request`（DESIGN.md §3.8.2.1）。
#[tauri::command]
pub async fn coding_confirm_command(state: State<'_, AppState>, request_id: Uuid, allow: bool) -> Result<(), AppError> {
    state.command_confirms.resolve(request_id, allow).await;
    Ok(())
}

async fn get_session(state: &State<'_, AppState>, workspace_id: Uuid) -> Result<Arc<Mutex<CodingSession>>, AppError> {
    state
        .coding_sessions
        .read()
        .await
        .get(&workspace_id)
        .cloned()
        .ok_or_else(|| AppError::NotFound(format!("no coding session for workspace {workspace_id}, call coding_start first")))
}
