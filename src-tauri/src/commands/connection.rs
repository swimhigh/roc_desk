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

/// 编辑连接后必须让已经缓存的连接池条目失效，否则改了密码/密钥/配对令牌不会
/// 生效——`SshConnectionPool`/`AgentConnectionPool` 都是"按 profile_id 复用一条
/// 已认证的物理连接"，编辑之前建立的那条连接完全不知道凭据已经变了，会一直用
/// 旧凭据握手时留下的旧连接，直到进程重启或连接自然断开（真实 bug：Agent 重新
/// `pair` 生成新令牌、客户端这边改了令牌后重连仍然报错，就是命中了这里——
/// 断开的是缓存的连接，不是正在使用的 UI 状态，下一次操作会用新凭据自动重连）。
#[tauri::command]
pub async fn connection_update(
    state: State<'_, AppState>,
    id: Uuid,
    input: ConnectionProfileInput,
) -> Result<ConnectionProfile, AppError> {
    let profile = state.connection_manager.update(id, input).await?;
    let _ = state.ssh_pool.disconnect(id).await;
    let _ = state.agent_pool.disconnect(id).await;
    Ok(profile)
}

#[tauri::command]
pub async fn connection_delete(state: State<'_, AppState>, id: Uuid) -> Result<(), AppError> {
    state.connection_manager.delete(id).await?;
    let _ = state.ssh_pool.disconnect(id).await;
    let _ = state.agent_pool.disconnect(id).await;
    Ok(())
}
