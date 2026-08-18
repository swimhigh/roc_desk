use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;
use uuid::Uuid;

use super::known_hosts::KnownHostsVerifier;
use super::session::SshSession;
use crate::connection::{ConnectionManager, ConnectionProfile};
use crate::error::AppError;
use crate::fsops::remote::RemoteFileOps;

/// 按连接档案 id 复用物理连接（DESIGN.md §3.2.2）：同一主机的终端、SFTP、
/// `run_command` 执行共用一条 `SshSession`，各自在其上开独立 Channel。
pub struct SshConnectionPool {
    sessions: RwLock<HashMap<Uuid, Arc<SshSession>>>,
    /// 每条连接对应一个共享的 `RemoteFileOps`（内部又懒缓存了一条 SFTP 子系统连接），
    /// 供工作区 Explorer 和 §3.3 的 SFTP 自由浏览快捷工具共用，不重复握手。
    file_ops: RwLock<HashMap<Uuid, Arc<RemoteFileOps>>>,
    connection_manager: Arc<ConnectionManager>,
    verifier: Arc<KnownHostsVerifier>,
}

impl SshConnectionPool {
    pub fn new(connection_manager: Arc<ConnectionManager>, verifier: Arc<KnownHostsVerifier>) -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
            file_ops: RwLock::new(HashMap::new()),
            connection_manager,
            verifier,
        }
    }

    pub async fn get_or_connect(&self, profile_id: Uuid) -> Result<Arc<SshSession>, AppError> {
        if let Some(existing) = self.sessions.read().await.get(&profile_id) {
            return Ok(existing.clone());
        }

        let profile: ConnectionProfile = self
            .connection_manager
            .get(profile_id)?
            .ok_or_else(|| AppError::NotFound(format!("connection not found: {profile_id}")))?;
        let secret = self.connection_manager.resolve_secret(&profile).await?;

        let session = Arc::new(SshSession::connect(&profile, secret, self.verifier.clone()).await?);
        self.connection_manager.touch_last_connected(profile_id)?;
        self.sessions.write().await.insert(profile_id, session.clone());
        Ok(session)
    }

    pub async fn get(&self, profile_id: Uuid) -> Option<Arc<SshSession>> {
        self.sessions.read().await.get(&profile_id).cloned()
    }

    /// 供 SFTP 自由浏览快捷工具（§3.3，无工作区边界限制）和工作区 Explorer 共用。
    pub async fn get_file_ops(&self, profile_id: Uuid) -> Result<Arc<RemoteFileOps>, AppError> {
        if let Some(existing) = self.file_ops.read().await.get(&profile_id) {
            return Ok(existing.clone());
        }
        let session = self.get_or_connect(profile_id).await?;
        let ops = Arc::new(RemoteFileOps::new(session));
        self.file_ops.write().await.insert(profile_id, ops.clone());
        Ok(ops)
    }

    pub async fn disconnect(&self, profile_id: Uuid) -> Result<(), AppError> {
        self.file_ops.write().await.remove(&profile_id);
        if let Some(session) = self.sessions.write().await.remove(&profile_id) {
            session.disconnect().await?;
        }
        Ok(())
    }
}
