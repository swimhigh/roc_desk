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

/// 资源管理器模块的盘符列表（Total Commander 式，Windows 下浏览 D:\、E:\ 等必须
/// 能切换盘符，只给一个起始目录走不出那一个盘）。没有现成的"列出所有盘符" API 值得
/// 为这一个小功能引入 Win32 依赖——`A`..`Z` 逐个探测 `X:\` 是否存在就是标准做法
/// （资源管理器本身内部也是类似逻辑）。非 Windows 平台没有盘符概念，返回根目录。
#[tauri::command]
pub fn local_list_drives() -> Vec<String> {
    #[cfg(windows)]
    {
        (b'A'..=b'Z')
            .filter_map(|b| {
                let letter = b as char;
                let root = format!("{letter}:\\");
                std::path::Path::new(&root).exists().then(|| format!("{letter}:/"))
            })
            .collect()
    }
    #[cfg(not(windows))]
    {
        vec!["/".to_string()]
    }
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

/// 资源管理器模块（Total Commander 式本地双栏文件管理，`docs/HOME_MODES_DESIGN.md`
/// §3.2/§6 Phase 4）的文件操作——和上面几个一样不做工作区边界校验，用户就是要在
/// 任意本地目录之间复制/移动/删除，不像 Explorer 的 `fs_*` 命令那样绑定某个工作区。

#[tauri::command]
pub async fn local_delete(path: String, is_dir: bool) -> Result<(), AppError> {
    LocalFileOps.delete(&path, is_dir).await
}

#[tauri::command]
pub async fn local_rename(from: String, to: String) -> Result<(), AppError> {
    LocalFileOps.rename(&from, &to).await
}

#[tauri::command]
pub async fn local_copy(from: String, to: String, is_dir: bool) -> Result<(), AppError> {
    LocalFileOps.copy(&from, &to, is_dir).await
}

#[tauri::command]
pub async fn local_create_dir(path: String) -> Result<(), AppError> {
    LocalFileOps.create_dir(&path).await
}

/// "移动"（双栏之间的 F6/剪切粘贴）：同一个磁盘卷内先尝试 `rename`，是原子的重命名，
/// 不需要真的搬运字节；`rename` 在 Windows 上跨盘符会直接报错（这是 `std::fs::rename`
/// 的固有限制，不是本项目实现的疏漏），这时退化成"整份复制到目的地、复制成功后
/// 删掉源"——和资源管理器/Total Commander 跨盘移动文件时的实际行为一致。
#[tauri::command]
pub async fn local_move(from: String, to: String, is_dir: bool) -> Result<(), AppError> {
    if LocalFileOps.rename(&from, &to).await.is_ok() {
        return Ok(());
    }
    LocalFileOps.copy(&from, &to, is_dir).await?;
    LocalFileOps.delete(&from, is_dir).await
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
