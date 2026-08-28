/// 前端把未捕获的异常/Promise rejection 上报到后端日志文件（2026-08-28 用户反馈
/// "程序有可能会自动退出不见了"——渲染进程一次未捕获的异常会让 UI 卡在白屏，从用户
/// 视角和"程序不见了"没什么区别，但之前完全没有记录，连排查方向都没有）。
/// `main.tsx` 的 `window.onerror`/`unhandledrejection` 和 `ErrorBoundary` 组件是
/// 这个命令的调用方，见那两处的注释。
#[tauri::command]
pub fn log_frontend_error(message: String, stack: Option<String>, source: Option<String>) {
    tracing::error!(target: "frontend", %message, stack = stack.as_deref().unwrap_or(""), source = source.as_deref().unwrap_or(""), "前端未捕获异常");
}
