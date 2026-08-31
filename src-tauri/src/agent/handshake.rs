//! TLS 握手 + 证书指纹 TOFU 校验（AGENT_DESIGN.md §3.1），与 `ssh::known_hosts`
//! 完全对称，但校验时机不同——rustls 的 `ServerCertVerifier` 是同步回调，没法在
//! 里面 `.await` 一个"等前端弹窗确认"的 oneshot。所以这里分两步：
//! 1. `TofuCertCapture`（同步）在 TLS 握手阶段无条件放行证书，只是把它的原始字节
//!    记下来——TLS 加密通道本身照常建立（证明对方确实持有匹配的私钥），只是还没有
//!    判断"这个身份要不要信任"。
//! 2. 握手完成后，`AgentCertVerifier::verify`（异步）用捕获到的指纹去查
//!    `agent_known_hosts` 表，未知/变化时弹窗等待用户确认——和 `KnownHostsVerifier`
//!    的 `verify`/`prompt` 一模一样的模式。不信任就直接丢弃这条已经建立的连接，
//!    不会有任何应用层数据（含 `Handshake` 请求本身）被发送出去。

use std::sync::{Arc, Mutex as StdMutex};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, Error as TlsError, SignatureScheme};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter};
use tokio::sync::{oneshot, Mutex};
use uuid::Uuid;

use crate::db::repo::agent_known_hosts_repo::{AgentKnownHostStatus, AgentKnownHostsRepo};
use crate::error::AppError;

/// 和 `ssh::known_hosts::TrustPromptRegistry` 是同一套 oneshot 模式，语义上是
/// 不同的用户决策（信任 Agent 证书指纹 vs 信任 SSH 主机公钥），按既有约定分开建类型。
#[derive(Default, Clone)]
pub struct AgentTrustPromptRegistry {
    pending: Arc<Mutex<std::collections::HashMap<Uuid, oneshot::Sender<bool>>>>,
}

impl AgentTrustPromptRegistry {
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

/// 握手阶段的证书捕获——无条件放行（不做链校验、不校验签名），只记录 DER 字节。
/// 安全性建立在"握手完成后立即做指纹 TOFU 判断，不信任就整条连接直接丢弃、
/// 一个字节的应用层数据都不发"这个前提上，见模块文档。
#[derive(Debug)]
pub struct TofuCertCapture {
    pub captured_der: StdMutex<Option<Vec<u8>>>,
}

impl TofuCertCapture {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { captured_der: StdMutex::new(None) })
    }

    pub fn take_fingerprint_sha256(&self) -> Option<String> {
        let der = self.captured_der.lock().unwrap().clone()?;
        let digest = Sha256::digest(&der);
        Some(digest.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(":"))
    }
}

impl ServerCertVerifier for TofuCertCapture {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, TlsError> {
        *self.captured_der.lock().unwrap() = Some(end_entity.as_ref().to_vec());
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(&self, _message: &[u8], _cert: &CertificateDer<'_>, _dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, TlsError> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(&self, _message: &[u8], _cert: &CertificateDer<'_>, _dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, TlsError> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![
            SignatureScheme::RSA_PKCS1_SHA256,
            SignatureScheme::RSA_PKCS1_SHA384,
            SignatureScheme::RSA_PKCS1_SHA512,
            SignatureScheme::ECDSA_NISTP256_SHA256,
            SignatureScheme::ECDSA_NISTP384_SHA384,
            SignatureScheme::ECDSA_NISTP521_SHA512,
            SignatureScheme::RSA_PSS_SHA256,
            SignatureScheme::RSA_PSS_SHA384,
            SignatureScheme::RSA_PSS_SHA512,
            SignatureScheme::ED25519,
        ]
    }
}

/// 握手完成后的指纹 TOFU 判断，和 `ssh::known_hosts::KnownHostsVerifier` 对称。
pub struct AgentCertVerifier {
    repo: Arc<AgentKnownHostsRepo>,
    prompts: AgentTrustPromptRegistry,
    app_handle: AppHandle,
}

impl AgentCertVerifier {
    pub fn new(repo: Arc<AgentKnownHostsRepo>, prompts: AgentTrustPromptRegistry, app_handle: AppHandle) -> Self {
        Self { repo, prompts, app_handle }
    }

    pub async fn verify(&self, connection_id: Uuid, host: &str, port: u16, fingerprint: &str) -> Result<bool, AppError> {
        match self.repo.lookup(connection_id, fingerprint)? {
            AgentKnownHostStatus::Match => Ok(true),
            AgentKnownHostStatus::Mismatch(old) => {
                let trusted = self.prompt(connection_id, host, port, fingerprint, Some(old)).await?;
                if trusted {
                    self.repo.save(connection_id, fingerprint)?;
                }
                Ok(trusted)
            }
            AgentKnownHostStatus::Unknown => {
                let trusted = self.prompt(connection_id, host, port, fingerprint, None).await?;
                if trusted {
                    self.repo.save(connection_id, fingerprint)?;
                }
                Ok(trusted)
            }
        }
    }

    async fn prompt(&self, connection_id: Uuid, host: &str, port: u16, fingerprint: &str, old_fingerprint: Option<String>) -> Result<bool, AppError> {
        let (request_id, rx) = self.prompts.register().await;
        self.app_handle
            .emit(
                "agent:cert-prompt",
                serde_json::json!({
                    "requestId": request_id,
                    "connectionId": connection_id,
                    "host": host,
                    "port": port,
                    "fingerprint": fingerprint,
                    "changed": old_fingerprint.is_some(),
                    "oldFingerprint": old_fingerprint,
                }),
            )
            .map_err(|e| AppError::Internal(e.to_string()))?;
        rx.await.map_err(|_| AppError::Internal("agent cert prompt cancelled".into()))
    }
}
