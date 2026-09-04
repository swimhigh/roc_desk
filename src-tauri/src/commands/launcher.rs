use serde::Serialize;
use tauri::State;

use crate::state::AppState;

/// 这个进程冷启动时命令行参数决定的角色（`docs/HOME_MODES_DESIGN.md` §3.5）——
/// 不带 `--mode` 就是首页/启动器进程；带了就是某个工作模块的模块窗口，`open`
/// 是可选的"直接打开这一项"参数（工作区 id / 连接 id / 文件路径）。这个值在
/// 进程生命周期内不会变，不需要像 `take_pending_open_paths` 那样"取走清空"。
#[derive(Debug, Clone, Serialize)]
pub struct LaunchContext {
    pub mode: Option<String>,
    pub open: Option<String>,
}

#[tauri::command]
pub fn get_launch_context(state: State<'_, AppState>) -> LaunchContext {
    LaunchContext { mode: state.launch_mode.clone(), open: state.launch_open.clone() }
}

/// 首页"点开一个模块"和模块窗口"返回首页"共用的入口——不带 `mode` 就是拉起（或
/// 聚焦已有的）首页，带 `mode` 就是开一个新的模块窗口。
///
/// 模块窗口进程不注册 `single-instance`（见 `lib.rs::run`），所以这里传了 `mode`
/// 的 spawn 永远会变成一个真正独立的新进程，同一个模块可以同时开多个窗口。
/// 不传 `mode`（"返回首页"）时，新拉起的这个进程仍然会注册 `single-instance`——
/// 如果首页已经在跑，它会在自己启动过程里发现互斥体已被占用，把这次启动转发给
/// 已有首页窗口（聚焦它）后自己退出；如果首页已经被关掉了，它就会正常跑起来成为
/// 新的首页。两种情况都直接复用现有插件本来就实现好的"发现已有实例"路径，不需要
/// 额外写跨进程 IPC 代码。
#[tauri::command]
pub fn spawn_module_window(mode: Option<String>, open: Option<String>) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut cmd = std::process::Command::new(exe);
    if let Some(mode) = mode {
        cmd.arg(format!("--mode={mode}"));
    }
    if let Some(open) = open {
        cmd.arg(format!("--open={open}"));
    }
    cmd.spawn().map_err(|e| e.to_string())?;
    Ok(())
}
