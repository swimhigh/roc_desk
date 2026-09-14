//! roc_desk 对 OpenAI Codex（codex-rs）的深度嵌入封装层。
//!
//! 见 `docs/CODEX_INTEGRATION_PLAN.md`：核心思路是不解析 codex 的 `apply_patch`
//! diff 格式、也不对接它的审批 RPC，而是在协议更底层的位置——codex-core 的
//! `Environment` 抽象要求的 exec-server JSON-RPC 协议（`fs/*`/`process/*`）——
//! 把 roc_desk 已有的三态执行（本地/SSH/Windows Agent）、Pending 状态机
//! （`ChangeStore`）接进去。codex-core 看到的是一个"每次都成功"的文件系统，
//! 真正的把关全在 roc_desk 自己实现的 `RocDeskExecServer` 里。
//!
//! 这个 crate 目前只实现 `exec_target`/`exec_server` 两个模块（对应
//! docs/CODEX_INTEGRATION_PLAN.md 的 Phase 1）：把 `ExecTarget` trait 的一个
//! 实现绑定到一个本地 WebSocket 端口上，让 codex-core 通过
//! `EnvironmentManager::upsert_environment_with_options` 注册这个地址后，
//! 所有 `fs/*`/`process/*` 请求都会落到 `ExecTarget` 上。驱动 codex-core 对话
//! 循环本身的 `ChatEngine`/`CodexCoreEngine`（Phase 3）是后续独立的工作，
//! 尚未在这个 crate 里实现。

pub mod engine;
pub mod exec_server;
pub mod exec_target;

pub use engine::{
    ChatEngine, CodexCoreEngine, CodexCoreEngineHandle, CodexCoreEngineOptions, EngineError,
    EngineEventSink, EngineInput,
};
pub use exec_server::{RocDeskExecServer, RocDeskExecServerError};
pub use exec_target::{CommandOutcome, ExecTarget, ExecTargetError, FileMetadata, PlatformHint};
