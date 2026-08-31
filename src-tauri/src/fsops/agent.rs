//! Agent 文件操作（AGENT_DESIGN.md §四.3）：第三个 `FileOps` 实现，和
//! `local.rs`/`remote.rs` 同级。所有方法都是"发 Request、匹配 Response"的直译，
//! 真正的文件系统访问发生在远程 Agent 进程本机（`agent/src/handlers/fs.rs`）。

use async_trait::async_trait;
use roc_desk_protocol::{Request, Response, ResponseBody};
use std::sync::Arc;

use super::{FileEntry, FileOps, WriteOutcome};
use crate::agent::session::{AgentSession, StreamFrame};
use crate::error::AppError;

pub struct AgentFileOps {
    session: Arc<AgentSession>,
}

impl AgentFileOps {
    pub fn new(session: Arc<AgentSession>) -> Self {
        Self { session }
    }
}

impl From<roc_desk_protocol::FileEntry> for FileEntry {
    fn from(e: roc_desk_protocol::FileEntry) -> Self {
        FileEntry { name: e.name, path: e.path, is_dir: e.is_dir, size: e.size, modified: e.modified }
    }
}

fn unexpected(response: Response) -> AppError {
    match response {
        Response::Error { message, .. } => AppError::Internal(message),
        Response::Conflict { .. } => AppError::Internal("此处不应该收到 Conflict 响应".into()),
        Response::Ok(_) => AppError::Internal("Agent 返回了意外的响应类型".into()),
    }
}

#[async_trait]
impl FileOps for AgentFileOps {
    async fn list_dir(&self, path: &str) -> Result<Vec<FileEntry>, AppError> {
        match self.session.request(Request::ListDir { path: path.to_string() }).await? {
            Response::Ok(ResponseBody::Entries(entries)) => Ok(entries.into_iter().map(Into::into).collect()),
            other => Err(unexpected(other)),
        }
    }

    async fn read_file_raw(&self, path: &str) -> Result<(Vec<u8>, i64), AppError> {
        read_streamed(&self.session, Request::ReadFile { path: path.to_string() }).await
    }

    async fn file_size(&self, path: &str) -> Result<u64, AppError> {
        match self.session.request(Request::Stat { path: path.to_string() }).await? {
            Response::Ok(ResponseBody::FileMeta { size, .. }) => Ok(size),
            other => Err(unexpected(other)),
        }
    }

    async fn read_file_raw_bounded(&self, path: &str, max_bytes: u64) -> Result<(Vec<u8>, i64), AppError> {
        read_streamed(&self.session, Request::ReadFileBounded { path: path.to_string(), max_bytes }).await
    }

    async fn write_file_bytes(&self, path: &str, bytes: &[u8], expected_mtime: Option<i64>) -> Result<WriteOutcome, AppError> {
        let request = Request::WriteFile { path: path.to_string(), expected_mtime };
        match self.session.write_stream(request, bytes).await? {
            Response::Ok(ResponseBody::Written { mtime }) => Ok(WriteOutcome::Written { mtime }),
            Response::Conflict { current_mtime, current_preview } => Ok(WriteOutcome::Conflict { current_mtime, current_preview }),
            other => Err(unexpected(other)),
        }
    }

    async fn delete(&self, path: &str, is_dir: bool) -> Result<(), AppError> {
        match self.session.request(Request::Delete { path: path.to_string(), is_dir }).await? {
            Response::Ok(_) => Ok(()),
            other => Err(unexpected(other)),
        }
    }

    async fn rename(&self, from: &str, to: &str) -> Result<(), AppError> {
        match self.session.request(Request::Rename { from: from.to_string(), to: to.to_string() }).await? {
            Response::Ok(_) => Ok(()),
            other => Err(unexpected(other)),
        }
    }

    async fn create_dir(&self, path: &str) -> Result<(), AppError> {
        match self.session.request(Request::CreateDir { path: path.to_string() }).await? {
            Response::Ok(_) => Ok(()),
            other => Err(unexpected(other)),
        }
    }
}

/// `ReadFile`/`ReadFileBounded` 的流式响应装配：先等一个 `FileMeta`（拿 mtime），
/// 再攒 `DataChunk`，`StreamEnd` 之后返回——和 Agent 侧 `server.rs::handle_read_file`
/// 的发送顺序严格对应。
async fn read_streamed(session: &AgentSession, request: Request) -> Result<(Vec<u8>, i64), AppError> {
    let mut rx = session.request_streamed(request).await?;
    let mut mtime = 0i64;
    let mut buf = Vec::new();
    while let Some(item) = rx.recv().await {
        match item {
            StreamFrame::Control(Response::Ok(ResponseBody::FileMeta { mtime: m, .. })) => mtime = m,
            StreamFrame::Control(other) => return Err(unexpected(other)),
            StreamFrame::Data(bytes) => buf.extend_from_slice(&bytes),
            StreamFrame::End => break,
        }
    }
    Ok((buf, mtime))
}
