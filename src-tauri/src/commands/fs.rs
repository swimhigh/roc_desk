use std::path::{Path, PathBuf};

use tauri::State;
use uuid::Uuid;

use crate::error::AppError;
use crate::fsops::encoding;
use crate::fsops::{FileContent, FileEntry, ReplaceSummary, SearchOptions, SearchSummary, WriteOutcome};
use crate::state::AppState;
use crate::workspace::{WorkspaceHandle, WorkspaceKind};

/// Explorer/编辑器用的文件读写命令（CODE_DESIGN.md §4.0），复用 fsops。
///
/// 每个命令都先做工作区归属校验：请求路径必须落在该工作区根目录之内，
/// 防止前端传入的路径（无论是 bug 还是恶意 payload）逃出工作区边界读写任意文件。

async fn get_handle(state: &State<'_, AppState>, workspace_id: Uuid) -> Result<WorkspaceHandle, AppError> {
    state
        .workspaces
        .read()
        .await
        .get(&workspace_id)
        .cloned()
        .ok_or_else(|| AppError::NotFound(format!("工作区未打开: {workspace_id}")))
}

/// 本地工作区场景下校验 `path` 没有逃出 `root_path`；远程工作区（尚未实现 SFTP 时）暂不做此检查，
/// 交由未来的 RemoteFileOps 在自己的实现里处理（远程路径的"越界"语义和本地文件系统不同）。
fn guard_local_path(handle: &WorkspaceHandle, path: &str) -> Result<(), AppError> {
    if handle.profile.kind != WorkspaceKind::Local {
        return Ok(());
    }
    let root = Path::new(&handle.profile.root_path);
    let root_canon = root.canonicalize().map_err(AppError::from)?;

    let candidate = PathBuf::from(path);
    // 新建文件时 candidate 本身还不存在，canonicalize 会失败：退化为校验其父目录，
    // 而不是直接放行未校验的原始路径（否则 "创建新文件" 场景就绕过了边界检查）。
    let candidate_canon = match candidate.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            let parent = candidate
                .parent()
                .ok_or_else(|| AppError::PermissionDenied(format!("非法路径: {path}")))?;
            let parent_canon = parent.canonicalize().map_err(|_| {
                AppError::PermissionDenied(format!("路径 {path} 不在工作区范围内"))
            })?;
            parent_canon.join(candidate.file_name().unwrap_or_default())
        }
    };

    if !candidate_canon.starts_with(&root_canon) {
        return Err(AppError::PermissionDenied(format!(
            "路径 {path} 不在工作区 {} 范围内",
            handle.profile.root_path
        )));
    }
    Ok(())
}

#[tauri::command]
pub async fn fs_list_dir(
    state: State<'_, AppState>,
    workspace_id: Uuid,
    path: String,
) -> Result<Vec<FileEntry>, AppError> {
    let handle = get_handle(&state, workspace_id).await?;
    guard_local_path(&handle, &path)?;
    handle.file_ops.list_dir(&path).await
}

#[tauri::command]
pub async fn fs_read_file(
    state: State<'_, AppState>,
    workspace_id: Uuid,
    path: String,
) -> Result<FileContent, AppError> {
    let handle = get_handle(&state, workspace_id).await?;
    guard_local_path(&handle, &path)?;
    handle.file_ops.read_file(&path).await
}

#[tauri::command]
pub async fn fs_write_file(
    state: State<'_, AppState>,
    workspace_id: Uuid,
    path: String,
    content: String,
    expected_mtime: Option<i64>,
) -> Result<WriteOutcome, AppError> {
    let handle = get_handle(&state, workspace_id).await?;
    guard_local_path(&handle, &path)?;
    handle.file_ops.write_file(&path, &content, expected_mtime).await
}

/// "Reopen with Encoding"（参考 VS Code）：忽略自动探测，强制按指定编码重新解码。
#[tauri::command]
pub async fn fs_read_file_with_encoding(
    state: State<'_, AppState>,
    workspace_id: Uuid,
    path: String,
    encoding_label: String,
) -> Result<FileContent, AppError> {
    let handle = get_handle(&state, workspace_id).await?;
    guard_local_path(&handle, &path)?;
    let (bytes, mtime) = handle.file_ops.read_file_raw(&path).await?;
    let text = encoding::decode_with(&bytes, &encoding_label).map_err(AppError::Internal)?;
    Ok(FileContent { text, encoding: encoding_label, mtime })
}

