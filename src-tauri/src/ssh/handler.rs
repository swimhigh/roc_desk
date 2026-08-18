use std::sync::Arc;

use async_trait::async_trait;
use russh::keys::key;

use super::known_hosts::KnownHostsVerifier;

/// `russh::client::Handler` 实现（DESIGN.md §3.2.1）。
/// 默认不做任何校验，必须显式实现 `check_server_key`，否则对 MITM 无防护。
pub struct SshHandler {
    pub host: String,
    pub port: u16,
    pub verifier: Arc<KnownHostsVerifier>,
}

#[async_trait]
impl russh::client::Handler for SshHandler {
    type Error = russh::Error;

    async fn check_server_key(&mut self, server_public_key: &key::PublicKey) -> Result<bool, Self::Error> {
        let fingerprint = server_public_key.fingerprint();
        self.verifier
            .verify(&self.host, self.port, &fingerprint)
            .await
            .map_err(|e| russh::Error::IO(std::io::Error::other(e.to_string())))
    }
}
