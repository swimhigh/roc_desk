pub mod profile;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::Utc;
use uuid::Uuid;
use serde::{Deserialize, Serialize};

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
    /// 工作区打开时一次性加载的本地元数据；用于校验会话不会跨工作区复用。
    pub metadata: WorkspaceMetadata,
    pub fallback_cache_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceMetadata {
    pub workspace_id: Uuid,
    pub kind: WorkspaceKind,
    pub root_path: String,
    pub connection_id: Option<Uuid>,
}

fn metadata_for(profile: &WorkspaceProfile) -> WorkspaceMetadata {
    WorkspaceMetadata { workspace_id: profile.id, kind: profile.kind, root_path: profile.root_path.clone(), connection_id: profile.connection_id }
}

pub struct WorkspaceManager {
    repo: Arc<WorkspaceRepo>,
    connection_manager: Arc<ConnectionManager>,
    ssh_pool: Arc<SshConnectionPool>,
    cache_root: PathBuf,
}

impl WorkspaceManager {
    pub fn new(
        repo: Arc<WorkspaceRepo>,
        connection_manager: Arc<ConnectionManager>,
        ssh_pool: Arc<SshConnectionPool>,
        cache_root: PathBuf,
    ) -> Self {
        Self { repo, connection_manager, ssh_pool, cache_root }
    }

    fn write_fallback_metadata(&self, metadata: &WorkspaceMetadata) -> Result<(), AppError> {
        let dir = self.cache_root.join(metadata.workspace_id.to_string());
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("workspace.json"), serde_json::to_vec_pretty(metadata).map_err(|e| AppError::Internal(e.to_string()))?)?;
        Ok(())
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

        let embedded = std::fs::read(root.join(".rock_desk").join("workspace.json"))
            .ok().and_then(|bytes| serde_json::from_slice::<WorkspaceMetadata>(&bytes).ok())
            .filter(|meta| meta.kind == WorkspaceKind::Local && meta.root_path.eq_ignore_ascii_case(path));
        let profile = match self.repo.find_by_local_path(path)? {
            Some(mut existing) => {
                existing.last_opened_at = Some(Utc::now().to_rfc3339());
                existing
            }
            None => WorkspaceProfile {
                id: embedded.map(|meta| meta.workspace_id).unwrap_or_else(Uuid::new_v4),
                kind: WorkspaceKind::Local,
                root_path: path.to_string(),
                connection_id: None,
                display_name,
                last_opened_at: Some(Utc::now().to_rfc3339()),
            },
        };
        self.repo.upsert(&profile)?;

        let metadata = metadata_for(&profile);
        self.write_fallback_metadata(&metadata)?;
        let metadata_dir = root.join(".rock_desk");
        std::fs::create_dir_all(&metadata_dir)?;
        std::fs::write(metadata_dir.join("workspace.json"), serde_json::to_vec_pretty(&metadata).map_err(|e| AppError::Internal(e.to_string()))?)?;
        let fallback_cache_dir = self.cache_root.join(metadata.workspace_id.to_string());
        Ok(WorkspaceHandle { profile, file_ops: Arc::new(LocalFileOps), metadata, fallback_cache_dir })
    }

    /// 连接远程主机并打开工作区（DESIGN.md §3.1.1）。内部经 `SshConnectionPool`
    /// 建连/复用连接（含 §3.2.1 的指纹校验），成功后用同一条连接构造 `RemoteFileOps`。
    ///
    /// 若该"连接档案 + 远程目录"组合此前已经打开过，复用同一个 id（和 `open_local`
    /// 对同一本地路径的处理方式一致），否则每次重新打开同一个远程工作区都会在
    /// "最近工作区"里产生一条新记录（真实 bug：2026-08-18 用户报告同一个远程目录
    /// 出现了 4 条一模一样的记录——此前这里漏了这一步判断，是本文件唯一没有走
    /// "查是否已存在"路径的分支）。
    pub async fn open_remote(&self, connection_id: Uuid, remote_path: &str) -> Result<WorkspaceHandle, AppError> {
        let connection = self
            .connection_manager
            .get(connection_id)?
            .ok_or_else(|| AppError::NotFound(format!("connection not found: {connection_id}")))?;

        let session = self.ssh_pool.get_or_connect(connection_id).await?;
        let file_ops: Arc<dyn FileOps> = Arc::new(RemoteFileOps::new(session));
        let metadata_path = format!("{}/.rock_desk/workspace.json", remote_path.trim_end_matches('/'));
        let embedded = file_ops.read_file(&metadata_path).await.ok()
            .and_then(|file| serde_json::from_str::<WorkspaceMetadata>(&file.text).ok())
            .filter(|meta| meta.kind == WorkspaceKind::Remote && meta.root_path == remote_path && meta.connection_id == Some(connection_id));

        let profile = match self.repo.find_by_remote(connection_id, remote_path)? {
            Some(mut existing) => {
                existing.last_opened_at = Some(Utc::now().to_rfc3339());
                existing
            }
            None => {
                let display_name = format!(
                    "{} ({}@{})",
                    remote_path.trim_end_matches('/').rsplit('/').next().unwrap_or(remote_path),
                    connection.username,
                    connection.host
                );
                WorkspaceProfile {
                    id: embedded.map(|meta| meta.workspace_id).unwrap_or_else(Uuid::new_v4),
                    kind: WorkspaceKind::Remote,
                    root_path: remote_path.to_string(),
                    connection_id: Some(connection_id),
                    display_name,
                    last_opened_at: Some(Utc::now().to_rfc3339()),
                }
            }
        };
        self.repo.upsert(&profile)?;

        let metadata = metadata_for(&profile);
        self.write_fallback_metadata(&metadata)?;
        // 远程目录可写时将工作区身份放在远端；只读目录则降级到本机全局缓存，
        // 但当前会话仍严格使用 RemoteFileOps，不会退回本地文件系统。
        let metadata_dir = format!("{}/.rock_desk", remote_path.trim_end_matches('/'));
        if file_ops.create_dir(&metadata_dir).await.is_ok() {
            let metadata_path = format!("{metadata_dir}/workspace.json");
            let _ = file_ops.write_file(&metadata_path, &serde_json::to_string_pretty(&metadata).unwrap_or_default(), None).await;
        }
        let fallback_cache_dir = self.cache_root.join(metadata.workspace_id.to_string());
        Ok(WorkspaceHandle { profile, file_ops, metadata, fallback_cache_dir })
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
