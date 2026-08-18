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
