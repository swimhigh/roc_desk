//! `codex_engine::ExecTarget` 在 roc_desk 侧的实现——把 codex-core 通过
//! `RocDeskExecServer` 发来的 `fs/*`/`process/*` 请求，路由到 roc_desk 已有的
//! `ChangeStore`（Pending 状态机）和 `run_command_gated_shared`（权限规则引擎 +
//! 确认弹窗 + 本地/SSH/Agent 三态执行），不重新实现一遍。
//!
//! **不持有 `Arc<Mutex<CodingSession>>`**——这是刻意的设计，不是疏漏：
//! codex-core 通过 exec-server 协议发起的 `fs/writeFile`/`process/start` 请求，
//! 处理时机是在 `CodingSession::send_message`（codex 路由分支）already 用
//! `&mut self` 拿着这个会话、且正在 `await` `thread.next_event()` 的时候，由
//! `RocDeskExecServer` 的后台 WebSocket 任务（另一个 tokio task）回调过来的。
//! 如果这里再去 `session.lock().await` 同一个 `Arc<Mutex<CodingSession>>`，
//! 会因为外层已经持有这把锁而死锁。所以这里只持有真正需要的、要么本身独立
//! 加锁（`ChangeStore`）要么整个会话生命周期内不变（`workspace_root`/`target`/
//! `session_id`）的那部分状态，配合 `run_command_gated_shared`/
//! `ChangeStore::stage` 这两个不需要 `&CodingSession` 的自由函数/方法。
//!

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;
use std::sync::Mutex as StdMutex;

use async_trait::async_trait;
use serde_json::json;
use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex;
use uuid::Uuid;

use codex_engine::{CommandOutcome, ExecTarget, ExecTargetError, FileMetadata, PlatformHint};

use super::changes::ChangeStore;
use super::permission::PermissionEngine;
use super::session::{run_command_gated_shared_with_status_in_context, CodingTarget};
use crate::agent::AgentConnectionPool;
use crate::coding::CommandConfirmRegistry;
use crate::db::repo::audit_log_repo::AuditLogRepo;
use crate::db::repo::permission_rules_repo::PermissionRulesRepo;
use crate::error::AppError;
use crate::fsops::FileOps;
use crate::ssh::SshConnectionPool;

fn to_exec_error(e: AppError) -> ExecTargetError {
    ExecTargetError::Other(e.to_string())
}

pub struct SessionExecTarget {
    session_id: Uuid,
    workspace_root: String,
    target: CodingTarget,
    /// `None`——`CodingTarget::Local`，codex-core 的 `cwd` 就是 `workspace_root`
    /// 本身（同一台机器上的真实路径），沿用原生 `PathBuf` 语义直接解析。
    /// `Some(placeholder)`——Remote(SSH)/Agent，codex-core 的 `cwd` 传的是这个
    /// 本机真实存在的占位目录（见 `commands/coding.rs::attach_codex_engine`），
    /// 因为远程 `workspace_root`（比如 SSH 上的 `/data/lipeng/kgmscli`）不是
    /// Windows 绝对路径语法，直接喂给 codex-core 内部的 `AbsolutePathBuf`/
    /// `PathUri` 会被按 Windows 语义强行拼出一个跟真实远程路径毫不相干的本机
    /// 路径。这里改在 `ExecTarget` 这一层把"相对占位目录的路径"翻译回真正的
    /// 远程 `workspace_root`——codex-core 自己从不读写占位目录的内容。
    local_cwd_placeholder: Option<PathBuf>,
    mode: Arc<AtomicU8>,
    auto_allow_readonly: Arc<AtomicBool>,
    file_ops: Arc<dyn FileOps>,
    change_store: Arc<Mutex<ChangeStore>>,
    /// 当前轮次 id——`CodingSession`（codex 路由分支）在每次 `send_message`
    /// 开始时更新这个共享值，供后续这一轮里所有 `stage_write` 调用打上同一个
    /// `turn_id`，语义和自研引擎的 `current_turn_id` 一致。
    turn_id: Arc<StdMutex<Uuid>>,
    ssh_pool: Arc<SshConnectionPool>,
    agent_pool: Arc<AgentConnectionPool>,
    audit_log: Arc<AuditLogRepo>,
    command_confirms: CommandConfirmRegistry,
    permission_rules: Arc<PermissionRulesRepo>,
    app_handle: AppHandle,
}

