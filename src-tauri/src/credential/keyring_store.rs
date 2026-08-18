use async_trait::async_trait;
use keyring::Entry;

use super::CredentialStore;
use crate::error::AppError;

const SERVICE_NAME: &str = "devhub";

/// 系统密钥链实现（Windows Credential Manager / macOS Keychain / Linux Secret Service）。
pub struct KeyringStore;

fn entry_for(key: &str) -> Result<Entry, AppError> {
    Entry::new(SERVICE_NAME, key).map_err(|e| AppError::Internal(format!("keyring entry: {e}")))
}

#[async_trait]
impl CredentialStore for KeyringStore {
    async fn set(&self, key: &str, secret: &str) -> Result<(), AppError> {
        let key = key.to_string();
        let secret = secret.to_string();
        tokio::task::spawn_blocking(move || {
            entry_for(&key)?
                .set_password(&secret)
                .map_err(|e| AppError::Internal(format!("keyring set: {e}")))
        })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
    }

    async fn get(&self, key: &str) -> Result<Option<String>, AppError> {
        let key = key.to_string();
        tokio::task::spawn_blocking(move || match entry_for(&key)?.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(AppError::Internal(format!("keyring get: {e}"))),
        })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
    }

    async fn delete(&self, key: &str) -> Result<(), AppError> {
        let key = key.to_string();
        tokio::task::spawn_blocking(move || match entry_for(&key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(AppError::Internal(format!("keyring delete: {e}"))),
        })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
    }
}