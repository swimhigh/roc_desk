use crate::error::AppError;
use crate::fsops::local::LocalFileOps;
use crate::fsops::{FileEntry, FileOps};

/// SFTP 双栏浏览器的本地一侧（DESIGN.md §3.3）：和 Explorer 的 `fs_*` 命令不同，
/// 这里不需要工作区边界校验——用户本来就是要在本地任意目录之间挑文件上传，
/// 就像 `sftp_*` 命令对远程侧同样不做边界限制一样（两者对称）。不依赖任何
/// `WorkspaceHandle`，直接构造一个 `LocalFileOps` 用。
#[tauri::command]
pub async fn local_list_dir(path: String) -> Result<Vec<FileEntry>, AppError> {
    LocalFileOps.list_dir(&path).await
}

#[tauri::command]
pub fn local_home_dir() -> Result<String, AppError> {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .map(|p| p.replace('\\', "/"))
        .map_err(|_| AppError::Internal("无法定位用户主目录".into()))
}

/// 从 Windows 资源管理器等外部窗口拖真实文件进 SFTP/Agent 双栏浏览器时，Tauri
/// 的 `onDragDropEvent` 只给路径字符串，不带是文件还是目录——但 `*_upload_entry`
/// 系列命令的 `is_dir` 参数是真正决定走"整目录递归上传"还是"单文件上传"分支的，
/// 不能瞎猜，前端拿到路径后必须先问一次。
#[tauri::command]
pub async fn local_is_dir(path: String) -> Result<bool, AppError> {
    tokio::fs::metadata(&path)
        .await
        .map(|m| m.is_dir())
        .map_err(|e| AppError::Internal(format!("无法读取 {path}：{e}")))
}