impl SessionExecTarget {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        session_id: Uuid,
        workspace_root: String,
        target: CodingTarget,
        local_cwd_placeholder: Option<PathBuf>,
        mode: Arc<AtomicU8>,
        auto_allow_readonly: Arc<AtomicBool>,
        file_ops: Arc<dyn FileOps>,
        change_store: Arc<Mutex<ChangeStore>>,
        turn_id: Arc<StdMutex<Uuid>>,
        ssh_pool: Arc<SshConnectionPool>,
        agent_pool: Arc<AgentConnectionPool>,
        audit_log: Arc<AuditLogRepo>,
        command_confirms: CommandConfirmRegistry,
        permission_rules: Arc<PermissionRulesRepo>,
        app_handle: AppHandle,
    ) -> Self {
        Self {
            session_id,
            workspace_root,
            target,
            local_cwd_placeholder,
            mode,
            auto_allow_readonly,
            file_ops,
            change_store,
            turn_id,
            ssh_pool,
            agent_pool,
            audit_log,
            command_confirms,
            permission_rules,
            app_handle,
        }
    }

    fn is_plan_mode(&self) -> bool {
        self.mode.load(Ordering::Relaxed) == 0
    }

    /// `workspace_root` 字符串实际使用的路径分隔符——`Local`（运行 roc_desk 的
    /// 宿主机是 Windows）和 `Agent`（远程 Windows）都是 `\`，`Remote` 是 SSH
    /// （假定 *nix，用 `/`），跟 `guard::is_blacklisted`/`session.rs` 里
    /// `is_windows_target = matches!(target, CodingTarget::Agent { .. })` 的
    /// 判断口径保持一致。
    fn path_separator(&self) -> char {
        match self.target {
            CodingTarget::Remote { .. } => '/',
            _ => '\\',
        }
    }

    /// 把 codex-core 传来的路径（对 Local 是本机真实路径，对 Remote/Agent 是
    /// 相对/绝对于 `local_cwd_placeholder` 这个本机占位目录的路径）解析成一个
    /// "目标机器上真实可用"的路径字符串。返回 `String` 而不是 `PathBuf`——远程
    /// 路径的分隔符跟运行 roc_desk 的这台 Windows 机器的原生语法不一致，构造
    /// 一个 `PathBuf` 再 `to_string_lossy()` 会被强制拼成反斜杠，对 SSH/*nix
    /// 目标是错的。
    fn resolve_workspace_path(&self, path: &Path) -> Result<String, ExecTargetError> {
        let Some(placeholder) = &self.local_cwd_placeholder else {
            // Local：codex-core 的 cwd 就是 workspace_root 本身，沿用原生
            // PathBuf 语义（跟改造前的行为完全一致）。
            let root = normalize_path(Path::new(&self.workspace_root));
            if !root.is_absolute() {
                return Err(ExecTargetError::Other(format!(
                    "工作区根目录不是绝对路径：{}",
                    self.workspace_root
                )));
            }
            let requested = if path.is_absolute() {
                path.to_path_buf()
            } else {
                root.join(path)
            };
            let resolved = normalize_path(&requested);
            if !resolved.starts_with(&root) {
                return Err(ExecTargetError::PermissionDenied(format!(
                    "路径超出工作区：{}",
                    path.display()
                )));
            }
            return Ok(resolved.to_string_lossy().to_string());
        };

        let placeholder = normalize_path(placeholder);
        let requested = if path.is_absolute() {
            path.to_path_buf()
        } else {
            placeholder.join(path)
        };
        let requested = normalize_path(&requested);
        // `strip_prefix` 失败说明归一化后的路径已经跑到占位目录外面去了——
        // 和 Local 分支的"路径超出工作区"是同一种越权语义（`normalize_path`
        // 已经处理了 `..` 上跳，这里失败只可能是模型试图访问占位目录之外的
        // 本机路径）。
        let rel = requested.strip_prefix(&placeholder).map_err(|_| {
            ExecTargetError::PermissionDenied(format!("路径超出工作区：{}", path.display()))
        })?;

        let separator = self.path_separator();
        let mut joined = self
            .workspace_root
            .trim_end_matches(['/', '\\'])
            .to_string();
        for component in rel.components() {
            if let Component::Normal(part) = component {
                joined.push(separator);
                joined.push_str(&part.to_string_lossy());
            }
        }
        Ok(joined)
    }
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::normalize_path;
    use std::path::{Path, PathBuf};

    #[test]
    fn normalize_path_removes_current_and_parent_components() {
        let path = Path::new("workspace/./src/../tests");
        assert_eq!(
            normalize_path(path),
            PathBuf::from("workspace").join("tests")
        );
    }

    #[test]
    fn normalize_path_keeps_an_absolute_root() {
        let root = std::env::current_dir().unwrap();
        let path = root.join("src").join("..").join("tests");
        assert_eq!(normalize_path(&path), root.join("tests"));
    }
}

