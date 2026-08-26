use tauri::{AppHandle, State};
use uuid::Uuid;

use crate::error::AppError;
use crate::rdp::PanelBounds;
use crate::state::AppState;

/// 打开一个 RDP 会话：拉起 wfreerdp.exe（FreeRDP）并把它的窗口内嵌到 `bounds`
/// 描述的屏幕区域（远程工具模式，DESIGN.md §3.9，见 `rdp/mod.rs` 顶部为什么不
/// 自己实现协议的说明）。
#[tauri::command]
pub async fn rdp_connect(
    state: State<'_, AppState>,
    app_handle: AppHandle,
    profile_id: Uuid,
    bounds: PanelBounds,
) -> Result<Uuid, AppError> {
    state.rdp_sessions.connect(&app_handle, profile_id, bounds).await
}

/// 面板尺寸变化时调用（窗口缩放、侧边栏拖拽调宽、标签切换）。
#[tauri::command]
pub async fn rdp_set_bounds(
    state: State<'_, AppState>,
    app_handle: AppHandle,
    session_id: Uuid,
    bounds: PanelBounds,
) -> Result<(), AppError> {
    state.rdp_sessions.set_bounds(&app_handle, session_id, bounds)
}

/// 切走这个标签页时调用——内嵌窗口不受 CSS 影响，必须显式隐藏。
#[tauri::command]
pub async fn rdp_hide(state: State<'_, AppState>, session_id: Uuid) -> Result<(), AppError> {
    state.rdp_sessions.hide(session_id)
}

/// 切回这个标签页时调用。
#[tauri::command]
pub async fn rdp_show(
    state: State<'_, AppState>,
    app_handle: AppHandle,
    session_id: Uuid,
    bounds: PanelBounds,
) -> Result<(), AppError> {
    state.rdp_sessions.show(&app_handle, session_id, bounds)
}

/// 关闭这个 RDP 会话：杀掉内嵌的 wfreerdp.exe 进程。
#[tauri::command]
pub async fn rdp_disconnect(state: State<'_, AppState>, session_id: Uuid) -> Result<(), AppError> {
    state.rdp_sessions.disconnect(session_id)
}

#[tauri::command]
pub async fn rdp_status(state: State<'_, AppState>, session_id: Uuid) -> Result<crate::rdp::RdpStatus, AppError> {
    state.rdp_sessions.status(session_id)
}
