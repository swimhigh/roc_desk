use tauri::State;
use uuid::Uuid;

use crate::connection::{ConnectionGroup, ConnectionGroupInput};
use crate::error::AppError;
use crate::state::AppState;

#[tauri::command]
pub async fn connection_group_list(state: State<'_, AppState>) -> Result<Vec<ConnectionGroup>, AppError> {
    state.connection_group_manager.list()
}

#[tauri::command]
pub async fn connection_group_create(
    state: State<'_, AppState>,
    input: ConnectionGroupInput,
) -> Result<ConnectionGroup, AppError> {
    state.connection_group_manager.create(input)
}

/// 重命名 + 移动到另一个父分组共用这一个命令（`input.parent_id` 不变就是纯重命名）。
#[tauri::command]
pub async fn connection_group_update(
    state: State<'_, AppState>,
    id: Uuid,
    input: ConnectionGroupInput,
) -> Result<ConnectionGroup, AppError> {
    state.connection_group_manager.update(id, input)
}

#[tauri::command]
pub async fn connection_group_delete(state: State<'_, AppState>, id: Uuid) -> Result<(), AppError> {
    state.connection_group_manager.delete(id)
}
