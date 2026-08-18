use std::collections::HashMap;
use std::sync::Arc;

use tauri::{AppHandle, Emitter};
use tokio::sync::{oneshot, Mutex};
use uuid::Uuid;

use crate::db::repo::known_hosts_repo::{KnownHostStatus, KnownHostsRepo};
use crate::error::AppError;

/// 等待前端响应的 TOFU / 指纹变化确认请求（DESIGN.md §3.2.1）。
#[derive(Default, Clone)]
pub struct TrustPromptRegistry {
    pending: Arc<Mutex<HashMap<Uuid, oneshot::Sender<bool>>>>,
}

impl TrustPromptRegistry {
    pub async fn resolve(&self, request_id: Uuid, trust: bool) {
        if let Some(tx) = self.pending.lock().await.remove(&request_id) {
            let _ = tx.send(trust);
        }
    }

    async fn register(&self) -> (Uuid, oneshot::Receiver<bool>) {
        let (tx, rx) = oneshot::channel();
        let id = Uuid::new_v4();
        self.pending.lock().await.insert(id, tx);
        (id, rx)
    }
}

/// 主机指纹校验（DESIGN.md §3.2.1）：TOFU 首次信任 + 指纹变化告警，
/// 两种场景都要等前端弹窗给出用户的显式选择才能继续握手。
pub struct KnownHostsVerifier {
    repo: Arc<KnownHostsRepo>,
    prompts: TrustPromptRegistry,
    app_handle: AppHandle,
}

impl KnownHostsVerifier {
    pub fn new(repo: Arc<KnownHostsRepo>, prompts: TrustPromptRegistry, app_handle: AppHandle) -> Self {
        Self { repo, prompts, app_handle }
    }

    pub async fn verify(&self, host: &str, port: u16, fingerprint: &str) -> Result<bool, AppError> {
        match self.repo.lookup(host, port, fingerprint)? {
            KnownHostStatus::Match => Ok(true),
            KnownHostStatus::Mismatch(old_fingerprint) => {
                let trusted = self
                    .prompt(host, port, fingerprint, Some(old_fingerprint))
                    .await?;
                if trusted {
                    self.repo.save(host, port, fingerprint)?;
                }
                Ok(trusted)
            }
            KnownHostStatus::Unknown => {
                let trusted = self.prompt(host, port, fingerprint, None).await?;
                if trusted {
                    self.repo.save(host, port, fingerprint)?;
                }
                Ok(trusted)
            }
        }
    }

    async fn prompt(
        &self,
        host: &str,
        port: u16,
        fingerprint: &str,
        old_fingerprint: Option<String>,
    ) -> Result<bool, AppError> {
        let (request_id, rx) = self.prompts.register().await;
        self.app_handle
            .emit(
                "ssh:host-key-prompt",
                serde_json::json!({
                    "requestId": request_id,
                    "host": host,
                    "port": port,
                    "fingerprint": fingerprint,
                    "changed": old_fingerprint.is_some(),
                    "oldFingerprint": old_fingerprint,
                }),
            )
            .map_err(|e| AppError::Internal(e.to_string()))?;

        rx.await.map_err(|_| AppError::Internal("host key prompt cancelled".into()))
    }
}
