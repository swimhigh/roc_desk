pub mod diff;
pub mod git_ops;
pub mod guard;
pub mod session;
pub mod tools;

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{oneshot, Mutex};
use uuid::Uuid;

pub use session::{ChangeStatus, CodingMode, CodingSession, CodingTarget, FileChange};

/// 等待前端响应的 `run_command` 确认请求（DESIGN.md §3.8.2.1），和
/// `ssh::known_hosts::TrustPromptRegistry` 是同一套 oneshot 模式，分开建一个类型
/// 是因为两者语义上是不同的用户决策（信任主机指纹 vs 允许执行一条命令），
/// 混用同一个注册表容易在两处调用之间搞混 requestId 的语义。
#[derive(Default, Clone)]
pub struct CommandConfirmRegistry {
    pending: Arc<Mutex<HashMap<Uuid, oneshot::Sender<bool>>>>,
}

impl CommandConfirmRegistry {
    pub async fn register(&self) -> (Uuid, oneshot::Receiver<bool>) {
        let (tx, rx) = oneshot::channel();
        let id = Uuid::new_v4();
        self.pending.lock().await.insert(id, tx);
        (id, rx)
    }

    pub async fn resolve(&self, request_id: Uuid, allow: bool) {
        if let Some(tx) = self.pending.lock().await.remove(&request_id) {
            let _ = tx.send(allow);
        }
    }
}
