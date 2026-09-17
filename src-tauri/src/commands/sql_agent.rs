use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, State};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::coding::tools::TodoItem;
use crate::db::repo::sql_agent_history_repo::{SqlAgentHistoryDetail, SqlAgentHistoryInput, SqlAgentHistorySummary};
use crate::error::AppError;
use crate::sql::agent::SqlAgentSession;
use crate::state::AppState;

/// SQL Agent 的会话信息——`coding::commands::CodingSessionInfo` 的简化版，见
/// `sql::agent::session` 模块文档里"没有搬哪些 coding 概念"的说明。
#[derive(Serialize)]
pub struct SqlAgentSessionInfo {
    pub id: Uuid,
    pub provider_id: Uuid,
    pub todos: Vec<TodoItem>,
}

async fn session_info(session: &SqlAgentSession) -> SqlAgentSessionInfo {
    SqlAgentSessionInfo { id: session.id, provider_id: session.provider_id, todos: session.todos.clone() }
}

async fn get_session(state: &State<'_, AppState>, data_source_id: Uuid) -> Result<Arc<Mutex<SqlAgentSession>>, AppError> {
    state
        .sql_agent_sessions
        .read()
        .await
        .get(&data_source_id)
        .cloned()
        .ok_or_else(|| AppError::NotFound(format!("sql agent session not started: {data_source_id}")))
}

#[tauri::command]
pub async fn sql_agent_start(
    state: State<'_, AppState>,
    data_source_id: Uuid,
    provider_id: Uuid,
) -> Result<SqlAgentSessionInfo, AppError> {
    if state.ai_provider_manager.get(provider_id)?.is_none() {
        return Err(AppError::NotFound(format!("ai provider not found: {provider_id}")));
    }
    if let Some(existing) = state.sql_agent_sessions.read().await.get(&data_source_id).cloned() {
        let mut guard = existing.lock().await;
        if state.ai_provider_manager.get(guard.provider_id)?.is_some() {
            return Ok(session_info(&guard).await);
        }
        guard.provider_id = provider_id;
        return Ok(session_info(&guard).await);
    }
    let profile = state
        .sql_data_source_service
        .get(data_source_id)?
        .ok_or_else(|| AppError::NotFound(format!("data source not found: {data_source_id}")))?;
    let session = SqlAgentSession::new(data_source_id, provider_id, &profile.name, profile.db_kind);
    let info = session_info(&session).await;
    state.sql_agent_sessions.write().await.insert(data_source_id, Arc::new(Mutex::new(session)));
    Ok(info)
}

#[tauri::command]
pub async fn sql_agent_new_session(
    state: State<'_, AppState>,
    data_source_id: Uuid,
    provider_id: Uuid,
) -> Result<SqlAgentSessionInfo, AppError> {
    if state.ai_provider_manager.get(provider_id)?.is_none() {
        return Err(AppError::NotFound(format!("ai provider not found: {provider_id}")));
    }
    state.sql_agent_sessions.write().await.remove(&data_source_id);
    let profile = state
        .sql_data_source_service
        .get(data_source_id)?
        .ok_or_else(|| AppError::NotFound(format!("data source not found: {data_source_id}")))?;
    let session = SqlAgentSession::new(data_source_id, provider_id, &profile.name, profile.db_kind);
    let info = session_info(&session).await;
    state.sql_agent_sessions.write().await.insert(data_source_id, Arc::new(Mutex::new(session)));
    Ok(info)
}

#[tauri::command]
pub async fn sql_agent_close(state: State<'_, AppState>, data_source_id: Uuid) -> Result<(), AppError> {
    state.sql_agent_sessions.write().await.remove(&data_source_id);
    Ok(())
}

#[tauri::command]
pub async fn sql_agent_set_provider(
    state: State<'_, AppState>,
    data_source_id: Uuid,
    provider_id: Uuid,
) -> Result<(), AppError> {
    if state.ai_provider_manager.get(provider_id)?.is_none() {
        return Err(AppError::NotFound(format!("ai provider not found: {provider_id}")));
    }
    let session = get_session(&state, data_source_id).await?;
    session.lock().await.provider_id = provider_id;
    Ok(())
}

