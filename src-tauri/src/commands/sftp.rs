use tauri::{AppHandle, State};
use uuid::Uuid;

use crate::error::AppError;
use crate::fsops::{FileContent, FileEntry, FileOps, WriteOutcome};
use crate::state::AppState;

/// SFTP 自由浏览快捷工具（DESIGN.md §3.3）：**故意不做工作区边界检查**——
/// 这就是它和 Explorer 用的 `fs_*` 命令（见 commands/fs.rs）的核心区别，
/// 用户主动打开这个工具就是为了看工作区之外的路径，不应该被拦或弹确认。

#[tauri::command]
pub async fn sftp_list_dir(
    state: State<'_, AppState>,
    profile_id: Uuid,
    path: String,
) -> Result<Vec<FileEntry>, AppError> {
    let ops = state.ssh_pool.get_file_ops(profile_id).await?;
    ops.list_dir(&path).await
}

#[tauri::command]
pub async fn sftp_read_file(
    state: State<'_, AppState>,
    profile_id: Uuid,
    path: String,
) -> Result<FileContent, AppError> {
    let ops = state.ssh_pool.get_file_ops(profile_id).await?;
    ops.read_file(&path).await
}

#[tauri::command]
pub async fn sftp_write_file(
    state: State<'_, AppState>,
    profile_id: Uuid,
    path: String,
    content: String,
    expected_mtime: Option<i64>,
) -> Result<WriteOutcome, AppError> {
    let ops = state.ssh_pool.get_file_ops(profile_id).await?;
    ops.write_file(&path, &content, expected_mtime).await
}

#[tauri::command]
pub async fn sftp_download(
    state: State<'_, AppState>,
    profile_id: Uuid,
    remote_path: String,
    local_path: String,
) -> Result<(), AppError> {
    let ops = state.ssh_pool.get_file_ops(profile_id).await?;
    ops.download_to_local(&remote_path, &local_path).await
}

#[tauri::command]
pub async fn sftp_upload(
    state: State<'_, AppState>,
    profile_id: Uuid,
    local_path: String,
    remote_path: String,
) -> Result<(), AppError> {
    let ops = state.ssh_pool.get_file_ops(profile_id).await?;
    ops.upload_from_local(&local_path, &remote_path).await
}

/// 双栏 SFTP 浏览器的"下载到本地目录"（DESIGN.md §3.3）：目标文件/目录名沿用远程
/// 原名，落在 `local_dir` 下面——文件走单文件传输，目录走递归遍历。`request_id`
/// 由前端生成，只有目录传输才会真的收到 `sftp:transfer-progress` 事件
/// （单文件传输通常够快，逐文件的粗粒度进度反而没意义，见 `emit_progress` 用法）。
#[tauri::command]
pub async fn sftp_download_entry(
    state: State<'_, AppState>,
    app_handle: AppHandle,
    profile_id: Uuid,
    remote_path: String,
    is_dir: bool,
    local_dir: String,
    request_id: Uuid,
) -> Result<(), AppError> {
    let ops = state.ssh_pool.get_file_ops(profile_id).await?;
    let name = remote_path.trim_end_matches('/').rsplit('/').next().unwrap_or(&remote_path);
    let local_target = format!("{}/{}", local_dir.trim_end_matches('/'), name);
    if is_dir {
        ops.download_recursive(&remote_path, &local_target, Some((app_handle, request_id))).await
    } else {
        ops.download_to_local(&remote_path, &local_target).await
    }
}

/// 和 `sftp_download_entry` 对称的"上传到远程目录"。
#[tauri::command]
pub async fn sftp_upload_entry(
    state: State<'_, AppState>,
    app_handle: AppHandle,
    profile_id: Uuid,
    local_path: String,
    is_dir: bool,
    remote_dir: String,
    request_id: Uuid,
) -> Result<(), AppError> {
    let ops = state.ssh_pool.get_file_ops(profile_id).await?;
    let name = local_path.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().unwrap_or(&local_path);
    let remote_target = format!("{}/{}", remote_dir.trim_end_matches('/'), name);
    if is_dir {
        ops.upload_recursive(&local_path, &remote_target, Some((app_handle, request_id))).await
    } else {
        ops.upload_from_local(&local_path, &remote_target).await
    }
}

/// 删除需要前端二次确认（UI 层，不是这里）——DESIGN.md §3.3 明确要求
/// "需前端二次确认"，但那是交互设计，不是后端应该拦的权限检查。
#[tauri::command]
pub async fn sftp_delete(state: State<'_, AppState>, profile_id: Uuid, path: String, is_dir: bool) -> Result<(), AppError> {
    let ops = state.ssh_pool.get_file_ops(profile_id).await?;
    ops.delete(&path, is_dir).await
}

#[tauri::command]
pub async fn sftp_rename(state: State<'_, AppState>, profile_id: Uuid, from: String, to: String) -> Result<(), AppError> {
    let ops = state.ssh_pool.get_file_ops(profile_id).await?;
    ops.rename(&from, &to).await
}
