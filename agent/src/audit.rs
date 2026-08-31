//! Agent 侧独立审计日志（AGENT_DESIGN.md §五）：防御纵深的第二层——即便客户端本身
//! 被篡改，服务端这份日志仍然可信。滚动追加写入一个 JSON Lines 文本文件，不用数据库
//! （审计量不大，不值得为此引入 rusqlite 依赖）。

use std::path::PathBuf;

use serde::Serialize;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

#[derive(Serialize)]
struct AuditEntry<'a> {
    at: String,
    event: &'a str,
    peer: &'a str,
    detail: &'a str,
}

pub struct AuditLog {
    path: PathBuf,
    lock: Mutex<()>,
}

impl AuditLog {
    pub fn new(dir: &std::path::Path) -> Self {
        Self { path: dir.join("audit.log"), lock: Mutex::new(()) }
    }

    pub async fn record(&self, event: &str, peer: &str, detail: &str) {
        let entry = AuditEntry { at: chrono::Utc::now().to_rfc3339(), event, peer, detail };
        let Ok(mut line) = serde_json::to_vec(&entry) else { return };
        line.push(b'\n');

        let _guard = self.lock.lock().await;
        let file = tokio::fs::OpenOptions::new().create(true).append(true).open(&self.path).await;
        if let Ok(mut file) = file {
            let _ = file.write_all(&line).await;
        }
    }
}