#[async_trait]
impl ExecTarget for SessionExecTarget {
    async fn read_file(&self, path: &Path) -> Result<Vec<u8>, ExecTargetError> {
        let path_str = self.resolve_workspace_path(path)?;
        if let Some(content) = self
            .change_store
            .lock()
            .await
            .pending_content_for(&path_str)
        {
            return Ok(content.into_bytes());
        }
        let content = self
            .file_ops
            .read_file_for_editor(&path_str)
            .await
            .map_err(to_exec_error)?;
        Ok(content.text.into_bytes())
    }

    async fn stage_write(&self, path: &Path, content: Vec<u8>) -> Result<(), ExecTargetError> {
        if self.is_plan_mode() {
            return Err(ExecTargetError::PermissionDenied(
                "Plan 模式不允许修改文件".into(),
            ));
        }
        let path_str = self.resolve_workspace_path(path)?;
        let text = String::from_utf8(content).map_err(|_| {
            ExecTargetError::Other(format!(
                "{path_str}: 不支持写入非 UTF-8 内容（和现有 FileChange 状态机的文本假设一致）"
            ))
        })?;
        let turn_id = *self.turn_id.lock().unwrap();
        let (change, sync) = self
            .change_store
            .lock()
            .await
            .stage(
                &path_str,
                text,
                turn_id,
                &self.ssh_pool,
                &self.agent_pool,
                &self.app_handle,
            )
            .await
            .map_err(to_exec_error)?;
        let _ = self.app_handle.emit(
            "coding:file-change",
            json!({ "sessionId": self.session_id, "change": &change, "sync": sync }),
        );
        Ok(())
    }

    async fn create_directory(&self, path: &Path, _recursive: bool) -> Result<(), ExecTargetError> {
        if self.is_plan_mode() {
            return Err(ExecTargetError::PermissionDenied(
                "Plan 模式不允许创建目录".into(),
            ));
        }
        let path_str = self.resolve_workspace_path(path)?;
        self.file_ops
            .create_dir(&path_str)
            .await
            .map_err(to_exec_error)
    }

    fn platform_hint(&self) -> PlatformHint {
        match self.target {
            CodingTarget::Local => PlatformHint {
                platform_os: "windows",
                shell_name: "powershell",
                shell_path: r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe",
                is_local_host: true,
            },
            CodingTarget::Remote { .. } => PlatformHint {
                platform_os: "linux",
                shell_name: "bash",
                shell_path: "/bin/bash",
                is_local_host: false,
            },
            CodingTarget::Agent { .. } => PlatformHint {
                platform_os: "windows",
                shell_name: "cmd",
                shell_path: r"C:\Windows\System32\cmd.exe",
                is_local_host: false,
            },
        }
    }

    async fn get_metadata(&self, path: &Path) -> Result<FileMetadata, ExecTargetError> {
        let path_str = self.resolve_workspace_path(path)?;
        // `FileOps` 没有单独的"stat 一个路径"原语（Local/Remote/Agent 三态都
        // 只有 `list_dir`），只能列出父目录再按文件名找对应条目——这跟
        // codex-core 实际的调用场景（探测 `AGENTS.md`/`.git`/`.agents/skills`
        // 存不存在）也是匹配的：调用方本来就是想知道"这个路径存不存在"，不是
        // 高频路径，列一次父目录的开销可以接受。
        let separator = self.path_separator();
        let (parent, name) = match path_str.rfind(separator) {
            Some(idx) => (path_str[..idx].to_string(), path_str[idx + 1..].to_string()),
            None => {
                return Err(ExecTargetError::NotFound(path_str));
            }
        };
        let parent = if parent.is_empty() {
            separator.to_string()
        } else {
            parent
        };
        if name.is_empty() {
            return Err(ExecTargetError::NotFound(path_str));
        }
        let entries = self
            .file_ops
            .list_dir(&parent)
            .await
            .map_err(|_| ExecTargetError::NotFound(path_str.clone()))?;
        let entry = entries
            .into_iter()
            .find(|e| e.name == name)
            .ok_or(ExecTargetError::NotFound(path_str))?;
        Ok(FileMetadata {
            is_directory: entry.is_dir,
            is_file: !entry.is_dir,
            is_symlink: false,
            size: entry.size.unwrap_or(0),
            created_at_ms: entry.modified.unwrap_or(0),
            modified_at_ms: entry.modified.unwrap_or(0),
        })
    }

