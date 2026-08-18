use tauri::{AppHandle, State};
use uuid::Uuid;

use crate::error::AppError;
use crate::state::AppState;

/// 建连或复用连接池中的连接（DESIGN.md §3.2.2 多路复用）。返回值就是 `profile_id`
/// 本身——连接池以 profile_id 为 key，不需要额外发一个 session_id。
#[tauri::command]
pub async fn ssh_connect(state: State<'_, AppState>, profile_id: Uuid) -> Result<Uuid, AppError> {
    state.ssh_pool.get_or_connect(profile_id).await?;
    Ok(profile_id)
}

#[tauri::command]
pub async fn ssh_open_shell(
    state: State<'_, AppState>,
    app_handle: AppHandle,
    profile_id: Uuid,
    rows: u16,
    cols: u16,
    cwd: Option<String>,
) -> Result<Uuid, AppError> {
    let session = state
        .ssh_pool
        .get(profile_id)
        .await
        .ok_or_else(|| AppError::NotFound(format!("no active ssh session for {profile_id}")))?;
    session.open_shell(rows, cols, cwd.as_deref(), app_handle).await
}

#[tauri::command]
pub async fn ssh_write(
    state: State<'_, AppState>,
    profile_id: Uuid,
    channel_id: Uuid,
    data: Vec<u8>,
) -> Result<(), AppError> {
    let session = state
        .ssh_pool
        .get(profile_id)
        .await
        .ok_or_else(|| AppError::NotFound(format!("no active ssh session for {profile_id}")))?;
    session.write(channel_id, data).await
}

#[tauri::command]
pub async fn ssh_resize(
    state: State<'_, AppState>,
    profile_id: Uuid,
    channel_id: Uuid,
    rows: u16,
    cols: u16,
) -> Result<(), AppError> {
    let session = state
        .ssh_pool
        .get(profile_id)
        .await
        .ok_or_else(|| AppError::NotFound(format!("no active ssh session for {profile_id}")))?;
    session.resize(channel_id, rows, cols).await
}

#[tauri::command]
pub async fn ssh_disconnect(state: State<'_, AppState>, profile_id: Uuid) -> Result<(), AppError> {
    state.ssh_pool.disconnect(profile_id).await
}

/// 关闭多终端场景下的某一个终端 Tab（不影响同一连接上的其它 Channel）。
#[tauri::command]
pub async fn ssh_close_channel(
    state: State<'_, AppState>,
    profile_id: Uuid,
    channel_id: Uuid,
) -> Result<(), AppError> {
    let session = state
        .ssh_pool
        .get(profile_id)
        .await
        .ok_or_else(|| AppError::NotFound(format!("no active ssh session for {profile_id}")))?;
    session.close_channel(channel_id).await
}

/// 响应 TOFU / 指纹变化弹窗（DESIGN.md §3.2.1）。
#[tauri::command]
pub async fn ssh_confirm_host_key(state: State<'_, AppState>, request_id: Uuid, trust: bool) -> Result<(), AppError> {
    state.trust_prompts.resolve(request_id, trust).await;
    Ok(())
}
