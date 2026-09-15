use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde_json::json;
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

use super::diff::generate_diff;
use super::git_ops;
use super::session::{ChangeStatus, CodingTarget, FileChange, FileSyncInfo};
use crate::agent::AgentConnectionPool;
use crate::error::AppError;
use crate::fsops::{FileOps, WriteOutcome};
use crate::ssh::SshConnectionPool;

/// 待落盘/已落盘文件改动的独立状态容器，和 `CodingSession` 里跑 AI 对话循环的
/// 部分分开加锁。
///
/// 原来 `changes`/`undo_stack` 是 `CodingSession` 的字段，和 `messages` 等对话状态
/// 共用外层同一把 `tokio::sync::Mutex`——`send_message` 处理一轮对话（可能连续
/// 调几十次工具，跑上一两分钟）期间会一直持有这把锁，用户这时候点某张已经弹出来
/// 的文件改动卡片的"应用/拒绝"，实际上是在排队等 `send_message` 整个函数返回才能
/// 拿到锁，界面上看起来就是"点了没反应"（2026-09 用户真实反馈：一次改 20 多个
/// 文件、逐个确认很繁琐，且中途点确认没反应）。拆成独立的锁之后，Accept/Reject/
/// Undo 系列操作不再需要等 AI 说完这一轮话——`commands::coding` 里那些命令直接从
/// `AppState.coding_changes`（和 `coding_sessions` 平级、同样以 workspace_id 为
/// key）拿这把锁，完全不经过 `CodingSession` 的锁。
pub struct ChangeStore {
    session_id: Uuid,
    workspace_root: String,
    target: CodingTarget,
    file_ops: Arc<dyn FileOps>,
    git_repo: bool,
    pub auto_git_commit: bool,
    /// "完全授权模式"的实时状态：开启后 AI 提出的文件改动直接落盘，并跳过普通
    /// 命令确认。原子值使开关能在 AI 任务运行中立即影响后续工具调用。
    pub full_auto: AtomicBool,
    /// "自动放行只读命令"的实时状态——同样的原因用原子值（不是 `CodingSession`
    /// 的普通字段）：`send_message` 处理一轮对话期间会一直持有 `CodingSession`
    /// 自己那把锁，普通字段的话用户在 AI 任务运行中点这个开关，实际是在排队等
    /// 当前这一轮说完才能拿到锁改成，界面上看起来"点了没反应"，跟 `full_auto`
    /// 2026-09 修过的是同一类问题（见上面 `full_auto` 和本结构体顶部的文档）。
    pub auto_allow_readonly: AtomicBool,
    changes: Vec<FileChange>,
    undo_stack: Vec<FileChange>,
}

impl ChangeStore {
    pub fn session_id(&self) -> Uuid {
        self.session_id
    }

    pub fn new(
        session_id: Uuid,
        workspace_root: String,
        target: CodingTarget,
        file_ops: Arc<dyn FileOps>,
        git_repo: bool,
    ) -> Self {
        Self {
            session_id,
            workspace_root,
            target,
            file_ops,
            git_repo,
            auto_git_commit: false,
            full_auto: AtomicBool::new(false),
            auto_allow_readonly: AtomicBool::new(false),
            changes: Vec::new(),
            undo_stack: Vec::new(),
        }
    }

    pub fn changes(&self) -> &[FileChange] {
        &self.changes
    }

    /// 供 `commands::coding::coding_set_auto_git_commit` 按需探测 Git 仓库用——
    /// 会话开始时不再预先探测（2026-09 用户要求："工作区不依赖 git，把这个探测
    /// 去掉"），只有用户真的去开这个开关时才需要知道 `target`/`workspace_root`
    /// 去跑一次 `git rev-parse`。
    pub fn target(&self) -> &CodingTarget {
        &self.target
    }

    pub fn git_repo(&self) -> bool {
        self.git_repo
    }

    pub fn workspace_root(&self) -> &str {
        &self.workspace_root
    }

    /// 按需探测结果写回来——探测本身在 `commands::coding` 那边做（要用
    /// `ssh_pool`/`agent_pool`，`ChangeStore` 不持有这两个池子）。
    pub fn set_git_repo(&mut self, git_repo: bool) {
        self.git_repo = git_repo;
    }

    /// 从持久化历史里灌回一份变更记录（`coding_start`/`coding_history_resume`
    /// 恢复会话时用），整体替换，不是增量合并。
    pub fn restore(&mut self, changes: Vec<FileChange>) {
        self.changes = changes;
    }