#[tauri::command]
pub async fn sql_agent_send_message(
    state: State<'_, AppState>,
    app_handle: AppHandle,
    data_source_id: Uuid,
    text: String,
) -> Result<String, AppError> {
    let session = get_session(&state, data_source_id).await?;
    let cancel_token = tokio_util::sync::CancellationToken::new();
    state.sql_agent_cancel_tokens.lock().unwrap().insert(data_source_id, cancel_token.clone());
    let mut session = session.lock().await;
    let result = session
        .send_message(
            &text,
            &state.ai_provider_manager,
            &state.sql_data_source_service,
            &state.sql_session_manager,
            &state.sql_query_history,
            &state.sql_agent_confirms,
            &state.sql_agent_questions,
            &app_handle,
            &cancel_token,
        )
        .await;
    state.sql_agent_cancel_tokens.lock().unwrap().remove(&data_source_id);
    result
}

#[tauri::command]
pub async fn sql_agent_cancel_turn(state: State<'_, AppState>, data_source_id: Uuid) -> Result<(), AppError> {
    if let Some(token) = state.sql_agent_cancel_tokens.lock().unwrap().get(&data_source_id) {
        token.cancel();
    }
    Ok(())
}

#[tauri::command]
pub async fn sql_agent_resolve_confirm(state: State<'_, AppState>, request_id: Uuid, allow: bool) -> Result<(), AppError> {
    state.sql_agent_confirms.resolve(request_id, allow).await;
    Ok(())
}

#[tauri::command]
pub async fn sql_agent_answer_question(state: State<'_, AppState>, request_id: Uuid, answer: String) -> Result<(), AppError> {
    state.sql_agent_questions.resolve(request_id, answer).await;
    Ok(())
}

#[tauri::command]
pub fn sql_agent_history_list(state: State<'_, AppState>, data_source_id: Uuid) -> Result<Vec<SqlAgentHistorySummary>, AppError> {
    state.sql_agent_history.list(data_source_id)
}

#[tauri::command]
pub fn sql_agent_history_get(state: State<'_, AppState>, id: Uuid) -> Result<Option<SqlAgentHistoryDetail>, AppError> {
    state.sql_agent_history.get(id)
}

#[tauri::command]
pub async fn sql_agent_history_save(state: State<'_, AppState>, input: SqlAgentHistoryInput) -> Result<(), AppError> {
    let mut input = input;
    if let Some(session) = state.sql_agent_sessions.read().await.get(&input.data_source_id).cloned() {
        let messages = session.lock().await.messages_snapshot();
        input.messages = serde_json::to_value(&messages).unwrap_or_default();
    }
    state.sql_agent_history.save(&input)
}

/// 打开一条历史记录并真正接续对话——把持久化的 `messages`（发给 AI 的真实对话
/// 上下文）灌回一个新构造的 `SqlAgentSession`，替换掉这个数据源当前的活跃
/// 会话；`session.id` 复用历史记录自己的 id，后续发消息时前端"保存历史"更新
/// 的还是同一行记录。
#[tauri::command]
pub async fn sql_agent_history_resume(
    state: State<'_, AppState>,
    data_source_id: Uuid,
    history_id: Uuid,
) -> Result<SqlAgentSessionInfo, AppError> {
    let detail = state
        .sql_agent_history
        .get(history_id)?
        .ok_or_else(|| AppError::NotFound(format!("history not found: {history_id}")))?;
    if detail.data_source_id != data_source_id {
        return Err(AppError::Internal("这条历史记录不属于当前数据源".into()));
    }
    if state.ai_provider_manager.get(detail.summary.provider_id)?.is_none() {
        return Err(AppError::NotFound(
            "这条历史记录关联的 AI 供应商已被删除，请先在模型管理里重新配置后再试".into(),
        ));
    }
    let profile = state
        .sql_data_source_service
        .get(data_source_id)?
        .ok_or_else(|| AppError::NotFound(format!("data source not found: {data_source_id}")))?;
    let mut session = SqlAgentSession::new(data_source_id, detail.summary.provider_id, &profile.name, profile.db_kind);
    session.id = history_id;
    let messages: Vec<serde_json::Value> = serde_json::from_value(detail.messages.clone()).unwrap_or_default();
    session.restore_messages(messages);

    let info = session_info(&session).await;
    state.sql_agent_sessions.write().await.insert(data_source_id, Arc::new(Mutex::new(session)));
    Ok(info)
}

#[tauri::command]
pub fn sql_agent_history_rename(state: State<'_, AppState>, id: Uuid, title: String) -> Result<(), AppError> {
    state.sql_agent_history.rename(id, title.trim())
}

#[tauri::command]
pub fn sql_agent_history_delete(state: State<'_, AppState>, id: Uuid) -> Result<(), AppError> {
    state.sql_agent_history.delete(id)
}
