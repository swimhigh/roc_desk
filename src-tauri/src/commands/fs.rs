use std::path::{Path, PathBuf};

use base64::Engine;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_opener::OpenerExt;
use uuid::Uuid;

use crate::error::AppError;
use crate::fsops::encoding;
use crate::fsops::{
    binary_info, jar_info, office_convert, search_stream, BinaryInfo, FileContent, FileEntry, JarInfo, ReplaceSummary,
    SearchMode, SearchOptions, WriteOutcome, BINARY_PREVIEW_MAX_BYTES, EXECUTABLE_INSPECT_MAX_BYTES,
};
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
    handle.file_ops.read_file_for_editor(&path).await
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
    let (bytes, mtime, total_size, truncated) = handle.file_ops.read_bytes_for_editor(&path).await?;
    let text = encoding::decode_with(&bytes, &encoding_label).map_err(AppError::Internal)?;
    Ok(FileContent { text, encoding: encoding_label, mtime, total_size, truncated })
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

/// 编辑器图片/PDF/Word/Excel 预览（2026-08-28 用户反馈：这些文件在编辑器里被当文本
/// 打开显示乱码）：返回 base64，前端自己按扩展名分流成 `<img>`/`<iframe>`/mammoth/
/// xlsx 解析——是什么类型、用什么方式展示都是前端已经要维护的"按扩展名分流"逻辑的
/// 一部分，不需要后端再判断一遍。
#[tauri::command]
pub async fn fs_read_binary_preview(state: State<'_, AppState>, workspace_id: Uuid, path: String) -> Result<String, AppError> {
    let handle = get_handle(&state, workspace_id).await?;
    guard_local_path(&handle, &path)?;
    let bytes = handle.file_ops.read_binary_for_preview(&path, BINARY_PREVIEW_MAX_BYTES).await?;
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

/// "用系统默认程序打开"（Word/Excel/PDF 等编辑器不预览、或压根不支持的二进制文件，
/// 2026-08-28 用户反馈）：本地工作区直接开原路径；远程工作区先下载到本地临时目录
/// 再开——系统程序不认识 SSH 路径。临时文件故意不清理：用户可能还在用外部程序编辑，
/// 提前删掉会导致外部程序保存失败；操作系统的临时目录本身会被系统按需清理。
#[tauri::command]
pub async fn fs_open_externally(
    app_handle: AppHandle,
    state: State<'_, AppState>,
    workspace_id: Uuid,
    path: String,
) -> Result<(), AppError> {
    let handle = get_handle(&state, workspace_id).await?;
    guard_local_path(&handle, &path)?;

    let target = if handle.profile.kind == WorkspaceKind::Local {
        path.clone()
    } else {
        let file_name = path.rsplit('/').next().unwrap_or(&path);
        let tmp_dir = std::env::temp_dir().join("roc_desk_open");
        std::fs::create_dir_all(&tmp_dir)?;
        let local_path = tmp_dir.join(file_name);
        handle.file_ops.download_to_local_file(&path, &local_path.to_string_lossy()).await?;
        local_path.to_string_lossy().to_string()
    };

    app_handle
        .opener()
        .open_path(target, None::<&str>)
        .map_err(|e| AppError::Internal(e.to_string()))
}

/// 旧版二进制 Office 文档（.doc/.xls/.ppt 等）没有轻量级纯 JS 库能解析——不像
/// .docx/.xlsx 是 zip+XML，mammoth/xlsx.js 用不了。用本机安装的 LibreOffice 临时转成
/// PDF 再复用已有的 `PdfPreview` 渲染（2026-08-28 用户建议）。本地工作区直接转源文件；
/// 远程工作区先下载到本地临时目录（复用 `fs_open_externally` 的思路），LibreOffice
/// 本来也只能处理本地文件。转换产物和 `fs_open_externally` 一样落在系统临时目录，
/// 不主动清理。
#[tauri::command]
pub async fn fs_convert_legacy_office_to_pdf(state: State<'_, AppState>, workspace_id: Uuid, path: String) -> Result<String, AppError> {
    let handle = get_handle(&state, workspace_id).await?;
    guard_local_path(&handle, &path)?;

    let tmp_dir = std::env::temp_dir().join("roc_desk_office_convert");
    let source_path = if handle.profile.kind == WorkspaceKind::Local {
        PathBuf::from(&path)
    } else {
        std::fs::create_dir_all(&tmp_dir)?;
        let file_name = path.rsplit('/').next().unwrap_or(&path);
        let local_path = tmp_dir.join(file_name);
        handle.file_ops.download_to_local_file(&path, &local_path.to_string_lossy()).await?;
        local_path
    };

    let pdf_path = office_convert::convert_to_pdf(&source_path, &tmp_dir).await?;
    let bytes = tokio::fs::read(&pdf_path).await.map_err(AppError::from)?;
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

/// 打开 EXE/DLL/SO 等不可编辑的可执行文件时展示基本信息 + 依赖库列表（2026-08-28
/// 需求），复用 `BINARY_PREVIEW_MAX_BYTES` 同一个体积上限——道理和图片/PDF 预览一样，
/// 头部结构解析需要完整文件字节，太大的文件不值得整份读进内存只为看个摘要。
#[tauri::command]
pub async fn fs_inspect_binary(state: State<'_, AppState>, workspace_id: Uuid, path: String) -> Result<BinaryInfo, AppError> {
    let handle = get_handle(&state, workspace_id).await?;
    guard_local_path(&handle, &path)?;
    let bytes = handle.file_ops.read_binary_for_preview(&path, EXECUTABLE_INSPECT_MAX_BYTES).await?;
    binary_info::inspect(&bytes)
}

/// 没有已知可执行文件扩展名的文件打开前先嗅探开头 64 字节判断是不是 ELF/PE/Mach-O
/// （2026-08-28 用户反馈："Linux 的可执行文件默认是以文本编辑器查看的，需要改成
/// 查看它的基本信息和依赖库信息"——Linux 下的可执行文件习惯上不带扩展名，只靠
/// 扩展名分流会完全漏掉这类文件）。只读一小段，不是整篇，前端只对没有已知扩展名
/// 的文件调用这个命令，不会给每次打开文本文件都加一次往返。
#[tauri::command]
pub async fn fs_peek_is_binary(state: State<'_, AppState>, workspace_id: Uuid, path: String) -> Result<bool, AppError> {
    let handle = get_handle(&state, workspace_id).await?;
    guard_local_path(&handle, &path)?;
    let (head, _mtime) = handle.file_ops.read_file_raw_bounded(&path, 64).await?;
    Ok(binary_info::looks_like_binary(&head))
}

/// 打开 JAR 包时展示基本信息（manifest/Main-Class/Class-Path）+ 内部条目列表
/// （2026-08-28 需求），语义和 `fs_inspect_binary` 一致，只是解析的是 ZIP 结构而不是
/// PE/ELF 头。
#[tauri::command]
pub async fn fs_inspect_jar(state: State<'_, AppState>, workspace_id: Uuid, path: String) -> Result<JarInfo, AppError> {
    let handle = get_handle(&state, workspace_id).await?;
    guard_local_path(&handle, &path)?;
    let bytes = handle.file_ops.read_binary_for_preview(&path, EXECUTABLE_INSPECT_MAX_BYTES).await?;
    jar_info::inspect(&bytes)
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

/// 左侧目录树的"搜索"功能（参考 VS Code 全局搜索面板）：流式返回，见
/// `fsops::search_stream` 的取舍说明。这个命令本身不返回搜索结果——结果通过
/// `search:file-result` 事件一条条推给前端（`requestId` 用来在前端过滤出属于
/// 这次搜索的事件，忽略被新搜索取代的旧搜索还没来得及吐出来的尾巴），跑完/出错/
/// 被取代分别发 `search:done`/`search:error`。
///
/// **`scope_path`**（2026-08-18，用户右键某个子目录要求"需要支持对这个子目录的
/// 搜索选项"）：不传就是整个工作区根目录，传了就只在这个子目录下搜——本地场景走
/// `guard_local_path` 校验不能跑出工作区边界，和 Explorer 其它命令的边界语义一致。
/// 这不只是个方便功能，也是缓解"搜索慢"的直接手段：把遍历范围从几万个文件的整个
/// 项目收窄到几十个文件的一个子目录，本地是 I/O 量级的差别，远程更是 SFTP round
/// trip 次数的差别。
#[tauri::command]
pub async fn fs_search_stream(
    state: State<'_, AppState>,
    app_handle: AppHandle,
    workspace_id: Uuid,
    request_id: Uuid,
    scope_path: Option<String>,
    query: String,
    mode: SearchMode,
    options: SearchOptions,
) -> Result<(), AppError> {
    let handle = get_handle(&state, workspace_id).await?;
    let root = match &scope_path {
        Some(p) => {
            guard_local_path(&handle, p)?;
            p.clone()
        }
        None => handle.profile.root_path.clone(),
    };

    *state.active_search.lock().unwrap() = Some(request_id);
    let active_search = state.active_search.clone();
    let should_cancel = move || *active_search.lock().unwrap() != Some(request_id);

    let emit_handle = app_handle.clone();
    let on_file = move |file| {
        let _ = emit_handle.emit(
            "search:file-result",
            serde_json::json!({ "requestId": request_id, "file": file }),
        );
    };

    let result = search_stream(handle.file_ops.as_ref(), &root, &query, &options, mode, on_file, should_cancel).await;
    match result {
        Ok(truncated) => {
            let _ = app_handle.emit("search:done", serde_json::json!({ "requestId": request_id, "truncated": truncated }));
        }
        Err(e) => {
            let _ = app_handle.emit("search:error", serde_json::json!({ "requestId": request_id, "message": e.to_string() }));
        }
    }
    Ok(())
}

/// 手动"停止搜索"（2026-08-29 用户反馈：搜索开始后没有办法主动停下来，只能干等
/// 跑完）。复用 `fs_search_stream` 已有的取消机制——`should_cancel` 每处理一个
/// 目录/文件都会检查一次 `active_search` 是否还等于自己的 `request_id`，之前只有
/// "开始新搜索"会改写这个值触发取消，这里补上"用户主动点停止"这第二个触发源。
/// 只在 `request_id` 仍然匹配时才清空——避免停止按钮的这次点击滞后到达时，
/// 误伤了用户之后又输入新关键词触发的另一次搜索。
#[tauri::command]
pub fn fs_search_cancel(state: State<'_, AppState>, request_id: Uuid) {
    let mut active = state.active_search.lock().unwrap();
    if *active == Some(request_id) {
        *active = None;
    }
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
