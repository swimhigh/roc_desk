pub mod keyring_store;

use async_trait::async_trait;

use crate::error::AppError;

/// 凭据存储抽象（CODE_DESIGN.md §3.3）。
///
/// SSH 密码/私钥口令、AI API Key 一律走这个 trait，不落库明文
/// （DESIGN.md §八-1：`tauri-plugin-store` 是明文 JSON，不能用来存机密）。
#[async_trait]
pub trait CredentialStore: Send + Sync {
    async fn set(&self, key: &str, secret: &str) -> Result<(), AppError>;
    async fn get(&self, key: &str) -> Result<Option<String>, AppError>;
    async fn delete(&self, key: &str) -> Result<(), AppError>;
}

pub use keyring_store::KeyringStore;
