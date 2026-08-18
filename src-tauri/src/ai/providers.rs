use std::sync::Arc;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::credential::CredentialStore;
use crate::db::repo::ai_providers_repo::AiProvidersRepo;
use crate::error::AppError;

/// 多模型适配（DESIGN.md §3.6）：豆包/OpenAI 兼容/DeepSeek/通义千问都走同一套
/// OpenAI 兼容协议，区别只在 `api_base`/`model`；本地 Ollama 同样兼容该协议，
/// 用 `is_local` 标注是为了 UI 做"本地/云端"视觉区分和未来的脱敏策略判断
/// （云端 Provider 才需要担心 DESIGN.md §3.6 提到的数据出境风险）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiProvider {
    pub id: Uuid,
    pub name: String,
    pub api_base: String,
    pub api_key_ref: Option<String>,
    pub model: String,
    pub is_local: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AiProviderInput {
    pub name: String,
    pub api_base: String,
    pub api_key: Option<String>,
    pub model: String,
    pub is_local: bool,
}

fn credential_key(id: Uuid) -> String {
    format!("ai:{id}:api_key")
}

pub struct AiProviderManager {
    repo: Arc<AiProvidersRepo>,
    credential_store: Arc<dyn CredentialStore>,
}

impl AiProviderManager {
    pub fn new(repo: Arc<AiProvidersRepo>, credential_store: Arc<dyn CredentialStore>) -> Self {
        Self { repo, credential_store }
    }

    pub async fn create(&self, input: AiProviderInput) -> Result<AiProvider, AppError> {
        let id = Uuid::new_v4();
        let api_key_ref = if let Some(key) = &input.api_key {
            if key.is_empty() {
                None
            } else {
                let cred_key = credential_key(id);
                self.credential_store.set(&cred_key, key).await?;
                Some(cred_key)
            }
        } else {
            None
        };

        let provider = AiProvider {
            id,
            name: input.name,
            api_base: input.api_base,
            api_key_ref,
            model: input.model,
            is_local: input.is_local,
            created_at: Utc::now().to_rfc3339(),
        };
        self.repo.create(&provider)?;
        Ok(provider)
    }

    pub async fn update(&self, id: Uuid, input: AiProviderInput) -> Result<AiProvider, AppError> {
        let existing = self
            .repo
            .get(id)?
            .ok_or_else(|| AppError::NotFound(format!("ai provider not found: {id}")))?;

        let api_key_ref = if let Some(key) = &input.api_key {
            if key.is_empty() {
                existing.api_key_ref
            } else {
                let cred_key = existing.api_key_ref.clone().unwrap_or_else(|| credential_key(id));
                self.credential_store.set(&cred_key, key).await?;
                Some(cred_key)
            }
        } else {
            existing.api_key_ref
        };

        let provider = AiProvider {
            id,
            name: input.name,
            api_base: input.api_base,
            api_key_ref,
            model: input.model,
            is_local: input.is_local,
            created_at: existing.created_at,
        };
        self.repo.update(&provider)?;
        Ok(provider)
    }

    pub async fn delete(&self, id: Uuid) -> Result<(), AppError> {
        if let Some(existing) = self.repo.get(id)? {
            if let Some(key) = existing.api_key_ref {
                self.credential_store.delete(&key).await?;
            }
        }
        self.repo.delete(id)?;
        Ok(())
    }

    pub fn list(&self) -> Result<Vec<AiProvider>, AppError> {
        self.repo.list()
    }

    pub fn get(&self, id: Uuid) -> Result<Option<AiProvider>, AppError> {
        self.repo.get(id)
    }

    pub async fn resolve_api_key(&self, provider: &AiProvider) -> Result<Option<String>, AppError> {
        match &provider.api_key_ref {
            Some(key) => self.credential_store.get(key).await,
            None => Ok(None),
        }
    }
}
