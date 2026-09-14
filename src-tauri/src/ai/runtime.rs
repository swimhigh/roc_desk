use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::error::AppError;

const GLOBAL_CONCURRENCY: usize = 4;
const PROVIDER_CONCURRENCY: usize = 2;

pub struct AiPermit {
    _global: OwnedSemaphorePermit,
    _provider: OwnedSemaphorePermit,
}

pub struct AiRuntime {
    client: reqwest::Client,
    global: Arc<Semaphore>,
    providers: Mutex<HashMap<Uuid, Arc<Semaphore>>>,
    chat_requests: Mutex<HashMap<Uuid, CancellationToken>>,
    coding_turns: Mutex<HashMap<Uuid, CancellationToken>>,
}

impl AiRuntime {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(120))
            .pool_idle_timeout(Duration::from_secs(60))
            .pool_max_idle_per_host(8)
            .build()
            .expect("failed to build shared AI HTTP client");
        Self {
            client,
            global: Arc::new(Semaphore::new(GLOBAL_CONCURRENCY)),
            providers: Mutex::new(HashMap::new()),
            chat_requests: Mutex::new(HashMap::new()),
            coding_turns: Mutex::new(HashMap::new()),
        }
    }

    pub fn client(&self) -> &reqwest::Client {
        &self.client
    }

    pub fn register_chat(&self, request_id: Uuid) -> CancellationToken {
        let token = CancellationToken::new();
        self.chat_requests
            .lock()
            .unwrap()
            .insert(request_id, token.clone());
        token
    }

    pub fn finish_chat(&self, request_id: Uuid) {
        self.chat_requests.lock().unwrap().remove(&request_id);
    }

    pub fn cancel_chat(&self, request_id: Uuid) -> bool {
        self.chat_requests
            .lock()
            .unwrap()
            .remove(&request_id)
            .is_some_and(|token| {
                token.cancel();
                true
            })
    }

    pub fn register_coding(&self, workspace_id: Uuid) -> Result<CancellationToken, AppError> {
        let mut turns = self.coding_turns.lock().unwrap();
        if turns.contains_key(&workspace_id) {
            return Err(AppError::Conflict("该工作区已有正在执行的 AI 轮次".into()));
        }
        let token = CancellationToken::new();
        turns.insert(workspace_id, token.clone());
        Ok(token)
    }

    pub fn finish_coding(&self, workspace_id: Uuid) {
        self.coding_turns.lock().unwrap().remove(&workspace_id);
    }

    pub fn cancel_coding(&self, workspace_id: Uuid) -> bool {
        self.coding_turns
            .lock()
            .unwrap()
            .remove(&workspace_id)
            .is_some_and(|token| {
                token.cancel();
                true
            })
    }

    pub async fn acquire(
        &self,
        provider_id: Uuid,
        cancellation: &CancellationToken,
    ) -> Result<AiPermit, AppError> {
        let provider = self
            .providers
            .lock()
            .unwrap()
            .entry(provider_id)
            .or_insert_with(|| Arc::new(Semaphore::new(PROVIDER_CONCURRENCY)))
            .clone();
        let global = tokio::select! {
            _ = cancellation.cancelled() => return Err(cancelled()),
            permit = self.global.clone().acquire_owned() => permit.map_err(|_| AppError::Internal("AI 全局并发控制器已关闭".into()))?,
        };
        let provider = tokio::select! {
            _ = cancellation.cancelled() => return Err(cancelled()),
            permit = provider.acquire_owned() => permit.map_err(|_| AppError::Internal("Provider 并发控制器已关闭".into()))?,
        };
        Ok(AiPermit {
            _global: global,
            _provider: provider,
        })
    }
}

pub fn cancelled() -> AppError {
    AppError::Connection("请求已取消".into())
}
