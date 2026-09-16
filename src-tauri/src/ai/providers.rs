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
    /// `"chat_completions"`（默认）或 `"responses"`——2026-09 老引擎新增 OpenAI
    /// Responses API 支持后，每个 provider 要标注自己用哪种 wire 协议，用户在
    /// Provider 设置里手动选，不做自动探测。裸字符串而不是枚举是跟现有代码风格
    /// 保持一致（`is_local` 也是裸 `bool`），`session.rs` 用 `== "responses"`
    /// 判断即可。
    pub wire_api: String,
    /// 对齐 Codex `config.toml` 的 `model_reasoning_effort`——gpt-5/o 系列这类
    /// 推理模型可以从客户端调节内部推理力度。`None`/空字符串表示不传这个参数
    /// （维持之前"交给服务端默认值"的行为，兼容不支持这个参数的 Provider）；
    /// 非空时是 `"minimal"`/`"low"`/`"medium"`/`"high"` 之一，`session.rs` 按
    /// `wire_api` 决定塞进 `reasoning.effort`（Responses API）还是顶层
    /// `reasoning_effort`（chat/completions 兼容层，不是所有中转都认这个字段，
    /// 用户自己判断要不要填）。裸 `Option<String>` 而不是枚举，跟 `wire_api`
    /// 同样的理由——校验交给前端下拉框，不在这里重复定义一遍取值范围。
    pub reasoning_effort: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AiProviderInput {
    pub name: String,
    pub api_base: String,
    pub api_key: Option<String>,
    pub model: String,
    pub is_local: bool,
    pub wire_api: String,
    pub reasoning_effort: Option<String>,
}

fn credential_key(id: Uuid) -> String {
    format!("ai:{id}:api_key")
}

/// 前端下拉框"不设置"选项传的是空字符串而不是 `null`（HTML `<select>` 的
/// 惯常写法）——这里统一收敛成 `None`，避免数据库里存进一个空字符串、后续
/// `session.rs` 还要另外判断"是 None 还是空串"两种"不生效"的写法。
fn normalize_reasoning_effort(effort: Option<String>) -> Option<String> {
    effort.filter(|e| !e.trim().is_empty())
}

pub struct AiProviderManager {
    repo: Arc<AiProvidersRepo>,
    credential_store: Arc<dyn CredentialStore>,
}

impl AiProviderManager {
    pub fn new(repo: Arc<AiProvidersRepo>, credential_store: Arc<dyn CredentialStore>) -> Self {
        Self {
            repo,
            credential_store,
        }
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
            wire_api: input.wire_api,
            reasoning_effort: normalize_reasoning_effort(input.reasoning_effort),
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
                let cred_key = existing
                    .api_key_ref
                    .clone()
                    .unwrap_or_else(|| credential_key(id));
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
            wire_api: input.wire_api,
            reasoning_effort: normalize_reasoning_effort(input.reasoning_effort),
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
