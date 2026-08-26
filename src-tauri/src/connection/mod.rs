pub mod group;
pub mod profile;

use std::sync::Arc;

use chrono::Utc;
use uuid::Uuid;

use crate::credential::CredentialStore;
use crate::db::repo::connection_groups_repo::ConnectionGroupsRepo;
use crate::db::repo::connections_repo::ConnectionsRepo;
use crate::error::AppError;

pub use group::{ConnectionGroup, ConnectionGroupInput};
pub use profile::{AuthMethod, ConnectionProfile, ConnectionProfileInput, Protocol};

pub struct ConnectionManager {
    repo: Arc<ConnectionsRepo>,
    credential_store: Arc<dyn CredentialStore>,
}

fn credential_key(id: Uuid) -> String {
    format!("ssh:{id}:secret")
}

impl ConnectionManager {
    pub fn new(repo: Arc<ConnectionsRepo>, credential_store: Arc<dyn CredentialStore>) -> Self {
        Self { repo, credential_store }
    }

    pub async fn create(&self, input: ConnectionProfileInput) -> Result<ConnectionProfile, AppError> {
        let id = Uuid::new_v4();
        let credential_ref = if let Some(secret) = &input.secret {
            let key = credential_key(id);
            self.credential_store.set(&key, secret).await?;
            Some(key)
        } else {
            None
        };

        let profile = ConnectionProfile {
            id,
            name: input.name,
            host: input.host,
            port: input.port,
            username: input.username,
            auth_method: input.auth_method,
            credential_ref,
            group_id: input.group_id,
            tags: input.tags,
            jump_host_id: input.jump_host_id,
            protocol: input.protocol,
            options: input.options,
            last_connected_at: None,
            created_at: Utc::now().to_rfc3339(),
        };
        self.repo.create(&profile)?;
        Ok(profile)
    }

    pub async fn update(&self, id: Uuid, input: ConnectionProfileInput) -> Result<ConnectionProfile, AppError> {
        let existing = self
            .repo
            .get(id)?
            .ok_or_else(|| AppError::NotFound(format!("connection not found: {id}")))?;

        let credential_ref = if let Some(secret) = &input.secret {
            let key = existing.credential_ref.clone().unwrap_or_else(|| credential_key(id));
            self.credential_store.set(&key, secret).await?;
            Some(key)
        } else {
            existing.credential_ref
        };

        let profile = ConnectionProfile {
            id,
            name: input.name,
            host: input.host,
            port: input.port,
            username: input.username,
            auth_method: input.auth_method,
            credential_ref,
            group_id: input.group_id,
            tags: input.tags,
            jump_host_id: input.jump_host_id,
            protocol: input.protocol,
            options: input.options,
            last_connected_at: existing.last_connected_at,
            created_at: existing.created_at,
        };
        self.repo.update(&profile)?;
        Ok(profile)
    }

    pub async fn delete(&self, id: Uuid) -> Result<(), AppError> {
        if let Some(existing) = self.repo.get(id)? {
            if let Some(key) = existing.credential_ref {
                self.credential_store.delete(&key).await?;
            }
        }
        self.repo.delete(id)?;
        Ok(())
    }

    pub fn list(&self, group_id: Option<Uuid>) -> Result<Vec<ConnectionProfile>, AppError> {
        self.repo.list(group_id)
    }

    pub fn get(&self, id: Uuid) -> Result<Option<ConnectionProfile>, AppError> {
        self.repo.get(id)
    }

    pub async fn resolve_secret(&self, profile: &ConnectionProfile) -> Result<Option<String>, AppError> {
        match &profile.credential_ref {
            Some(key) => self.credential_store.get(key).await,
            None => Ok(None),
        }
    }

    pub fn touch_last_connected(&self, id: Uuid) -> Result<(), AppError> {
        self.repo.touch_last_connected(id)
    }
}

/// 会话树的文件夹管理（远程工具模式，DESIGN.md §3.9）。不涉及密钥，比 `ConnectionManager`
/// 简单一截——唯一需要自己把关的是"移动不能把一个分组挪进它自己的子孙里"，SQLite 的外键
/// 约束管不到这种循环引用。
pub struct ConnectionGroupManager {
    repo: Arc<ConnectionGroupsRepo>,
}

impl ConnectionGroupManager {
    pub fn new(repo: Arc<ConnectionGroupsRepo>) -> Self {
        Self { repo }
    }

    pub fn list(&self) -> Result<Vec<ConnectionGroup>, AppError> {
        self.repo.list()
    }

    pub fn create(&self, input: ConnectionGroupInput) -> Result<ConnectionGroup, AppError> {
        if let Some(parent_id) = input.parent_id {
            self.repo
                .get(parent_id)?
                .ok_or_else(|| AppError::NotFound(format!("parent group not found: {parent_id}")))?;
        }
        let group = ConnectionGroup { id: Uuid::new_v4(), name: input.name, parent_id: input.parent_id };
        self.repo.create(&group)?;
        Ok(group)
    }

    pub fn update(&self, id: Uuid, input: ConnectionGroupInput) -> Result<ConnectionGroup, AppError> {
        self.repo.get(id)?.ok_or_else(|| AppError::NotFound(format!("group not found: {id}")))?;
        if let Some(parent_id) = input.parent_id {
            if parent_id == id || self.is_descendant(parent_id, id)? {
                return Err(AppError::Conflict("不能把分组移动到它自己的子分组里".into()));
            }
        }
        let group = ConnectionGroup { id, name: input.name, parent_id: input.parent_id };
        self.repo.update(&group)?;
        Ok(group)
    }

    pub fn delete(&self, id: Uuid) -> Result<(), AppError> {
        self.repo.delete(id)
    }

    /// `candidate` 是不是 `ancestor_id` 的子孙（沿 parent_id 往上走，走到根或走出
    /// 一个合理的深度上限都算"不是"——上限只是防一份被破坏的数据出现循环引用时
    /// 死循环，正常的分组树几层深就够了）。
    fn is_descendant(&self, candidate: Uuid, ancestor_id: Uuid) -> Result<bool, AppError> {
        let mut current = Some(candidate);
        for _ in 0..64 {
            let Some(id) = current else { return Ok(false) };
            if id == ancestor_id {
                return Ok(true);
            }
            current = self.repo.get(id)?.and_then(|g| g.parent_id);
        }
        Ok(false)
    }
}
