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
            if existing.is_alive() {
                return Ok(existing.clone());
            }
        }
        // 缓存的会话已经断线（网络掉线、服务器重启等）——不能就这么把死连接
        // 交出去，否则后面任何 Channel/SFTP 操作都会失败，"重新连接"点了跟没点
        // 一样（真实反馈：不重启整个 app 这个工作区/终端就再也连不上了）。清掉
        // 这条缓存和绑定的 file_ops，往下走正常的建连路径重新连一条。
        self.sessions.write().await.remove(&profile_id);
        self.file_ops.write().await.remove(&profile_id);

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
        // 缓存的 `RemoteFileOps` 内部攥着一份 `Arc<SshSession>`——`get_or_connect`
        // 那边把死会话从 `sessions` 里清掉之后，这里如果还直接把缓存的 `RemoteFileOps`
        // 交出去，拿到的还是包着死连接的那个旧对象，等于没修。用 `get_or_connect`
        // 先问一次连接是不是还活着，活着才信任缓存的 `file_ops`。
        let session = self.get_or_connect(profile_id).await?;
        if let Some(existing) = self.file_ops.read().await.get(&profile_id) {
            return Ok(existing.clone());
        }
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
