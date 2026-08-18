use tauri::{AppHandle, State};
use uuid::Uuid;

use crate::ai::{AiProvider, AiProviderInput, ChatMessage};
use crate::error::AppError;
use crate::state::AppState;

#[tauri::command]
pub async fn ai_provider_list(state: State<'_, AppState>) -> Result<Vec<AiProvider>, AppError> {
    state.ai_provider_manager.list()
}

#[tauri::command]
pub async fn ai_provider_create(
    state: State<'_, AppState>,
    input: AiProviderInput,
) -> Result<AiProvider, AppError> {
    state.ai_provider_manager.create(input).await
}

#[tauri::command]
pub async fn ai_provider_update(
    state: State<'_, AppState>,
    id: Uuid,
    input: AiProviderInput,
) -> Result<AiProvider, AppError> {
    state.ai_provider_manager.update(id, input).await
}

#[tauri::command]
pub async fn ai_provider_delete(state: State<'_, AppState>, id: Uuid) -> Result<(), AppError> {
    state.ai_provider_manager.delete(id).await
}

/// 发起一次流式对话请求；命令本身立即返回 `requestId`，增量文本经
/// `ai:chat-chunk`/`ai:chat-done`/`ai:chat-error` 事件推送（DESIGN.md §3.6）。
#[tauri::command]
pub async fn ai_chat_send(
    state: State<'_, AppState>,
    app_handle: AppHandle,
    provider_id: Uuid,
    messages: Vec<ChatMessage>,
    redact_enabled: bool,
) -> Result<Uuid, AppError> {
    let provider = state
        .ai_provider_manager
        .get(provider_id)?
        .ok_or_else(|| AppError::NotFound(format!("ai provider not found: {provider_id}")))?;
    let api_key = state.ai_provider_manager.resolve_api_key(&provider).await?;

    let request_id = Uuid::new_v4();
    let client = state.ai_chat_client.clone();
    tokio::spawn(async move {
        client
            .stream_chat(&provider, api_key.as_deref(), &messages, redact_enabled, app_handle, request_id)
            .await;
    });

    Ok(request_id)
}
