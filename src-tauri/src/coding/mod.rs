pub mod changes;
pub mod diff;
pub mod git_ops;
pub mod guard;
pub mod permission;
pub mod session;
pub mod skills;
pub mod tools;
pub mod webfetch;

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{oneshot, Mutex};
use uuid::Uuid;

pub use changes::ChangeStore;
pub use session::{
    ChangeStatus, ChatAttachment, CodingMode, CodingSession, CodingTarget, FileChange, FileSyncInfo,
};

/// 等待前端响应的 `run_command` 确认请求（DESIGN.md §3.8.2.1），和
/// `ssh::known_hosts::TrustPromptRegistry` 是同一套 oneshot 模式，分开建一个类型
/// 是因为两者语义上是不同的用户决策（信任主机指纹 vs 允许执行一条命令），
/// 混用同一个注册表容易在两处调用之间搞混 requestId 的语义。同一套模式也用来
/// 门禁 MCP 工具调用（`ToolCall::Mcp`）——语义仍然是"允许执行一次带副作用的动作"，
/// 不需要再单独建一个注册表。
#[derive(Default, Clone)]
pub struct CommandConfirmRegistry {
    pending: Arc<Mutex<HashMap<Uuid, (Uuid, oneshot::Sender<bool>)>>>,
}

impl CommandConfirmRegistry {
    pub async fn register(&self, session_id: Uuid) -> (Uuid, oneshot::Receiver<bool>) {
        let (tx, rx) = oneshot::channel();
        let id = Uuid::new_v4();
        self.pending.lock().await.insert(id, (session_id, tx));
        (id, rx)
    }

    pub async fn resolve(&self, request_id: Uuid, allow: bool) {
        if let Some((_, tx)) = self.pending.lock().await.remove(&request_id) {
            let _ = tx.send(allow);
        }
    }

    /// 完全授权模式在命令确认已弹出后才打开时，立即放行同一会话仍在等待的请求。
    pub async fn allow_session(&self, session_id: Uuid) {
        let pending = {
            let mut pending = self.pending.lock().await;
            let request_ids = pending
                .iter()
                .filter_map(|(request_id, (pending_session_id, _))| {
                    (*pending_session_id == session_id).then_some(*request_id)
                })
                .collect::<Vec<_>>();
            request_ids
                .into_iter()
                .filter_map(|request_id| pending.remove(&request_id).map(|(_, tx)| tx))
                .collect::<Vec<_>>()
        };
        for tx in pending {
            let _ = tx.send(true);
        }
    }
}

/// `question` 工具的等待注册表：结构和 `CommandConfirmRegistry` 几乎一样，区别
/// 是这里等的是用户填的一段文本回答，不是一个 bool（见 `coding/tools.rs` 的
/// `ToolCall::AskUser`）。分开建一个类型而不是把 `CommandConfirmRegistry` 泛型化，
/// 是因为两者面向的前端弹窗、事件名、语义都不同，泛型化省下的代码量不值得
/// 增加的间接层。
#[derive(Default, Clone)]
pub struct QuestionRegistry {
    pending: Arc<Mutex<HashMap<Uuid, oneshot::Sender<String>>>>,
}

impl QuestionRegistry {
    pub async fn register(&self) -> (Uuid, oneshot::Receiver<String>) {
        let (tx, rx) = oneshot::channel();
        let id = Uuid::new_v4();
        self.pending.lock().await.insert(id, tx);
        (id, rx)
    }

    pub async fn resolve(&self, request_id: Uuid, answer: String) {
        if let Some(tx) = self.pending.lock().await.remove(&request_id) {
            let _ = tx.send(answer);
        }
    }
}
