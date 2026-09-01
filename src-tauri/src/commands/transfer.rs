use tauri::State;
use uuid::Uuid;

use crate::db::repo::transfer_log_repo::TransferLogEntry;
use crate::error::AppError;
use crate::state::AppState;

/// SFTP/Agent 双栏浏览器"停止传输"（用户反馈：拖拽/上传下载大文件夹时没有办法
/// 主动停下来，只能干等传完）——和 `fs_search_cancel` 是同一套模式，只是这里可能
/// 同时有多个传输各自跑在不同的 `request_id` 下，用集合记"被取消的" id，不是单个
/// "当前活跃的" id。递归复制每处理完一个文件都会检查一次自己的 `request_id` 是否
/// 在这个集合里，在就尽快中止并把这个 id 从集合里清掉（见 `commands::sftp`/
/// `commands::agent` 里几个 `*_entry` 命令收尾时的清理）。
#[tauri::command]
pub fn transfer_cancel(state: State<'_, AppState>, request_id: Uuid) {
    state.cancelled_transfers.lock().unwrap().insert(request_id);
}

/// 传输历史查询（用户 2026-09-01 需求："传输日志需要记录，并可在界面上查询追溯"）。
/// `search` 简单匹配本地/远程路径和连接名称。
#[tauri::command]
pub fn transfer_log_list(
    state: State<'_, AppState>,
    limit: u32,
    offset: u32,
    search: Option<String>,
) -> Result<Vec<TransferLogEntry>, AppError> {
    state.transfer_log.list(limit, offset, search.as_deref())
}

#[tauri::command]
pub fn transfer_log_clear(state: State<'_, AppState>) -> Result<(), AppError> {
    state.transfer_log.clear()
}