    /// 会话里已经存在的、尚未撤销的最新提议内容——同一轮对话里模型可能对同一个
    /// 文件连续 `write_file`/`edit_file` 好几次，后续的 `read_file` 不应该看到
    /// "过期"的磁盘内容。
    pub fn pending_content_for(&self, path: &str) -> Option<String> {
        self.changes
            .iter()
            .rev()
            .find(|c| {
                c.path == path
                    && c.status != ChangeStatus::Rejected
                    && c.status != ChangeStatus::Undone
            })
            .map(|c| c.new_content.clone())
    }

    async fn write_and_commit(
        &self,
        ssh_pool: &SshConnectionPool,
        agent_pool: &AgentConnectionPool,
        app_handle: &AppHandle,
        path: &str,
        content: &str,
        expected_mtime: Option<i64>,
    ) -> Result<i64, AppError> {
        let outcome = self
            .file_ops
            .write_file(path, content, expected_mtime)
            .await?;
        let mtime = match outcome {
            WriteOutcome::Written { mtime } => mtime,
            WriteOutcome::Conflict {
                current_mtime,
                current_preview,
            } => {
                return Err(AppError::Conflict(format!(
                    "文件 {path} 已被外部修改（当前 mtime：{current_mtime}），未覆盖磁盘内容。当前内容预览：{current_preview}"
                )));
            }
        };
        if self.auto_git_commit && self.git_repo {
            let message = format!("AI 编程助手：修改 {path}");
            let result = git_ops::commit_file(
                &self.target,
                &self.workspace_root,
                path,
                &message,
                ssh_pool,
                agent_pool,
            )
            .await;
            let output = match result {
                Ok(out) => out,
                Err(e) => format!("Git 提交失败：{e}"),
            };
            let _ = app_handle.emit(
                "coding:git-commit-result",
                json!({ "sessionId": self.session_id, "path": path, "output": output }),
            );
        }
        Ok(mtime)
    }

    /// `write_file`/`edit_file` 落到这里——`full_auto` 关闭时和原来行为一致，只
    /// 生成 Diff 等用户确认（返回的 `FileSyncInfo` 是 `None`）；打开时立即写盘并
    /// 返回同步信息，调用方（`CodingSession::stage_change`）负责把它一起通过
    /// `coding:file-change` 事件广播给前端，让已经打开的编辑器 buffer 刷新
    /// （否则完全授权模式下文件在磁盘上变了，编辑器里开着的 buffer 却不知道）。
    pub async fn stage(
        &mut self,
        path: &str,
        new_content: String,
        turn_id: Uuid,
        ssh_pool: &SshConnectionPool,
        agent_pool: &AgentConnectionPool,
        app_handle: &AppHandle,
    ) -> Result<(FileChange, Option<FileSyncInfo>), AppError> {
        let (old_content, expected_mtime) = match self.changes.iter().rev().find(|c| {
            c.path == path && c.status != ChangeStatus::Rejected && c.status != ChangeStatus::Undone
        }) {
            Some(change) => (change.new_content.clone(), change.expected_mtime),
            None => match self.file_ops.read_file(path).await {
                Ok(file) => (file.text, Some(file.mtime)),
                Err(_) => (String::new(), None),
            },
        };
        let diff = generate_diff(&old_content, &new_content);
        let id = Uuid::new_v4();
        let mut change = FileChange {
            id,
            path: path.to_string(),
            old_content,
            new_content: new_content.clone(),
            diff,
            status: ChangeStatus::Pending,
            expected_mtime,
            turn_id,
        };

        let sync = if self.full_auto.load(Ordering::Relaxed) {
            let mtime = self
                .write_and_commit(
                    ssh_pool,
                    agent_pool,
                    app_handle,
                    path,
                    &new_content,
                    expected_mtime,
                )
                .await?;
            change.status = ChangeStatus::Applied;
            change.expected_mtime = Some(mtime);
            self.undo_stack.clear();
            Some(FileSyncInfo {
                change_id: id,
                path: path.to_string(),
                content: new_content,
                mtime,
            })
        } else {
            None
        };

        self.changes.push(change.clone());
        Ok((change, sync))
    }

    /// 用户点击 FileChangeCard 的"应用"：这时才真正写盘（`expected_mtime` 传
    /// `None` 强制覆盖——用户已经在 UI 里显式确认过这份 Diff，不再需要走 mtime
    /// 冲突检测那一套，那是给"和别的编辑动作并发"场景设计的）。
    pub async fn accept(
        &mut self,
        change_id: Uuid,
        ssh_pool: &SshConnectionPool,
        agent_pool: &AgentConnectionPool,
        app_handle: &AppHandle,
    ) -> Result<FileSyncInfo, AppError> {
        let (path, content, expected_mtime) = {
            let change = self
                .changes
                .iter()
                .find(|c| c.id == change_id)
                .ok_or_else(|| AppError::NotFound(format!("change not found: {change_id}")))?;
            if change.status != ChangeStatus::Pending {
                return Err(AppError::Conflict(format!(
                    "change {change_id} is not pending"
                )));
            }
            (
                change.path.clone(),
                change.new_content.clone(),
                change.expected_mtime,
            )
        };
        let mtime = self
            .write_and_commit(
                ssh_pool,
                agent_pool,
                app_handle,
                &path,
                &content,
                expected_mtime,
            )
            .await?;
        if let Some(change) = self.changes.iter_mut().find(|c| c.id == change_id) {
            change.status = ChangeStatus::Applied;
            change.expected_mtime = Some(mtime);
        }
        self.undo_stack.clear();
        Ok(FileSyncInfo {
            change_id,
            path,
            content,
            mtime,
        })
    }