    async fn run_command(
        &self,
        argv: Vec<String>,
        cwd: &Path,
        env: HashMap<String, String>,
    ) -> Result<CommandOutcome, ExecTargetError> {
        // `run_command_gated_shared` 期望一整条 shell 命令字符串（权限规则引擎/
        // 黑名单都是按整串命令文本匹配的），不是 argv 数组，所以这里仍然要把
        // codex 传来的 argv 拼成一条命令——但拼接规则必须按目标机器的 shell 语法
        // 选择：`Agent`（远程 Windows）最终经 `cmd.exe /C <字符串>` 执行
        // （见 `agent/session.rs::exec`），POSIX 的 `shell_quote`（单引号转义）
        // 对 cmd.exe 完全无效，会把带空格/特殊字符的参数拆碎，这也是
        // `git_ops.rs` 对 Agent 目标改用 `exec_argv` 绕开字符串拼接的原因
        // （AGENT_DESIGN.md §一）。这里的执行路径必须经过共享的权限/黑名单
        // 引擎，没法像 `git_ops.rs` 那样直接跳过拼字符串这一步，所以改成按
        // 目标选择对应语法的转义规则。
        //
        // `CodingTarget::Local` 一开始漏算在内——之前只把 `Agent` 当"目标是
        // Windows"，但 roc_desk 本来就只跑在 Windows 上，本地目标最终也是走
        // `run_local_command_output_with_env` 里的 `cmd.exe /C <字符串>`
        // （`session.rs`），同样不认 POSIX 单引号。2026-09 用户实测复现：本地
        // 目标下 `platform_hint` 正确申报了 `powershell` 之后，codex-core 按
        // PowerShell 语法把 argv 拼成 `[powershell.exe, -NoProfile, -Command,
        // "<内容>"]`，这里却仍然用 `shell_quote` 给它套了一层 POSIX 单引号，
        // `cmd.exe` 拿到这种字符串直接解析错乱，模型看到的是"路径/目录语法
        // 错误"——其实是转义规则从一开始就用错了，不是权限/沙箱问题。
        if self.is_plan_mode() {
            return Err(ExecTargetError::PermissionDenied(
                "Plan 模式不允许执行命令".into(),
            ));
        }
        let cwd = self.resolve_workspace_path(cwd)?;
        let is_windows_target = !matches!(self.target, CodingTarget::Remote { .. });
        let command = argv
            .iter()
            .map(|a| {
                if is_windows_target {
                    cmd_quote(a)
                } else {
                    crate::log::remote::shell_quote(a)
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
        // 环境变量：`run_target_command`（`session.rs`）目前只有 `CodingTarget::Local`
        // 真正把 `env` 传进子进程——SSH 的 `session.exec(command)`、Windows Agent 的
        // `session.exec(command, cwd)` 都没有接收环境变量的参数，这是三态执行本身
        // 早就有的限制，不是这里新引入的。之前这里对非 Local 目标直接硬错误拒绝执行，
        // 但 codex-core 几乎每次工具调用都会带一些环境变量覆盖，硬错误会导致远程
        // 目标下所有命令都跑不了；改成静默丢弃不支持的环境变量，跟 SSH/Agent 路径
        // 早就有的行为一致。
        let env = if matches!(self.target, CodingTarget::Local) {
            env
        } else {
            HashMap::new()
        };
        let permission_engine =
            PermissionEngine::load(&self.permission_rules).map_err(to_exec_error)?;
        let full_auto = self
            .change_store
            .lock()
            .await
            .full_auto
            .load(Ordering::Relaxed);
        let output = run_command_gated_shared_with_status_in_context(
            self.session_id,
            &self.target,
            &self.workspace_root,
            self.auto_allow_readonly.load(Ordering::Relaxed),
            full_auto,
            &cwd,
            &env,
            &command,
            &self.ssh_pool,
            &self.agent_pool,
            &self.audit_log,
            &self.command_confirms,
            &permission_engine,
            &self.app_handle,
        )
        .await
        .map_err(to_exec_error)?;
        let output = CommandOutcome {
            output: output.output.into_bytes(),
            exit_code: output.exit_code,
        };
        Ok(output)
    }
}

/// `cmd.exe /C` 的参数转义——不存在 POSIX 单引号那样的"整体禁止转义"语法，规则是
/// 双引号包裹 + 内部双引号翻倍。只在参数包含空白/特殊字符时才加引号，避免给正常的
/// 单个路径/标志套上多余的引号。
fn cmd_quote(arg: &str) -> String {
    let needs_quote = arg.is_empty()
        || arg
            .chars()
            .any(|c| c.is_whitespace() || matches!(c, '"' | '^' | '&' | '|' | '<' | '>' | '%'));
    if !needs_quote {
        return arg.to_string();
    }
    format!("\"{}\"", arg.replace('"', "\"\""))
}
