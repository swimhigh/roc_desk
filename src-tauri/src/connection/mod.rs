pub mod group;
pub mod profile;

use std::sync::Arc;

use chrono::Utc;
use uuid::Uuid;

use crate::credential::CredentialStore;
use crate::db::repo::connections_repo::ConnectionsRepo;
use crate::error::AppError;

pub use profile::{AuthMethod, ConnectionProfile, ConnectionProfileInput};

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