    pub fn reject(&mut self, change_id: Uuid) -> Result<(), AppError> {
        let change = self
            .changes
            .iter_mut()
            .find(|c| c.id == change_id)
            .ok_or_else(|| AppError::NotFound(format!("change not found: {change_id}")))?;
        if change.status != ChangeStatus::Pending {
            return Err(AppError::Conflict(format!(
                "change {change_id} is not pending"
            )));
        }
        change.status = ChangeStatus::Rejected;
        Ok(())
    }

    pub async fn undo(&mut self, change_id: Uuid) -> Result<FileSyncInfo, AppError> {
        let (path, old_content, expected_mtime) = {
            let change = self
                .changes
                .iter()
                .find(|c| c.id == change_id)
                .ok_or_else(|| AppError::NotFound(format!("change not found: {change_id}")))?;
            if change.status != ChangeStatus::Applied {
                return Err(AppError::Conflict(format!(
                    "change {change_id} is not applied"
                )));
            }
            (
                change.path.clone(),
                change.old_content.clone(),
                change.expected_mtime,
            )
        };
        let outcome = self
            .file_ops
            .write_file(&path, &old_content, expected_mtime)
            .await?;
        let mtime = match outcome {
            WriteOutcome::Written { mtime } => mtime,
            WriteOutcome::Conflict {
                current_mtime,
                current_preview,
            } => {
                return Err(AppError::Conflict(format!(
                    "文件 {path} 已被外部修改（当前 mtime：{current_mtime}），未执行撤销。当前内容预览：{current_preview}"
                )));
            }
        };
        let change = self
            .changes
            .iter_mut()
            .find(|c| c.id == change_id)
            .expect("checked above");
        change.status = ChangeStatus::Undone;
        change.expected_mtime = Some(mtime);
        self.undo_stack.push(change.clone());
        Ok(FileSyncInfo {
            change_id,
            path,
            content: old_content,
            mtime,
        })
    }

    /// 标准编辑器行为：撤销点之后一旦产生新的、真正落盘的修改（`accept`），原有
    /// redo 分支就失效，否则 redo 可能把已经被覆盖的旧内容写回（见 `accept` 里的
    /// `undo_stack.clear()`）。
    pub async fn redo(&mut self) -> Result<Option<FileSyncInfo>, AppError> {
        let Some(mut change) = self.undo_stack.last().cloned() else {
            return Ok(None);
        };
        let outcome = self
            .file_ops
            .write_file(&change.path, &change.new_content, change.expected_mtime)
            .await?;
        let mtime = match outcome {
            WriteOutcome::Written { mtime } => mtime,
            WriteOutcome::Conflict {
                current_mtime,
                current_preview,
            } => {
                return Err(AppError::Conflict(format!(
                    "文件 {} 已被外部修改（当前 mtime：{current_mtime}），未执行重做。当前内容预览：{current_preview}",
                    change.path
                )));
            }
        };
        self.undo_stack.pop();
        change.status = ChangeStatus::Applied;
        change.expected_mtime = Some(mtime);
        let id = change.id;
        let path = change.path.clone();
        let content = change.new_content.clone();
        if let Some(existing) = self.changes.iter_mut().find(|c| c.id == id) {
            *existing = change;
        }
        Ok(Some(FileSyncInfo {
            change_id: id,
            path,
            content,
            mtime,
        }))
    }

    /// 撤销某一轮对话里的全部已应用改动（不依赖 git）。按时间倒序逐个调用
    /// `undo`：同一轮里如果对同一个文件改了两次，必须先撤后面那次、再撤前面
    /// 那次，才能正确地一步步退回到这一轮开始之前的内容。
    pub async fn revert_turn(&mut self, turn_id: Uuid) -> Result<Vec<FileSyncInfo>, AppError> {
        let ids: Vec<Uuid> = self
            .changes
            .iter()
            .rev()
            .filter(|c| c.turn_id == turn_id && c.status == ChangeStatus::Applied)
            .map(|c| c.id)
            .collect();
        let mut results = Vec::with_capacity(ids.len());
        for id in ids {
            results.push(self.undo(id).await?);
        }
        Ok(results)
    }
}
