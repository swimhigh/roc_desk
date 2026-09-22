use tauri::State;

use crate::state::AppState;

/// 冷启动时 Windows"打开方式"/双击已关联文件带来的文件路径（`lib.rs::run` 里从
/// `std::env::args()` 取出存进 `AppState.pending_open_paths`）——前端 App.tsx
/// 挂载后调一次取走并清空，避免重复触发。取了就清是因为这只是"启动时带的参数"，
/// 不是持续订阅的状态，前端处理过一次之后这份数据就没有意义了。
///
/// 其余本地文件系统命令（列目录/读写/复制/移动/二进制预览……）已经搬进
/// `roc_desk_explorer::cmd`（`roc_desk-explorer` 仓库），这个命令因为依赖
/// 宿主专属的 `AppState.pending_open_paths` 留在宿主，见
/// `docs/MULTI_REPO_SPLIT_PROGRESS.md`。
#[tauri::command]
pub fn take_pending_open_paths(state: State<'_, AppState>) -> Vec<String> {
    std::mem::take(&mut *state.pending_open_paths.lock().unwrap())
}
