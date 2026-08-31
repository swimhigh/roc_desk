//! TLS 证书管理（AGENT_DESIGN.md §3.1）：首次启动没有证书就用 `rcgen` 生成一份
//! 自签名证书 + 私钥，落盘后固定复用。证书本身不需要被任何公共 CA 信任——客户端
//! 校验的是这个具体证书的 SHA-256 指纹（TOFU 模型，与 SSH `known_hosts` 完全对称），
//! 不做标准证书链校验。

use std::path::Path;

use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
use sha2::{Digest, Sha256};

pub struct AgentCert {
    pub cert_der: CertificateDer<'static>,
    pub key_der: PrivatePkcs8KeyDer<'static>,
    /// 十六进制、大写、冒号分隔（`AA:BB:...`），和 SSH 主机指纹展示风格一致。
    pub fingerprint_sha256: String,
}

fn fingerprint_of(cert_der: &[u8]) -> String {
    let digest = Sha256::digest(cert_der);
    digest.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(":")
}

/// `dir` 下没有 `cert.pem`/`key.pem` 就生成一份新的并落盘；已存在则直接加载复用
/// （重启 Agent 不应该让所有已配对客户端的指纹信任全部失效）。
pub fn load_or_generate(dir: &Path) -> std::io::Result<AgentCert> {
    let cert_path = dir.join("cert.pem");
    let key_path = dir.join("key.pem");

    if cert_path.exists() && key_path.exists() {
        let cert_pem = std::fs::read_to_string(&cert_path)?;
        let key_pem = std::fs::read_to_string(&key_path)?;
        let cert_der: CertificateDer<'static> = rustls_pemfile::certs(&mut cert_pem.as_bytes())
            .next()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "cert.pem 中没有证书"))??;
        let key_der: PrivatePkcs8KeyDer<'static> = rustls_pemfile::pkcs8_private_keys(&mut key_pem.as_bytes())
            .next()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "key.pem 中没有私钥"))??;
        let fingerprint_sha256 = fingerprint_of(cert_der.as_ref());
        return Ok(AgentCert { cert_der, key_der, fingerprint_sha256 });
    }

    let subject_alt_names = vec!["roc-desk-agent".to_string()];
    let generated = rcgen::generate_simple_self_signed(subject_alt_names)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("生成自签名证书失败: {e}")))?;
    let cert_pem = generated.cert.pem();
    let key_pem = generated.key_pair.serialize_pem();
    std::fs::write(&cert_path, &cert_pem)?;
    std::fs::write(&key_path, &key_pem)?;

    let cert_der: CertificateDer<'static> = generated.cert.der().clone();
    let key_der = PrivatePkcs8KeyDer::from(generated.key_pair.serialize_der());
    let fingerprint_sha256 = fingerprint_of(cert_der.as_ref());
    Ok(AgentCert { cert_der, key_der, fingerprint_sha256 })
}
