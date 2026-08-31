use tauri::{AppHandle, State};
use uuid::Uuid;

use crate::error::AppError;
use crate::fsops::agent::AgentFileOps;
use crate::fsops::local::LocalFileOps;
use crate::fsops::{copy_between, FileContent, FileEntry, FileOps, WriteOutcome};
use crate::state::AppState;

async fn file_ops(state: &AppState, profile_id: Uuid) -> Result<AgentFileOps, AppError> {
    let session = state.agent_pool.get_or_connect(profile_id).await?;
    Ok(AgentFileOps::new(session))
}

/// 建连或复用连接池中的连接，和 `ssh_connect` 是同一种模式（AGENT_DESIGN.md §四.2）。
#[tauri::command]
pub async fn agent_connect(state: State<'_, AppState>, profile_id: Uuid) -> Result<Uuid, AppError> {
    state.agent_pool.get_or_connect(profile_id).await?;
    Ok(profile_id)
}

#[tauri::command]
pub async fn agent_disconnect(state: State<'_, AppState>, profile_id: Uuid) -> Result<(), AppError> {
    state.agent_pool.disconnect(profile_id).await
}

/// 响应 TLS 证书指纹 TOFU / 指纹变化弹窗（AGENT_DESIGN.md §3.1），
/// 和 `ssh_confirm_host_key` 是同一种模式。
#[tauri::command]
pub async fn agent_confirm_cert(state: State<'_, AppState>, request_id: Uuid, trust: bool) -> Result<(), AppError> {
    state.agent_trust_prompts.resolve(request_id, trust).await;
    Ok(())
}

/// 连接设置对话框"测试连接"按钮：直接用表单里当前填的 host/port/配对令牌验证一次
/// 完整的 TCP + TLS 握手 + 令牌校验，不经过 `ConnectionProfile`（表单值可能还没
/// 保存）、不做证书指纹 TOFU 持久化——纯粹的"这几个字段填得对不对"验证，返回一句
/// 人类可读的结果文本。
#[tauri::command]
pub async fn agent_test_connection(host: String, port: u16, token: String) -> Result<String, AppError> {
    let result = crate::agent::AgentSession::test_connect(&host, port, token).await?;
    Ok(format!(
        "连接成功：主机 {}，Agent 版本 {}，证书指纹 {}",
        result.hostname, result.server_version, result.fingerprint
    ))
}

/// 工作区挂载向导用来浏览 Agent 目标机器的目录（和 `sftp_list_dir` 是同一种用途，
/// 分开一个命令是因为协议完全不同——SFTP 走 `RemoteFileOps`，这里走
/// `AgentFileOps`，都实现同一个 `FileOps` trait，直接复用其 `list_dir`）。
#[tauri::command]
pub async fn agent_list_dir(state: State<'_, AppState>, profile_id: Uuid, path: String) -> Result<Vec<FileEntry>, AppError> {
    file_ops(&state, profile_id).await?.list_dir(&path).await
}

/// Windows 盘符列表（Explorer 树的"根"概念）——工作区挂载向导浏览 Agent 目标机器时
/// 用它渲染"此电脑"下的盘符，而不是像 SSH 那样从 `/` 开始（AGENT_DESIGN.md §3.3）。
#[tauri::command]
pub async fn agent_list_roots(state: State<'_, AppState>, profile_id: Uuid) -> Result<Vec<String>, AppError> {
    let session = state.agent_pool.get_or_connect(profile_id).await?;
    match session.request(roc_desk_protocol::Request::ListRoots).await? {
        roc_desk_protocol::Response::Ok(roc_desk_protocol::ResponseBody::Roots(roots)) => Ok(roots),
        roc_desk_protocol::Response::Error { message, .. } => Err(AppError::Internal(message)),
        _ => Err(AppError::Internal("Agent 返回了意外的响应类型".into())),
    }
}

/// 交互式终端（AGENT_DESIGN.md §四.4 Phase 2），命令签名和 `commands::ssh` 里的
/// 四个终端命令一一对应，前端复用同一个 `TerminalView` 组件。
#[tauri::command]
pub async fn agent_open_shell(
    state: State<'_, AppState>,
    app_handle: AppHandle,
    profile_id: Uuid,
    rows: u16,
    cols: u16,
    cwd: Option<String>,
) -> Result<Uuid, AppError> {
    let session = state.agent_pool.get_or_connect(profile_id).await?;
    session.open_shell(rows, cols, cwd.as_deref().unwrap_or(""), app_handle).await
}

