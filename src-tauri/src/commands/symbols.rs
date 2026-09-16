use tauri::State;
use uuid::Uuid;

use crate::error::AppError;
use crate::state::AppState;
use crate::symbols::{build_index, SymbolLocation};
use crate::workspace::WorkspaceHandle;

async fn get_handle(state: &State<'_, AppState>, workspace_id: Uuid) -> Result<WorkspaceHandle, AppError> {
    state
        .workspaces
        .read()
        .await
        .get(&workspace_id)
        .cloned()
        .ok_or_else(|| AppError::NotFound(format!("工作区未打开: {workspace_id}")))
}

/// 扫描整个工作区重建符号索引（打开工作区/手动触发时调用），返回索引到的符号数量
/// 供前端做个日志/提示（目前前端没有展示，静默失败也无所谓）。
#[tauri::command]
pub async fn symbols_build_index(state: State<'_, AppState>, workspace_id: Uuid) -> Result<usize, AppError> {
    let handle = get_handle(&state, workspace_id).await?;
    let index = build_index(handle.file_ops.as_ref(), &handle.profile.root_path).await?;
    let count = index.len();
    state.symbol_indexes.write().await.insert(workspace_id, index);
    Ok(count)
}

/// "转到定义/声明"——按光标处的单词查索引，命中多个候选时原样都返回，由前端
/// Monaco 的多结果 UI 让用户自己挑。
#[tauri::command]
pub async fn symbols_go_to_definition(
    state: State<'_, AppState>,
    workspace_id: Uuid,
    symbol: String,
) -> Result<Vec<SymbolLocation>, AppError> {
    let indexes = state.symbol_indexes.read().await;
    Ok(indexes
        .get(&workspace_id)
        .map(|index| index.lookup(&symbol))
        .unwrap_or_default())
}

/// 文件保存后增量重建这一个文件的符号条目，不用重跑整个工作区（前端保存成功后
/// 调用，见 CodeEditor.tsx 的 handleSave）。索引还没建过（比如打开工作区后没
/// 触发过 `symbols_build_index` 就直接编辑保存了）时，就地新建一份只含这一个
/// 文件的索引——后续真正跑一次全量 `symbols_build_index` 会整份覆盖掉它。
#[tauri::command]
pub async fn symbols_reindex_file(
    state: State<'_, AppState>,
    workspace_id: Uuid,
    path: String,
    content: String,
) -> Result<(), AppError> {
    let mut indexes = state.symbol_indexes.write().await;
    indexes
        .entry(workspace_id)
        .or_default()
        .index_file(&path, &content);
    Ok(())
}