/// "Save with Encoding"（参考 VS Code）：按指定编码编码后写盘，而不是固定 UTF-8。
#[tauri::command]
pub async fn fs_write_file_with_encoding(
    state: State<'_, AppState>,
    workspace_id: Uuid,
    path: String,
    content: String,
    encoding_label: String,
    expected_mtime: Option<i64>,
) -> Result<WriteOutcome, AppError> {
    let handle = get_handle(&state, workspace_id).await?;
    guard_local_path(&handle, &path)?;
    let bytes = encoding::encode_with(&content, &encoding_label).map_err(AppError::Internal)?;
    handle.file_ops.write_file_bytes(&path, &bytes, expected_mtime).await
}

#[tauri::command]
pub fn fs_supported_encodings() -> Vec<&'static str> {
    encoding::SUPPORTED_ENCODINGS.to_vec()
}

/// Explorer 右键"删除"（参考 VS Code）。
#[tauri::command]
pub async fn fs_delete(state: State<'_, AppState>, workspace_id: Uuid, path: String, is_dir: bool) -> Result<(), AppError> {
    let handle = get_handle(&state, workspace_id).await?;
    guard_local_path(&handle, &path)?;
    handle.file_ops.delete(&path, is_dir).await
}

/// Explorer 右键"重命名"（参考 VS Code）；也用于"剪切+粘贴"的移动——剪切在前端只是
/// 记一下 clipboard 状态，真正的移动动作就是对目标父目录调一次 rename。
#[tauri::command]
pub async fn fs_rename(state: State<'_, AppState>, workspace_id: Uuid, from: String, to: String) -> Result<(), AppError> {
    let handle = get_handle(&state, workspace_id).await?;
    guard_local_path(&handle, &from)?;
    guard_local_path(&handle, &to)?;
    handle.file_ops.rename(&from, &to).await
}

/// Explorer 右键"复制+粘贴"（参考 VS Code）。目前只支持文件，见 `FileOps::copy` 的文档注释。
#[tauri::command]
pub async fn fs_copy(state: State<'_, AppState>, workspace_id: Uuid, from: String, to: String, is_dir: bool) -> Result<(), AppError> {
    let handle = get_handle(&state, workspace_id).await?;
    guard_local_path(&handle, &from)?;
    guard_local_path(&handle, &to)?;
    handle.file_ops.copy(&from, &to, is_dir).await
}

/// 左侧目录树的"搜索"功能（参考 VS Code 全局搜索面板，2026-08-18 需求）：在整个
/// 工作区根目录下全文搜索，见 `fsops::FileOps::search_text` 的取舍说明。
#[tauri::command]
pub async fn fs_search(
    state: State<'_, AppState>,
    workspace_id: Uuid,
    query: String,
    options: SearchOptions,
) -> Result<SearchSummary, AppError> {
    let handle = get_handle(&state, workspace_id).await?;
    handle.file_ops.search_text(&handle.profile.root_path, &query, &options).await
}

/// 查找并替换全部（参考 VS Code 搜索面板的 Replace）。`paths` 是前端已经拿到的
/// 搜索结果里的文件路径——不重新在后端跑一遍搜索，替换范围严格等于用户在结果里
/// 看到的那些文件，不会有"UI 显示的和实际改的不一致"的落差。
#[tauri::command]
pub async fn fs_replace(
    state: State<'_, AppState>,
    workspace_id: Uuid,
    paths: Vec<String>,
    query: String,
    replacement: String,
    options: SearchOptions,
) -> Result<ReplaceSummary, AppError> {
    let handle = get_handle(&state, workspace_id).await?;
    for path in &paths {
        guard_local_path(&handle, path)?;
    }
    handle.file_ops.replace_text(&paths, &query, &replacement, &options).await
}
