use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;
use uuid::Uuid;

/// 建连本身没有超时保护过——2026-09 用户实测复现：远程主机网络层面不可达时
/// （不是"服务器拒绝"这种能立刻报错的场景，是连 TCP 握手都没人应答/被防火墙
/// 静默丢弃那种），`SshSession::connect` 会无限期挂起，`get_or_connect` 跟着
/// 永远不返回，调用方（`run_command`/SFTP/终端……）表现成"卡住不动"，界面上
/// 完全看不出是在等连接还是在等别的什么。给建连单独包一层超时，跟
/// `session.rs::EXEC_TIMEOUT`（命令执行超时）是同一个道理，只是这里更短——
/// 正常网络下握手应该在几秒内完成，30 秒已经足够宽松。
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

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
    pub fn new(
        connection_manager: Arc<ConnectionManager>,
        verifier: Arc<KnownHostsVerifier>,
    ) -> Self {
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

        let session = match tokio::time::timeout(
            CONNECT_TIMEOUT,
            SshSession::connect(&profile, secret, self.verifier.clone()),
        )
        .await
        {
            Ok(result) => Arc::new(result?),
            Err(_) => {
                return Err(AppError::Connection(format!(
                    "连接超时（{}s）：{}，请检查网络是否可达",
                    CONNECT_TIMEOUT.as_secs(),
                    profile.host
                )));
            }
        };
        self.connection_manager.touch_last_connected(profile_id)?;
        self.sessions
            .write()
            .await
            .insert(profile_id, session.clone());
        Ok(session)
    }

    pub async fn get(&self, profile_id: Uuid) -> Option<Arc<SshSession>> {
        self.sessions.read().await.get(&profile_id).cloned()
    }

    /// 主动清掉一条缓存连接——`is_alive()` 只检查本地的 handle 是否已经被显式
    /// 关闭（`!sender.is_closed()`），网络层面静默失联（服务器无响应/连接被
    /// NAT/防火墙悄悄丢弃，既没收到 FIN 也没收到 RST）时它仍然会报"活着"。
    /// 2026-09 用户实测复现：远程 SSH 目标下 `run_command`（`top`/`ps` 这类简单
    /// 只读命令）卡住 60~120 秒最终以 `EXEC_TIMEOUT` 超时收场——这之后如果不主动
    /// 清掉这条死连接，下一次 `get_or_connect` 还是会把同一条失联的连接原样交出去，
    /// 陷入"每次都要等满 120 秒超时"的死循环。调用方应该在拿到
    /// `SshSession::exec`/`open_sftp` 等操作的错误（尤其是超时）之后调用这个方法，
    /// 逼下一次 `get_or_connect` 走真正的重连路径。
    pub async fn evict(&self, profile_id: Uuid) {
        self.sessions.write().await.remove(&profile_id);
        self.file_ops.write().await.remove(&profile_id);
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
