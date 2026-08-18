use tauri::{AppHandle, State};
use uuid::Uuid;

use crate::error::AppError;
use crate::state::AppState;

/// 本地终端（DESIGN.md §3.2，本地工作区分支）。`cwd` 就是当前工作区根目录——
/// 参考 VS Code 打开项目后集成终端默认进到项目目录，而不是用户主目录。
#[tauri::command]
pub async fn pty_open(
    state: State<'_, AppState>,
    app_handle: AppHandle,
    cwd: String,
    rows: u16,
    cols: u16,
) -> Result<Uuid, AppError> {
    state.local_pty.open(cwd, rows, cols, app_handle).await
}

#[tauri::command]
pub async fn pty_write(state: State<'_, AppState>, channel_id: Uuid, data: Vec<u8>) -> Result<(), AppError> {
    state.local_pty.write(channel_id, data).await
}

#[tauri::command]
pub async fn pty_resize(state: State<'_, AppState>, channel_id: Uuid, rows: u16, cols: u16) -> Result<(), AppError> {
    state.local_pty.resize(channel_id, rows, cols).await
}

#[tauri::command]
pub async fn pty_close(state: State<'_, AppState>, channel_id: Uuid) -> Result<(), AppError> {
    state.local_pty.close(channel_id).await
}
