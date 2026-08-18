use tauri::AppHandle;
use tauri::State;
use uuid::Uuid;

use crate::browser::{self, normalize_url, PanelBounds};
use crate::db::repo::browser_history_repo::BrowserHistoryEntry;
use crate::error::AppError;
use crate::state::AppState;

/// 打开一个网页：内嵌到 `bounds` 描述的面板区域（见 `browser` 模块顶部说明，
/// 2026-08-18 起改为内嵌，不再弹独立窗口）并记入历史。返回规整后的 URL，前端用它
/// 更新地址栏显示（用户输入的可能是裸域名或搜索词，不是最终真正打开的地址）。
#[tauri::command]
pub async fn browser_open(
    state: State<'_, AppState>,
    app_handle: AppHandle,
    url: String,
    bounds: PanelBounds,
) -> Result<String, AppError> {
    let normalized = normalize_url(&url)?;
    browser::open_or_navigate(&app_handle, &normalized, bounds)?;
    state.browser_history.add(&normalized, Some(&normalized))?;
    Ok(normalized)
}

/// 面板尺寸变化时调用（窗口缩放、侧边栏拖拽调宽），只重新定位，不产生历史记录。
#[tauri::command]
pub async fn browser_set_bounds(app_handle: AppHandle, bounds: PanelBounds) -> Result<(), AppError> {
    browser::set_bounds(&app_handle, bounds)
}

/// 切走"网页浏览"Tab 时调用。
#[tauri::command]
pub async fn browser_hide(app_handle: AppHandle) -> Result<(), AppError> {
    browser::hide(&app_handle)
}

/// 切回"网页浏览"Tab 且此前已打开过网页时调用。
#[tauri::command]
pub async fn browser_show(app_handle: AppHandle, bounds: PanelBounds) -> Result<(), AppError> {
    browser::show(&app_handle, bounds)
}

/// 关闭当前网页（返回历史列表视图）或工作区切换/退出时调用。
#[tauri::command]
pub async fn browser_close(app_handle: AppHandle) -> Result<(), AppError> {
    browser::close(&app_handle)
}

#[tauri::command]
pub async fn browser_history_list(state: State<'_, AppState>, limit: Option<usize>) -> Result<Vec<BrowserHistoryEntry>, AppError> {
    state.browser_history.list_recent(limit.unwrap_or(200))
}

#[tauri::command]
pub async fn browser_history_remove(state: State<'_, AppState>, id: Uuid) -> Result<(), AppError> {
    state.browser_history.remove(id)
}

#[tauri::command]
pub async fn browser_history_clear(state: State<'_, AppState>) -> Result<(), AppError> {
    state.browser_history.clear()
}
