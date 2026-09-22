use chrono::Utc;
use tauri::State;
use uuid::Uuid;

use crate::db::repo::http_request_history_repo::NewHistoryEntry;
use crate::error::AppError;
use crate::fsops::FileOps;
use crate::http_desk::model::*;
use crate::http_desk::{client, export, import, service};
use crate::http_desk::vars::VarContext;
use crate::state::AppState;
use crate::workspace::WorkspaceHandle;

/// HTTP 桌面的所有 command 都需要先拿到目标工作区已经打开的 `WorkspaceHandle`——
/// 打开动作本身复用 `commands::workspace::workspace_open_local`/`workspace_open_remote`
/// （docs/HTTP_DESKTOP_PLAN.md §3.1），这里不重复实现，只负责"已打开的工作区里
/// 找不到就报错"。
async fn get_handle(state: &State<'_, AppState>, workspace_id: Uuid) -> Result<WorkspaceHandle, AppError> {
    state
        .workspaces
        .read()
        .await
        .get(&workspace_id)
        .cloned()
        .ok_or_else(|| AppError::NotFound(format!("工作区尚未打开: {workspace_id}")))
}

async fn build_var_context_owned(
    file_ops: &dyn FileOps,
    root: &str,
    slug: &str,
    environment_id: Option<&str>,
) -> Result<(Vec<EnvVar>, Vec<EnvVar>, Vec<EnvVar>), AppError> {
    let environment = match environment_id {
        Some(id) if !id.is_empty() => service::list_environments(file_ops, root, slug)
            .await?
            .into_iter()
            .find(|e| e.id == id)
            .map(|e| e.variables)
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    let collection = service::collection_variables(file_ops, root, slug).await?;
    let global = service::get_global_variables(file_ops, root).await?;
    Ok((environment, collection, global))
}

fn redact(text: &str, secrets: &[String]) -> String {
    let mut out = text.to_string();
    for s in secrets {
        if !s.is_empty() {
            out = out.replace(s.as_str(), "***");
        }
    }
    out
}

// ---------------------------------------------------------------------
// 集合
// ---------------------------------------------------------------------

#[tauri::command]
pub async fn http_list_collections(
    state: State<'_, AppState>,
    workspace_id: Uuid,
) -> Result<Vec<HttpCollectionSummary>, AppError> {
    let handle = get_handle(&state, workspace_id).await?;
    service::list_collections(handle.file_ops.as_ref(), &handle.profile.root_path).await
}

#[tauri::command]
pub async fn http_create_collection(
    state: State<'_, AppState>,
    workspace_id: Uuid,
    name: String,
) -> Result<HttpCollectionSummary, AppError> {
    let handle = get_handle(&state, workspace_id).await?;
    service::create_collection(handle.file_ops.as_ref(), &handle.profile.root_path, &name).await
}

#[tauri::command]
pub async fn http_rename_collection(
    state: State<'_, AppState>,
    workspace_id: Uuid,
    slug: String,
    name: String,
) -> Result<(), AppError> {
    let handle = get_handle(&state, workspace_id).await?;
    service::rename_collection(handle.file_ops.as_ref(), &handle.profile.root_path, &slug, &name).await
}

#[tauri::command]
pub async fn http_delete_collection(
    state: State<'_, AppState>,
    workspace_id: Uuid,
    slug: String,
) -> Result<(), AppError> {
    let handle = get_handle(&state, workspace_id).await?;
    service::delete_collection(handle.file_ops.as_ref(), &handle.profile.root_path, &slug).await
}

// ---------------------------------------------------------------------
// 请求
// ---------------------------------------------------------------------

#[tauri::command]
pub async fn http_list_requests(
    state: State<'_, AppState>,
    workspace_id: Uuid,
    slug: String,
) -> Result<Vec<RequestSummary>, AppError> {
    let handle = get_handle(&state, workspace_id).await?;
    service::list_requests(handle.file_ops.as_ref(), &handle.profile.root_path, &slug).await
}

#[tauri::command]
pub async fn http_get_request(
    state: State<'_, AppState>,
    workspace_id: Uuid,
    slug: String,
    id: String,
) -> Result<RequestDef, AppError> {
    let handle = get_handle(&state, workspace_id).await?;
    service::get_request(handle.file_ops.as_ref(), &handle.profile.root_path, &slug, &id).await
}

#[tauri::command]
pub async fn http_create_request(
    state: State<'_, AppState>,
    workspace_id: Uuid,
    slug: String,
    name: String,
    folder: Vec<String>,
) -> Result<RequestDef, AppError> {
    let handle = get_handle(&state, workspace_id).await?;
    service::create_request(
        handle.file_ops.as_ref(),
        &handle.profile.root_path,
        &slug,
        &name,
        folder,
    )
    .await
}

#[tauri::command]
pub async fn http_save_request(
    state: State<'_, AppState>,
    workspace_id: Uuid,
    slug: String,
    request: RequestDef,
) -> Result<(), AppError> {
    let handle = get_handle(&state, workspace_id).await?;
    service::save_request(handle.file_ops.as_ref(), &handle.profile.root_path, &slug, &request).await
}

#[tauri::command]
pub async fn http_delete_request(
    state: State<'_, AppState>,
    workspace_id: Uuid,
    slug: String,
    id: String,
) -> Result<(), AppError> {
    let handle = get_handle(&state, workspace_id).await?;
    service::delete_request(handle.file_ops.as_ref(), &handle.profile.root_path, &slug, &id).await?;
    state
        .http_workspace_tabs
        .delete_by_request(&workspace_id.to_string(), &id)?;
    Ok(())
}

// ---------------------------------------------------------------------
// 环境 / 变量
// ---------------------------------------------------------------------

#[tauri::command]
pub async fn http_list_environments(
    state: State<'_, AppState>,
    workspace_id: Uuid,
    slug: String,
) -> Result<Vec<EnvironmentDef>, AppError> {
    let handle = get_handle(&state, workspace_id).await?;
    service::list_environments(handle.file_ops.as_ref(), &handle.profile.root_path, &slug).await
}

#[tauri::command]
pub async fn http_save_environment(
    state: State<'_, AppState>,
    workspace_id: Uuid,
    slug: String,
    environment: EnvironmentDef,
) -> Result<EnvironmentDef, AppError> {
    let handle = get_handle(&state, workspace_id).await?;
    service::save_environment(
        handle.file_ops.as_ref(),
        &handle.profile.root_path,
        &slug,
        environment,
    )
    .await
}

#[tauri::command]
pub async fn http_delete_environment(
    state: State<'_, AppState>,
    workspace_id: Uuid,
    slug: String,
    env_id: String,
) -> Result<(), AppError> {
    let handle = get_handle(&state, workspace_id).await?;
    service::delete_environment(handle.file_ops.as_ref(), &handle.profile.root_path, &slug, &env_id).await
}

#[tauri::command]
pub async fn http_get_global_variables(
    state: State<'_, AppState>,
    workspace_id: Uuid,
) -> Result<Vec<EnvVar>, AppError> {
    let handle = get_handle(&state, workspace_id).await?;
    service::get_global_variables(handle.file_ops.as_ref(), &handle.profile.root_path).await
}

#[tauri::command]
pub async fn http_save_global_variables(
    state: State<'_, AppState>,
    workspace_id: Uuid,
    variables: Vec<EnvVar>,
) -> Result<(), AppError> {
    let handle = get_handle(&state, workspace_id).await?;
    service::save_global_variables(handle.file_ops.as_ref(), &handle.profile.root_path, &variables).await
}

#[tauri::command]
pub async fn http_get_collection_meta(
    state: State<'_, AppState>,
    workspace_id: Uuid,
    slug: String,
) -> Result<HttpCollectionMeta, AppError> {
    let handle = get_handle(&state, workspace_id).await?;
    service::get_collection_meta(handle.file_ops.as_ref(), &handle.profile.root_path, &slug).await
}

#[tauri::command]
pub async fn http_save_collection_meta(
    state: State<'_, AppState>,
    workspace_id: Uuid,
    slug: String,
    meta: HttpCollectionMeta,
) -> Result<(), AppError> {
    let handle = get_handle(&state, workspace_id).await?;
    service::save_collection_meta(handle.file_ops.as_ref(), &handle.profile.root_path, &slug, &meta).await
}

// ---------------------------------------------------------------------
// 发送请求 + 历史
// ---------------------------------------------------------------------

#[tauri::command]
pub async fn http_send_request(
    state: State<'_, AppState>,
    workspace_id: Uuid,
    slug: String,
    request: RequestDef,
    environment_id: Option<String>,
) -> Result<HttpExecuteResult, AppError> {
    let handle = get_handle(&state, workspace_id).await?;
    let (environment, collection, global) = build_var_context_owned(
        handle.file_ops.as_ref(),
        &handle.profile.root_path,
        &slug,
        environment_id.as_deref(),
    )
    .await?;
    let secrets: Vec<String> = environment
        .iter()
        .chain(collection.iter())
        .chain(global.iter())
        .filter(|v| v.secret && !v.value.is_empty())
        .map(|v| v.value.clone())
        .collect();
    let ctx = VarContext {
        environment: &environment,
        collection: &collection,
        global: &global,
    };

    let outcome = client::execute(&request, &ctx).await;
    let created_at = Utc::now().to_rfc3339();
    let history_id = Uuid::new_v4().to_string();
    let request_snapshot = serde_json::to_string(&request).unwrap_or_default();

    match &outcome {
        Ok(result) => {
            let redacted = HttpExecuteResult {
                resolved_url: redact(&result.resolved_url, &secrets),
                body: redact(&result.body, &secrets),
                headers: result
                    .headers
                    .iter()
                    .map(|(k, v)| (k.clone(), redact(v, &secrets)))
                    .collect(),
                ..result.clone()
            };
            let response_snapshot = serde_json::to_string(&redacted).unwrap_or_default();
            let _ = state.http_request_history.create(&NewHistoryEntry {
                id: history_id,
                workspace_id: workspace_id.to_string(),
                collection_slug: slug,
                request_id: if request.id.is_empty() { None } else { Some(request.id.clone()) },
                environment_id,
                method: request.method.clone(),
                url: request.url.clone(),
                status_code: Some(result.status as i64),
                duration_ms: Some(result.duration_ms as i64),
                response_size_bytes: Some(result.size_bytes as i64),
                error_message: None,
                request_snapshot: request_snapshot.as_str(),
                response_snapshot: Some(response_snapshot.as_str()),
                created_at,
            });
        }
        Err(e) => {
            let _ = state.http_request_history.create(&NewHistoryEntry {
                id: history_id,
                workspace_id: workspace_id.to_string(),
                collection_slug: slug,
                request_id: if request.id.is_empty() { None } else { Some(request.id.clone()) },
                environment_id,
                method: request.method.clone(),
                url: request.url.clone(),
                status_code: None,
                duration_ms: None,
                response_size_bytes: None,
                error_message: Some(redact(&e.to_string(), &secrets)),
                request_snapshot: request_snapshot.as_str(),
                response_snapshot: None,
                created_at,
            });
        }
    }

    outcome
}

#[tauri::command]
pub fn http_import_curl(command: String) -> Result<RequestDef, AppError> {
    import::parse_curl(&command)
}

/// Postman/OpenAPI 导入整份文件内容后直接落成一个新集合（不是像 `http_import_curl`
/// 那样只解析出单个请求交给前端再手动建），因为源文件本来就是"一整份集合"的粒度，
/// 拆成"前端逐个调用 create_request"会引入没必要的中间态（一半导入成功、用户中途
/// 关掉对话框）。前端负责用文件选择器读出文件内容传进来，这里只管解析+落盘。
#[tauri::command]
pub async fn http_import_postman(
    state: State<'_, AppState>,
    workspace_id: Uuid,
    content: String,
) -> Result<HttpCollectionSummary, AppError> {
    let handle = get_handle(&state, workspace_id).await?;
    let imported = import::parse_postman_collection(&content)?;
    service::import_collection(handle.file_ops.as_ref(), &handle.profile.root_path, imported).await
}

#[tauri::command]
pub async fn http_import_openapi(
    state: State<'_, AppState>,
    workspace_id: Uuid,
    content: String,
) -> Result<HttpCollectionSummary, AppError> {
    let handle = get_handle(&state, workspace_id).await?;
    let imported = import::parse_openapi(&content)?;
    service::import_collection(handle.file_ops.as_ref(), &handle.profile.root_path, imported).await
}

#[tauri::command]
pub async fn http_export_postman(
    state: State<'_, AppState>,
    workspace_id: Uuid,
    slug: String,
) -> Result<String, AppError> {
    let handle = get_handle(&state, workspace_id).await?;
    export::to_postman_collection(handle.file_ops.as_ref(), &handle.profile.root_path, &slug).await
}

#[tauri::command]
pub async fn http_list_history(
    state: State<'_, AppState>,
    workspace_id: Uuid,
    limit: Option<i64>,
) -> Result<Vec<HttpRequestHistoryEntry>, AppError> {
    state
        .http_request_history
        .list_for_workspace(&workspace_id.to_string(), limit.unwrap_or(200))
}

#[tauri::command]
pub async fn http_get_history_detail(
    state: State<'_, AppState>,
    id: String,
) -> Result<Option<HttpRequestHistoryDetail>, AppError> {
    state.http_request_history.get_detail(&id)
}

#[tauri::command]
pub async fn http_delete_history(state: State<'_, AppState>, id: String) -> Result<(), AppError> {
    state.http_request_history.delete(&id)
}

#[tauri::command]
pub async fn http_clear_history(state: State<'_, AppState>, workspace_id: Uuid) -> Result<(), AppError> {
    state.http_request_history.clear_for_workspace(&workspace_id.to_string())
}

// ---------------------------------------------------------------------
// 标签页
// ---------------------------------------------------------------------

#[tauri::command]
pub async fn http_list_tabs(
    state: State<'_, AppState>,
    workspace_id: Uuid,
) -> Result<Vec<HttpWorkspaceTab>, AppError> {
    state.http_workspace_tabs.list_for_workspace(&workspace_id.to_string())
}

#[tauri::command]
pub async fn http_open_tab(
    state: State<'_, AppState>,
    workspace_id: Uuid,
    slug: String,
    request_id: String,
    title: String,
) -> Result<HttpWorkspaceTab, AppError> {
    let existing = state.http_workspace_tabs.list_for_workspace(&workspace_id.to_string())?;
    if let Some(tab) = existing.iter().find(|t| t.request_id == request_id) {
        return Ok(tab.clone());
    }
    let tab = HttpWorkspaceTab {
        id: Uuid::new_v4().to_string(),
        workspace_id: workspace_id.to_string(),
        collection_slug: slug,
        request_id,
        title,
        sort_order: existing.len() as i64,
        updated_at: Utc::now().to_rfc3339(),
    };
    state.http_workspace_tabs.create(&tab)?;
    Ok(tab)
}

#[tauri::command]
pub async fn http_close_tab(state: State<'_, AppState>, id: String) -> Result<(), AppError> {
    state.http_workspace_tabs.delete(&id)
}
