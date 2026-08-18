pub mod profile;

use std::path::Path;
use std::sync::Arc;

use chrono::Utc;
use uuid::Uuid;

use crate::connection::ConnectionManager;
use crate::db::repo::workspace_repo::WorkspaceRepo;
use crate::error::AppError;
use crate::fsops::local::LocalFileOps;
use crate::fsops::remote::RemoteFileOps;
use crate::fsops::FileOps;
use crate::ssh::SshConnectionPool;

pub use profile::{WorkspaceKind, WorkspaceProfile};

/// 当前进程内已打开的工作区运行时句柄（不落库，见 CODE_DESIGN.md §3.8）。
#[derive(Clone)]
pub struct WorkspaceHandle {
    pub profile: WorkspaceProfile,
    pub file_ops: Arc<dyn FileOps>,
}

pub struct WorkspaceManager {
    repo: Arc<WorkspaceRepo>,
    connection_manager: Arc<ConnectionManager>,
    ssh_pool: Arc<SshConnectionPool>,
}

impl WorkspaceManager {
    pub fn new(
        repo: Arc<WorkspaceRepo>,
        connection_manager: Arc<ConnectionManager>,
        ssh_pool: Arc<SshConnectionPool>,
    ) -> Self {
        Self { repo, connection_manager, ssh_pool }
    }

    /// 打开本地文件夹作为工作区；若该路径此前已作为工作区打开过，复用同一个 id
    /// （否则每次重新打开同一个文件夹都会在"最近工作区"里产生重复项）。
    pub fn open_local(&self, path: &str) -> Result<WorkspaceHandle, AppError> {
        let root = Path::new(path);
        if !root.is_dir() {
            return Err(AppError::NotFound(format!("目录不存在: {path}")));
        }

        let display_name = root
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string());

        let profile = match self.repo.find_by_local_path(path)? {
            Some(mut existing) => {
                existing.last_opened_at = Some(Utc::now().to_rfc3339());
                existing
            }
            None => WorkspaceProfile {
                id: Uuid::new_v4(),
                kind: WorkspaceKind::Local,
                root_path: path.to_string(),
                connection_id: None,
                display_name,
                last_opened_at: Some(Utc::now().to_rfc3339()),
            },
        };
        self.repo.upsert(&profile)?;

        Ok(WorkspaceHandle {
            profile,
            file_ops: Arc::new(LocalFileOps),
        })
    }

    /// 连接远程主机并打开工作区（DESIGN.md §3.1.1）。内部经 `SshConnectionPool`
    /// 建连/复用连接（含 §3.2.1 的指纹校验），成功后用同一条连接构造 `RemoteFileOps`。
    pub async fn open_remote(&self, connection_id: Uuid, remote_path: &str) -> Result<WorkspaceHandle, AppError> {
        let connection = self
            .connection_manager
            .get(connection_id)?
            .ok_or_else(|| AppError::NotFound(format!("connection not found: {connection_id}")))?;

        let session = self.ssh_pool.get_or_connect(connection_id).await?;

        let display_name = format!(
            "{} ({}@{})",
            remote_path.trim_end_matches('/').rsplit('/').next().unwrap_or(remote_path),
            connection.username,
            connection.host
        );

        let profile = WorkspaceProfile {
            id: Uuid::new_v4(),
            kind: WorkspaceKind::Remote,
            root_path: remote_path.to_string(),
            connection_id: Some(connection_id),
            display_name,
            last_opened_at: Some(Utc::now().to_rfc3339()),
        };
        self.repo.upsert(&profile)?;

        Ok(WorkspaceHandle {
            profile,
            file_ops: Arc::new(RemoteFileOps::new(session)),
        })
    }

    pub fn list_recent(&self, limit: usize) -> Result<Vec<WorkspaceProfile>, AppError> {
        self.repo.list_recent(limit)
    }

    pub fn remove_from_recent(&self, id: Uuid) -> Result<(), AppError> {
        self.repo.remove(id)
    }

    pub fn touch(&self, id: Uuid) -> Result<(), AppError> {
        self.repo.touch_last_opened(id)
    }
}
