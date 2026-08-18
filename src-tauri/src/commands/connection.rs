use tauri::State;
use uuid::Uuid;

use crate::connection::{ConnectionProfile, ConnectionProfileInput};
use crate::error::AppError;
use crate::state::AppState;

#[tauri::command]
pub async fn connection_list(
    state: State<'_, AppState>,
    group_id: Option<Uuid>,
) -> Result<Vec<ConnectionProfile>, AppError> {
    state.connection_manager.list(group_id)
}

#[tauri::command]
pub async fn connection_create(
    state: State<'_, AppState>,
    input: ConnectionProfileInput,
) -> Result<ConnectionProfile, AppError> {
    state.connection_manager.create(input).await
}

#[tauri::command]
pub async fn connection_update(
    state: State<'_, AppState>,
    id: Uuid,
    input: ConnectionProfileInput,
) -> Result<ConnectionProfile, AppError> {
    state.connection_manager.update(id, input).await
}

#[tauri::command]
pub async fn connection_delete(state: State<'_, AppState>, id: Uuid) -> Result<(), AppError> {
    state.connection_manager.delete(id).await
}
