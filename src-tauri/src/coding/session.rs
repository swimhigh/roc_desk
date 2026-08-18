use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

use super::diff::{generate_diff, DiffLine};
use super::git_ops;
use super::guard;
use super::tools::{self, ToolCall};
use super::CommandConfirmRegistry;
use crate::ai::AiProviderManager;
use crate::db::repo::audit_log_repo::AuditLogRepo;
use crate::error::AppError;
use crate::fsops::FileOps;
use crate::ssh::SshConnectionPool;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingMode {
    Plan,
    Build,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum CodingTarget {
    Local,
    Remote { connection_id: Uuid, host_label: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeStatus {
    Pending,
    Applied,
    Rejected,
    Undone,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileChange {
    pub id: Uuid,
    pub path: String,
    pub old_content: String,
    pub new_content: String,
    pub diff: Vec<DiffLine>,
    pub status: ChangeStatus,
}

/// AI 编程助手会话（DESIGN.md §3.8.3）：自动绑定某个已打开的工作区，一个进程内
/// 每个工作区最多一个活跃会话（CODE_DESIGN.md 里没有多会话并发的需求）。
///
/// 文件改动走"生成 Diff 立即可见 + 落盘延后到用户 Accept"的流程（对应
/// FileChangeCard.tsx 的 pending/applied/rejected 三态和"用户可逐条 Accept/Reject"
/// 的文案），而不是像骨架代码那样立即写盘再靠 Undo 补救——AI 编程助手的核心场景是
/// 触碰生产服务器，"改完再后悔"的代价比"多点一次确认"高得多。为了不让同一轮对话
/// 里后续的 read_file 看到"过期"的内容，`pending_content_for` 会优先返回未落盘的
/// 提议内容，让模型的推理和已经生成的 Diff 保持一致。
pub struct CodingSession {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub workspace_root: String,
    pub target: CodingTarget,
    pub mode: CodingMode,
    pub provider_id: Uuid,
    pub auto_allow_readonly: bool,
    /// 工作区根目录是否在一个 Git 仓库里——`coding_start` 命令构造完 session 后
    /// 探测一次填进来（探测本身要跑一条命令，`new()` 保持同步，不在这里做）。
    /// 不是仓库时前端应该把"自动提交"开关直接 disable 掉。
    pub git_repo: bool,
    /// 用户是否要求"每次 Accept 一条变更就自动 git add + commit"（DESIGN.md §3.8.2）。
    pub auto_git_commit: bool,
    pub changes: Vec<FileChange>,
    undo_stack: Vec<FileChange>,
    messages: Vec<serde_json::Value>,
    file_ops: Arc<dyn FileOps>,
}

const MAX_TOOL_ITERATIONS: usize = 8;

impl CodingSession {
    pub fn new(
        workspace_id: Uuid,
        workspace_root: String,
        target: CodingTarget,
        provider_id: Uuid,
        file_ops: Arc<dyn FileOps>,
    ) -> Self {
        let target_desc = match &target {
            CodingTarget::Local => "本地工作区".to_string(),
            CodingTarget::Remote { host_label, .. } => format!("远程主机 {host_label}"),
        };
        let system_prompt = format!(
            "你是集成在 roc_desk 桌面工具里的 AI 编程助手，当前绑定的工作区根目录是 `{workspace_root}`（{target_desc}）。\
             你可以用提供的工具读写文件、搜索代码、执行命令。write_file/edit_file 产生的改动不会立即生效，\
             而是生成 Diff 交给用户确认，所以你可以放心连续提出多个改动，不需要等待每一步都被确认才能继续推理。\
             run_command 有安全限制：破坏性命令会被直接拦截，其余命令需要用户在弹窗里确认才会真正执行。",
        );
        Self {
            id: Uuid::new_v4(),
            workspace_id,
            workspace_root,
            target,
            mode: CodingMode::Plan,
            provider_id,
            auto_allow_readonly: false,
            git_repo: false,
            auto_git_commit: false,
            changes: Vec::new(),
            undo_stack: Vec::new(),
            messages: vec![json!({ "role": "system", "content": system_prompt })],
            file_ops,
        }
    }

    fn target_label(&self) -> String {
        match &self.target {
            CodingTarget::Local => "本地".to_string(),
            CodingTarget::Remote { host_label, .. } => host_label.clone(),
        }
    }

    /// 会话里已经存在的、尚未撤销的最新提议内容（见结构体文档）。
    fn pending_content_for(&self, path: &str) -> Option<String> {
        self.changes
            .iter()
            .rev()
            .find(|c| c.path == path && c.status != ChangeStatus::Rejected && c.status != ChangeStatus::Undone)
            .map(|c| c.new_content.clone())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn send_message(
        &mut self,
        user_text: &str,
        providers: &AiProviderManager,
        ssh_pool: &SshConnectionPool,
        audit: &AuditLogRepo,
        confirms: &CommandConfirmRegistry,
        app_handle: &AppHandle,
    ) -> Result<String, AppError> {
        self.messages.push(json!({ "role": "user", "content": user_text }));

        let provider = providers
            .get(self.provider_id)?
            .ok_or_else(|| AppError::NotFound(format!("ai provider not found: {}", self.provider_id)))?;
        let api_key = providers.resolve_api_key(&provider).await?;
        let client = reqwest::Client::new();

        for _ in 0..MAX_TOOL_ITERATIONS {
            let url = format!("{}/chat/completions", provider.api_base.trim_end_matches('/'));
            let mut req = client.post(&url).json(&json!({
                "model": provider.model,
                "messages": self.messages,
                "tools": tools_for_mode(self.mode),
                "tool_choice": "auto",
            }));
            if let Some(key) = &api_key {
                req = req.bearer_auth(key);
            }
            let resp = req.send().await?;
            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(AppError::Connection(format!("HTTP {status}: {body}")));
            }
            let body: serde_json::Value = resp.json().await?;
            let message = body["choices"][0]["message"].clone();
            let tool_calls = message["tool_calls"].as_array().cloned().unwrap_or_default();

            if tool_calls.is_empty() {
                let text = message["content"].as_str().unwrap_or("").to_string();
                self.messages.push(json!({ "role": "assistant", "content": text }));
                return Ok(text);
            }

            self.messages.push(message);
            for call in &tool_calls {
                let call_id = call["id"].as_str().unwrap_or_default().to_string();
                let fn_name = call["function"]["name"].as_str().unwrap_or_default().to_string();
                let fn_args = call["function"]["arguments"].as_str().unwrap_or("{}");

                let _ = app_handle.emit(
                    "coding:tool-call-start",
                    json!({ "sessionId": self.id, "tool": fn_name }),
                );

                let result_text = match tools::parse_tool_call(&fn_name, fn_args) {
                    Ok(call) => self
                        .execute_tool(call, ssh_pool, audit, confirms, app_handle)
                        .await
                        .unwrap_or_else(|e| format!("工具执行出错：{e}")),
                    Err(e) => format!("工具调用参数解析失败：{e}"),
                };

                let _ = app_handle.emit(
                    "coding:tool-call-end",
                    json!({ "sessionId": self.id, "tool": fn_name }),
                );

                self.messages.push(json!({
                    "role": "tool",
                    "tool_call_id": call_id,
                    "content": result_text,
                }));
            }
        }

        Err(AppError::Internal("工具调用循环次数超限，可能陷入死循环，已中止本轮对话".into()))
    }

    async fn execute_tool(
        &mut self,
        call: ToolCall,
        ssh_pool: &SshConnectionPool,
        audit: &AuditLogRepo,
        confirms: &CommandConfirmRegistry,
        app_handle: &AppHandle,
    ) -> Result<String, AppError> {
        match call {
            ToolCall::ReadFile { path } => {
                if let Some(content) = self.pending_content_for(&path) {
                    return Ok(content);
                }
                Ok(self.file_ops.read_file(&path).await?.text)
            }
            ToolCall::ListDirectory { path } => {
                let entries = self.file_ops.list_dir(&path).await?;
                Ok(serde_json::to_string(&entries).unwrap_or_default())
            }
            ToolCall::SearchFiles { pattern, path } => self.search_files(&pattern, &path, ssh_pool).await,
            ToolCall::WriteFile { path, content } => self.stage_change(&path, content, app_handle).await,
            ToolCall::EditFile { path, old_text, new_text } => {
                let original = match self.pending_content_for(&path) {
                    Some(c) => c,
                    None => self.file_ops.read_file(&path).await?.text,
                };
                if !original.contains(&old_text) {
                    return Err(AppError::Internal(format!(
                        "edit_file 失败：在 {path} 中没有找到匹配的 old_text，请先用 read_file 确认现有内容"
                    )));
                }
                let updated = original.replacen(&old_text, &new_text, 1);
                self.stage_change(&path, updated, app_handle).await
            }
            ToolCall::RunCommand { command } => {
                self.run_command_gated(&command, ssh_pool, audit, confirms, app_handle).await
            }
        }
    }

    async fn search_files(&self, pattern: &str, path: &str, ssh_pool: &SshConnectionPool) -> Result<String, AppError> {
        match &self.target {
            CodingTarget::Local => {
                let results = tools::search_files_local(std::path::Path::new(path), pattern, 50);
                Ok(if results.is_empty() { "没有匹配结果".to_string() } else { results.join("\n") })
            }
            CodingTarget::Remote { connection_id, .. } => {
                let session = ssh_pool.get_or_connect(*connection_id).await?;
                let quoted_pattern = crate::log::remote::shell_quote(pattern);
                let quoted_path = crate::log::remote::shell_quote(path);
                let cmd = format!(
                    "rg -n -F -- {quoted_pattern} {quoted_path} 2>/dev/null || grep -rn -F -- {quoted_pattern} {quoted_path} 2>/dev/null"
                );
                session.exec(&cmd).await
            }
        }
    }

    async fn stage_change(&mut self, path: &str, new_content: String, app_handle: &AppHandle) -> Result<String, AppError> {
        let old_content = match self.pending_content_for(path) {
            Some(c) => c,
            None => self.file_ops.read_file(path).await.map(|f| f.text).unwrap_or_default(),
        };
        let diff = generate_diff(&old_content, &new_content);
        let id = Uuid::new_v4();
        let change = FileChange {
            id,
            path: path.to_string(),
            old_content,
            new_content,
            diff,
            status: ChangeStatus::Pending,
        };
        let _ = app_handle.emit("coding:file-change", json!({ "sessionId": self.id, "change": &change }));
        self.changes.push(change);
        Ok(format!("已为 {path} 生成变更（id={id}），已在界面展示 Diff，等待用户 Accept 后才会真正写入磁盘。"))
    }

    async fn run_command_gated(
        &mut self,
        command: &str,
        ssh_pool: &SshConnectionPool,
        audit: &AuditLogRepo,
        confirms: &CommandConfirmRegistry,
        app_handle: &AppHandle,
    ) -> Result<String, AppError> {
        let target_label = self.target_label();

        if guard::is_blacklisted(command) {
            audit.record(self.id, &target_label, command, "blocked", None);
            let _ = app_handle.emit("coding:command-blocked", json!({ "sessionId": self.id, "command": command }));
            return Ok(format!("已拦截高危命令：{command}，如需执行请前往终端模块手动操作"));
        }

        let allowed = if self.auto_allow_readonly && guard::is_whitelisted(command) {
            true
        } else {
            let (request_id, rx) = confirms.register().await;
            let is_remote = matches!(&self.target, CodingTarget::Remote { .. });
            let _ = app_handle.emit(
                "coding:command-confirm-request",
                json!({
                    "sessionId": self.id,
                    "requestId": request_id,
                    "command": command,
                    "host": if is_remote { Some(target_label.clone()) } else { None },
                }),
            );
            rx.await.unwrap_or(false)
        };

        if !allowed {
            audit.record(self.id, &target_label, command, "rejected", None);
            return Ok(format!("用户拒绝执行命令：{command}"));
        }

        let output = match &self.target {
            CodingTarget::Local => run_local_command(command, &self.workspace_root).await?,
            CodingTarget::Remote { connection_id, .. } => {
                let session = ssh_pool.get_or_connect(*connection_id).await?;
                session.exec(command).await?
            }
        };

        let summary: String = output.chars().take(2000).collect();
        audit.record(self.id, &target_label, command, "executed", Some(&summary));
        Ok(output.chars().take(4000).collect())
    }

    /// 用户点击 FileChangeCard 的"应用"：这时才真正写盘（expected_mtime 传 None
    /// 强制覆盖——用户已经在 UI 里显式确认过这份 Diff，不再需要走 mtime 冲突检测
    /// 那一套，那是给"和别的编辑动作并发"场景设计的）。
    ///
    /// 开启"自动 Git 提交"时顺带 commit 这一个文件——提交失败（没配置 git 身份、
    /// 目标机器没装 git 等）不应该让"应用变更"这个更核心的动作跟着失败，只把结果
    /// 通过事件告诉前端，用户自己看得懂那段 git 输出。
    pub async fn accept_change(
        &mut self,
        change_id: Uuid,
        ssh_pool: &SshConnectionPool,
        app_handle: &AppHandle,
    ) -> Result<(), AppError> {
        let change = self
            .changes
            .iter_mut()
            .find(|c| c.id == change_id)
            .ok_or_else(|| AppError::NotFound(format!("change not found: {change_id}")))?;
        if change.status != ChangeStatus::Pending {
            return Err(AppError::Conflict(format!("change {change_id} is not pending")));
        }
        self.file_ops.write_file(&change.path, &change.new_content, None).await?;
        change.status = ChangeStatus::Applied;
        let path = change.path.clone();
        self.undo_stack.clear();

        if self.auto_git_commit && self.git_repo {
            let message = format!("AI 编程助手：修改 {path}");
            let result = git_ops::commit_file(&self.target, &self.workspace_root, &path, &message, ssh_pool).await;
            let output = match result {
                Ok(out) => out,
                Err(e) => format!("Git 提交失败：{e}"),
            };
            let _ = app_handle.emit(
                "coding:git-commit-result",
                json!({ "sessionId": self.id, "path": path, "output": output }),
            );
        }

        Ok(())
    }

    pub fn reject_change(&mut self, change_id: Uuid) -> Result<(), AppError> {
        let change = self
            .changes
            .iter_mut()
            .find(|c| c.id == change_id)
            .ok_or_else(|| AppError::NotFound(format!("change not found: {change_id}")))?;
        if change.status != ChangeStatus::Pending {
            return Err(AppError::Conflict(format!("change {change_id} is not pending")));
        }
        change.status = ChangeStatus::Rejected;
        Ok(())
    }

    pub async fn undo_change(&mut self, change_id: Uuid) -> Result<(), AppError> {
        let change = self
            .changes
            .iter_mut()
            .find(|c| c.id == change_id)
            .ok_or_else(|| AppError::NotFound(format!("change not found: {change_id}")))?;
        if change.status != ChangeStatus::Applied {
            return Err(AppError::Conflict(format!("change {change_id} is not applied")));
        }
        self.file_ops.write_file(&change.path, &change.old_content, None).await?;
        change.status = ChangeStatus::Undone;
        self.undo_stack.push(change.clone());
        Ok(())
    }

    /// 标准编辑器行为：撤销点之后一旦产生新的、真正落盘的修改（`accept_change`），
    /// 原有 redo 分支就失效，否则 redo 可能把已经被覆盖的旧内容写回
    /// （DESIGN.md §3.8.3 骨架代码后的注解）——见 `accept_change` 里的 `undo_stack.clear()`。
    pub async fn redo_change(&mut self) -> Result<Option<Uuid>, AppError> {
        let Some(mut change) = self.undo_stack.pop() else { return Ok(None) };
        self.file_ops.write_file(&change.path, &change.new_content, None).await?;
        change.status = ChangeStatus::Applied;
        let id = change.id;
        if let Some(existing) = self.changes.iter_mut().find(|c| c.id == id) {
            *existing = change;
        }
        Ok(Some(id))
    }
}

fn tools_for_mode(mode: CodingMode) -> serde_json::Value {
    let all = tools::tool_schema();
    match mode {
        CodingMode::Build => all,
        CodingMode::Plan => {
            const READ_ONLY: &[&str] = &["read_file", "list_directory", "search_files"];
            serde_json::Value::Array(
                all.as_array()
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|t| READ_ONLY.contains(&t["function"]["name"].as_str().unwrap_or("")))
                    .collect(),
            )
        }
    }
}

pub(super) async fn run_local_command(command: &str, cwd: &str) -> Result<String, AppError> {
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = tokio::process::Command::new("cmd");
        c.arg("/C").arg(command);
        c
    };
    #[cfg(not(target_os = "windows"))]
    let mut cmd = {
        let mut c = tokio::process::Command::new("sh");
        c.arg("-c").arg(command);
        c
    };
    cmd.current_dir(cwd);
    let output = cmd.output().await?;
    Ok(format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    ))
}
