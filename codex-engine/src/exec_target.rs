use std::collections::HashMap;
use std::path::Path;

use async_trait::async_trait;

/// `RocDeskExecServer` 对外委派的执行后端——roc_desk 侧实现这个 trait，把
/// codex-core 发来的 `fs/*`/`process/*` 请求路由到本地/SSH/Windows Agent 三种
/// 目标（复用 `CodingTarget`/`FileOps`/`run_command_gated`，不重新实现一遍）。
///
/// 这个 trait 定义在 `codex-engine` 里而不是 `src-tauri` 里，是为了避免
/// workspace 内的循环依赖：`src-tauri` 依赖 `codex-engine`（`codex-engine` 反过来
/// 不依赖 `src-tauri` 的任何类型），`src-tauri` 侧只需要实现这个 trait、把已有的
/// `ChangeStore`/`guard::run_command_gated` 包一层适配即可。
#[async_trait]
pub trait ExecTarget: Send + Sync {
    /// 读取文件内容。如果该路径存在未落盘的 pending 改动（roc_desk 自己的
    /// `ChangeStore` 语义），实现方要返回 pending 内容而不是磁盘内容——这样
    /// 模型在同一轮里多次读写同一文件时看到的是"提议中"的最新版本，和现有
    /// 自研引擎里 `pending_content_for` 的语义保持一致。
    async fn read_file(&self, path: &Path) -> Result<Vec<u8>, ExecTargetError>;

    /// "写入"一个文件——注意这不代表真的落盘。实现方应该把这次写入转换成
    /// roc_desk 的 `ChangeStore::stage`（生成一条 `Pending` 状态的
    /// `FileChange`），对 codex 而言这次调用永远视为成功，真正落盘延后到
    /// 用户在 UI 上点 Accept（`full_auto` 开启时例外，立即落盘）。
    async fn stage_write(&self, path: &Path, content: Vec<u8>) -> Result<(), ExecTargetError>;

    /// 创建目录——直接落盘（目录创建本身不参与 Pending/Accept 审批，和现有
    /// 自研引擎的 `create_directory` 工具语义一致）。
    async fn create_directory(&self, path: &Path, recursive: bool) -> Result<(), ExecTargetError>;

    /// 执行一次命令并等待完成，委派给 roc_desk 已有的 `run_command_gated`
    /// （权限规则引擎 + 确认弹窗 + 本地/SSH/Agent 三态路由）。这里刻意设计成
    /// "等待完成才返回"的同步语义，而不是流式交互式进程——和现有自研引擎的
    /// `run_command` 工具、以及 Windows Agent 的一次性命令执行原语（默认 120s
    /// 超时）保持一致，不引入新的交互式 stdin 能力。
    async fn run_command(
        &self,
        argv: Vec<String>,
        cwd: &Path,
        env: HashMap<String, String>,
    ) -> Result<CommandOutcome, ExecTargetError>;

    /// 查询单个路径的元信息——codex-core 用它探测 `AGENTS.md`/`CLAUDE.md`
    /// 是否存在（`agents_md.refresh`）、沿着目录链找 `.git`/`.agents/skills`
    /// 标记（技能发现）。之前没实现这个方法，`RocDeskExecServer` 直接回
    /// "method not implemented"，这两个功能在 codex 引擎路径下完全用不了；
    /// 现在委派给 `ExecTarget` 的实现方（roc_desk 侧用 `FileOps::list_dir`
    /// 列出父目录、按文件名找对应条目拼出来，因为 `FileOps` 本身没有单独的
    /// "stat 一个路径" 原语，Local/Remote/Agent 三态都是这个形状）。
    async fn get_metadata(&self, path: &Path) -> Result<FileMetadata, ExecTargetError>;

    /// 告诉 codex-core 这个执行目标的真实操作系统/默认 shell——`initialize`
    /// 握手阶段的 `EnvironmentInfo` 需要这个信息，codex-core 会按申报的 shell
    /// 名字（`"bash"`/`"cmd"`/`"powershell"` 等，见 vendor/codex 的
    /// `core/src/shell.rs::Shell::from_environment_shell_info`）决定怎么给
    /// 实际要跑的命令套壳（`Shell::derive_exec_args`：POSIX shell 套
    /// `<shell> -c "<command>"`，PowerShell 套 `powershell.exe -Command
    /// "<command>"`）。这个信息如果报错，套壳会直接套错——2026-09 用户实测
    /// 复现：SSH（Linux）远程目标下，codex 把要跑的 `sed` 命令套成了
    /// `powershell.exe -Command "sed ..."` 去执行，因为 exec-server 握手阶段
    /// 报的 shell 一直是本机（运行 roc_desk 的 Windows 机器）的 PowerShell，
    /// 跟真正要执行命令的目标机器完全对不上。
    fn platform_hint(&self) -> PlatformHint;
}

/// 与 codex-core `ShellInfo` 对齐的最小子集——只暴露 `Shell::
/// from_environment_shell_info` 认识的 `name`（`"bash"`/`"sh"`/`"cmd"`/
/// `"powershell"`/`"zsh"`）和对应的 shell 可执行文件路径。不直接依赖
/// `codex_exec_server_protocol::ShellInfo` 类型——跟 `FileMetadata` 一样，
/// 保持 `exec_target.rs`（trait 定义处）不依赖协议细节。
#[derive(Debug, Clone, Copy)]
pub struct PlatformHint {
    pub platform_os: &'static str,
    pub shell_name: &'static str,
    pub shell_path: &'static str,
    /// 执行目标是不是运行 roc_desk 的这台机器本身——只有这种情况下，
    /// `EnvironmentInfo::local()` 探测出来的 `cwd`/`user_home_dir`/`temp_dir`
    /// 才是真实有效的；Remote/Agent 这些字段报的是本机路径，跟真正的执行
    /// 目标毫不相干，报了反而误导 codex-core，不如干脆不报。
    pub is_local_host: bool,
}

/// 和 `codex_exec_server_protocol::FsGetMetadataResponse` 字段对齐，但故意不
/// 直接依赖那个 crate 的类型——`codex-engine` 里 `exec_target.rs`（这个 trait 的
/// 定义处）和 `exec_server.rs`（协议实现处）保持解耦，跟文件头注释里
/// "roc_desk 侧实现这个 trait，不需要知道 exec-server 协议细节" 的原则一致。
#[derive(Debug, Clone, Copy)]
pub struct FileMetadata {
    pub is_directory: bool,
    pub is_file: bool,
    /// roc_desk 的 `FileEntry`（`FileOps::list_dir` 的返回类型）不区分符号链接，
    /// 三态实现都统一报 `false`——这是一个已知的近似，不是刻意误报：目前没有
    /// 调用方（AGENTS.md 探测/技能发现）依赖这个字段做真实的符号链接判断。
    pub is_symlink: bool,
    pub size: u64,
    /// `FileOps` 不单独跟踪创建时间，用修改时间兜底——同样是已知近似。
    pub created_at_ms: i64,
    pub modified_at_ms: i64,
}

pub struct CommandOutcome {
    /// 合并后的 stdout+stderr（和 `SshSession::exec`/`run_local_command` 的
    /// 现有约定一致，不单独区分两个流）。
    pub output: Vec<u8>,
    /// 命令的退出码；被信号杀死等场景下可能拿不到，用 `None` 表示。
    pub exit_code: Option<i32>,
}

#[derive(Debug, thiserror::Error)]
pub enum ExecTargetError {
    #[error("path not found: {0}")]
    NotFound(String),
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    #[error("execution target error: {0}")]
    Other(String),
}
