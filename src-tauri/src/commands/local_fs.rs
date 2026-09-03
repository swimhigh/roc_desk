use base64::Engine;
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

use crate::error::AppError;
use crate::fsops::local::LocalFileOps;
use crate::fsops::{
    binary_info, encoding, jar_info, office_convert, BinaryInfo, FileContent, FileEntry, FileOps, JarInfo,
    WriteOutcome, BINARY_PREVIEW_MAX_BYTES, EXECUTABLE_INSPECT_MAX_BYTES,
};
use crate::state::AppState;

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

/// "游离文件"（没有打开工作区/不属于任何工作区，靠拖拽、Ctrl+O、Windows 文件关联
/// 直接打开的单个本地文件，DESIGN.md 独立文件编辑 §）：编辑器读写这类文件时不经过
/// `WorkspaceHandle`，也就没有 `guard_local_path` 的工作区边界校验——这类文件本来
/// 就在任何工作区之外，校验它"是否落在工作区根目录内"没有意义。命令签名和
/// `commands/fs.rs` 里对应的 `fs_*` 版本一一对应，只是去掉 `workspace_id` 参数、
/// 直接用 `LocalFileOps` 代替 `handle.file_ops`。

#[tauri::command]
pub async fn local_read_file(path: String) -> Result<FileContent, AppError> {
    LocalFileOps.read_file_for_editor(&path).await
}

#[tauri::command]
pub async fn local_write_file(path: String, content: String, expected_mtime: Option<i64>) -> Result<WriteOutcome, AppError> {
    LocalFileOps.write_file(&path, &content, expected_mtime).await
}

#[tauri::command]
pub async fn local_read_file_with_encoding(path: String, encoding_label: String) -> Result<FileContent, AppError> {
    let (bytes, mtime, total_size, truncated) = LocalFileOps.read_bytes_for_editor(&path).await?;
    let text = encoding::decode_with(&bytes, &encoding_label).map_err(AppError::Internal)?;
    Ok(FileContent { text, encoding: encoding_label, mtime, total_size, truncated })
}

#[tauri::command]
pub async fn local_write_file_with_encoding(
    path: String,
    content: String,
    encoding_label: String,
    expected_mtime: Option<i64>,
) -> Result<WriteOutcome, AppError> {
    let bytes = encoding::encode_with(&content, &encoding_label).map_err(AppError::Internal)?;
    LocalFileOps.write_file_bytes(&path, &bytes, expected_mtime).await
}

#[tauri::command]
pub async fn local_read_binary_preview(path: String) -> Result<String, AppError> {
    let bytes = LocalFileOps.read_binary_for_preview(&path, BINARY_PREVIEW_MAX_BYTES).await?;
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

#[tauri::command]
pub async fn local_open_externally(app_handle: AppHandle, path: String) -> Result<(), AppError> {
    app_handle
        .opener()
        .open_path(path, None::<&str>)
        .map_err(|e| AppError::Internal(e.to_string()))
}

#[tauri::command]
pub async fn local_convert_legacy_office_to_pdf(path: String) -> Result<String, AppError> {
    let tmp_dir = std::env::temp_dir().join("roc_desk_office_convert");
    let pdf_path = office_convert::convert_to_pdf(std::path::Path::new(&path), &tmp_dir).await?;
    let bytes = tokio::fs::read(&pdf_path).await.map_err(AppError::from)?;
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

#[tauri::command]
pub async fn local_inspect_binary(path: String) -> Result<BinaryInfo, AppError> {
    let bytes = LocalFileOps.read_binary_for_preview(&path, EXECUTABLE_INSPECT_MAX_BYTES).await?;
    binary_info::inspect(&bytes)
}

#[tauri::command]
pub async fn local_peek_is_binary(path: String) -> Result<bool, AppError> {
    let (head, _mtime) = LocalFileOps.read_file_raw_bounded(&path, 64).await?;
    Ok(binary_info::looks_like_binary(&head))
}

#[tauri::command]
pub async fn local_inspect_jar(path: String) -> Result<JarInfo, AppError> {
    let bytes = LocalFileOps.read_binary_for_preview(&path, EXECUTABLE_INSPECT_MAX_BYTES).await?;
    jar_info::inspect(&bytes)
}

/// 冷启动时 Windows"打开方式"/双击已关联文件带来的文件路径（`lib.rs::run` 里从
/// `std::env::args()` 取出存进 `AppState.pending_open_paths`）——前端 App.tsx
/// 挂载后调一次取走并清空，避免重复触发。取了就清是因为这只是"启动时带的参数"，
/// 不是持续订阅的状态，前端处理过一次之后这份数据就没有意义了。
#[tauri::command]
pub fn take_pending_open_paths(state: State<'_, AppState>) -> Vec<String> {
    std::mem::take(&mut *state.pending_open_paths.lock().unwrap())
}