#[tauri::command]
pub async fn agent_write(state: State<'_, AppState>, profile_id: Uuid, channel_id: Uuid, data: Vec<u8>) -> Result<(), AppError> {
    let session = state
        .agent_pool
        .get(profile_id)
        .await
        .ok_or_else(|| AppError::NotFound(format!("no active agent session for {profile_id}")))?;
    session.write_shell(channel_id, data).await
}

#[tauri::command]
pub async fn agent_resize(state: State<'_, AppState>, profile_id: Uuid, channel_id: Uuid, rows: u16, cols: u16) -> Result<(), AppError> {
    let session = state
        .agent_pool
        .get(profile_id)
        .await
        .ok_or_else(|| AppError::NotFound(format!("no active agent session for {profile_id}")))?;
    session.resize_shell(channel_id, cols, rows).await
}

#[tauri::command]
pub async fn agent_close_channel(state: State<'_, AppState>, profile_id: Uuid, channel_id: Uuid) -> Result<(), AppError> {
    let session = state
        .agent_pool
        .get(profile_id)
        .await
        .ok_or_else(|| AppError::NotFound(format!("no active agent session for {profile_id}")))?;
    session.close_shell(channel_id).await
}

// --- 自由浏览快捷工具（AGENT_DESIGN.md §四.3，和 commands/sftp.rs 是同一种用途，
// 分开一套命令是因为底层协议完全不同：SFTP 走 RemoteFileOps，这里走
// AgentFileOps，故意不做工作区边界检查——用户主动打开这个工具就是为了看
// 工作区之外的路径。) ---

#[tauri::command]
pub async fn agent_read_file(state: State<'_, AppState>, profile_id: Uuid, path: String) -> Result<FileContent, AppError> {
    file_ops(&state, profile_id).await?.read_file_for_editor(&path).await
}

#[tauri::command]
pub async fn agent_write_file(
    state: State<'_, AppState>,
    profile_id: Uuid,
    path: String,
    content: String,
    expected_mtime: Option<i64>,
) -> Result<WriteOutcome, AppError> {
    file_ops(&state, profile_id).await?.write_file(&path, &content, expected_mtime).await
}

#[tauri::command]
pub async fn agent_delete(state: State<'_, AppState>, profile_id: Uuid, path: String, is_dir: bool) -> Result<(), AppError> {
    file_ops(&state, profile_id).await?.delete(&path, is_dir).await
}

#[tauri::command]
pub async fn agent_rename(state: State<'_, AppState>, profile_id: Uuid, from: String, to: String) -> Result<(), AppError> {
    file_ops(&state, profile_id).await?.rename(&from, &to).await
}

#[tauri::command]
pub async fn agent_download(state: State<'_, AppState>, profile_id: Uuid, remote_path: String, local_path: String) -> Result<(), AppError> {
    let ops = file_ops(&state, profile_id).await?;
    copy_between(&ops, &remote_path, &LocalFileOps, &local_path, false, &None).await
}

#[tauri::command]
pub async fn agent_upload(state: State<'_, AppState>, profile_id: Uuid, local_path: String, remote_path: String) -> Result<(), AppError> {
    let ops = file_ops(&state, profile_id).await?;
    copy_between(&LocalFileOps, &local_path, &ops, &remote_path, false, &None).await
}

/// 双栏浏览器的"下载到本地目录"：目标文件/目录名沿用远程原名，落在 `local_dir`
/// 下面，和 `sftp_download_entry` 是同一种用途（见那边的文档注释）。
#[tauri::command]
pub async fn agent_download_entry(
    state: State<'_, AppState>,
    app_handle: AppHandle,
    profile_id: Uuid,
    remote_path: String,
    is_dir: bool,
    local_dir: String,
    request_id: Uuid,
) -> Result<(), AppError> {
    let ops = file_ops(&state, profile_id).await?;
    let name = remote_path.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().unwrap_or(&remote_path);
    let local_target = format!("{}/{}", local_dir.trim_end_matches(['/', '\\']), name);
    copy_between(&ops, &remote_path, &LocalFileOps, &local_target, is_dir, &Some((app_handle, request_id))).await
}

/// 和 `agent_download_entry` 对称的"上传到远程目录"。
#[tauri::command]
pub async fn agent_upload_entry(
    state: State<'_, AppState>,
    app_handle: AppHandle,
    profile_id: Uuid,
    local_path: String,
    is_dir: bool,
    remote_dir: String,
    request_id: Uuid,
) -> Result<(), AppError> {
    let ops = file_ops(&state, profile_id).await?;
    let name = local_path.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().unwrap_or(&local_path);
    let remote_target = format!("{}/{}", remote_dir.trim_end_matches('/'), name);
    copy_between(&LocalFileOps, &local_path, &ops, &remote_target, is_dir, &Some((app_handle, request_id))).await
}
