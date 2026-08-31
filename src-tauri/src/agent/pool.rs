use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;
use uuid::Uuid;

use super::handshake::AgentCertVerifier;
use super::session::AgentSession;
use crate::connection::ConnectionManager;
use crate::error::AppError;

/// 按连接档案 id 复用一条 Agent TLS 连接，和 `SshConnectionPool` 同构
/// （AGENT_DESIGN.md §四.2）。
pub struct AgentConnectionPool {
    sessions: RwLock<HashMap<Uuid, Arc<AgentSession>>>,
    connection_manager: Arc<ConnectionManager>,
    cert_verifier: Arc<AgentCertVerifier>,
}

impl AgentConnectionPool {
    pub fn new(connection_manager: Arc<ConnectionManager>, cert_verifier: Arc<AgentCertVerifier>) -> Self {
        Self { sessions: RwLock::new(HashMap::new()), connection_manager, cert_verifier }
    }

    pub async fn get_or_connect(&self, profile_id: Uuid) -> Result<Arc<AgentSession>, AppError> {
        if let Some(existing) = self.sessions.read().await.get(&profile_id) {
            return Ok(existing.clone());
        }

        let profile = self
            .connection_manager
            .get(profile_id)?
            .ok_or_else(|| AppError::NotFound(format!("connection not found: {profile_id}")))?;
        let token = self
            .connection_manager
            .resolve_secret(&profile)
            .await?
            .ok_or_else(|| AppError::Auth("缺少配对令牌，请在连接设置里重新填写".into()))?;

        let session = Arc::new(AgentSession::connect(&profile, token, &self.cert_verifier).await?);
        self.connection_manager.touch_last_connected(profile_id)?;
        self.sessions.write().await.insert(profile_id, session.clone());
        Ok(session)
    }

    pub async fn get(&self, profile_id: Uuid) -> Option<Arc<AgentSession>> {
        self.sessions.read().await.get(&profile_id).cloned()
    }

    pub async fn disconnect(&self, profile_id: Uuid) -> Result<(), AppError> {
        self.sessions.write().await.remove(&profile_id);
        Ok(())
    }
}
