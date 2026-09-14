//! `ChatEngine`：把"驱动一轮对话"抽象成一个 trait，`CodexCoreEngine`（这个文件）
//! 和 roc_desk 现有的自研 HTTP+工具循环各是一个实现。见
//! docs/CODEX_INTEGRATION_PLAN.md Phase 3。
//!
//! `CodexCoreEngine` 参照官方 `codex-rs/thread-manager-sample` 的模式驱动
//! `codex_core_api::ThreadManager`/`CodexThread`：
//! - `Config.permissions` 设成 `AskForApproval::Never` + `PermissionProfile::Disabled`
//!   ——不让 codex 自己弹审批、也不套它自己的沙箱，因为真正的把关全在
//!   `RocDeskExecServer`（`exec_server.rs`）那一层做，这里只是让 codex 认为
//!   "什么都被允许"。
//! - turn 通过 `ThreadSettingsOverrides.environments` 绑定到调用方传入的
//!   `environment_id`（调用方要先用 `EnvironmentManager::upsert_environment_with_options`
//!   注册这个 id 指向 `RocDeskExecServer::websocket_url()`）。
//! - 鉴权用 `CodexAuth::from_api_key` + `AuthManager::from_auth_for_testing_with_home`——
//!   这是 codex-login 里唯一一个"直接注入一个 API Key，不走 codex 自己的
//!   auth.json/keyring/ChatGPT 登录流程"的公开入口，虽然命名是 `for_testing`，
//!   但没有 `#[cfg(test)]` 门禁。跟着 codex 升级时要注意这个 API 有没有改名/删除。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;

use codex_core_api::init_state_db;
use codex_core_api::{
    build_models_manager, install_image_generation_extension,
    local_agent_graph_store_from_state_db, passthrough_image_store, resolve_installation_id,
    thread_store_from_config, AskForApproval, AuthManager, CodexAppsToolsCache, CodexAuth,
    CodexHomeUserInstructionsProvider, CodexThread, Config, Constrained, EnvironmentConfigState,
    EnvironmentManager, ExecServerRuntimePaths, ExtensionRegistryBuilder, NewThread,
    PermissionProfile, Permissions, ProjectConfig, SessionSource, StartIfIdleSubmission,
    StartThreadOptions, ThreadId, ThreadManager, TurnEnvironmentSelection, TurnInputRequest,
    UriBasedFileOpener, UserInput, OPENAI_PROVIDER_ID,
};
use codex_exec_server::RemoteEnvironmentOptions;
use codex_model_provider_info::ModelProviderInfo;
use codex_protocol::protocol::{EventMsg, Op, ThreadSettingsOverrides, TurnEnvironmentSelections};
use codex_utils_absolute_path::AbsolutePathBuf;
use codex_utils_path_uri::PathUri;

/// 对话事件的旁路回调——只关心"要不要往前端广播点什么"的那部分事件（助手文本/
/// 推理过程/工具调用进度），文件读写/命令执行的副作用不走这里，那些已经在
/// `ExecTarget` 实现里直接接进 `ChangeStore`/`run_command_gated` 了（见
/// `exec_target.rs`/`src-tauri/src/coding/codex_exec_target.rs`）。默认全部
/// no-op，调用方只需要实现自己关心的部分。
pub trait EngineEventSink: Send + Sync + 'static {
    fn on_assistant_delta(&self, _text: &str) {}
    fn on_reasoning_delta(&self, _text: &str) {}
    fn on_tool_progress(&self, _label: &str) {}
    /// 配对 `on_tool_progress`——`EventMsg::ExecCommandEnd`/`PatchApplyEnd` 触发。
    /// 2026-09 用户实测复现：这个回调之前根本不存在，`ExecCommandEnd`/
    /// `PatchApplyEnd` 落进 `run_turn` 事件循环的 `_ => {}` 通配分支被静默丢弃，
    /// 前端"正在执行"的时间线条目永远等不到配对的"执行完成"事件、永远转圈——
    /// 哪怕后端那条命令其实几百毫秒就跑完了（日志能证实：`run_command_gated`
    /// 的诊断打点从"已放行开始执行"到"实际执行完成"经常不到一秒）。这不是
    /// 真的卡死，是纯前端展示状态没更新。
    fn on_tool_progress_end(&self) {}
}

pub enum EngineInput {
    Text(String),
    ImageDataUrl(String),
}

#[async_trait]
pub trait ChatEngine: Send {
    async fn run_turn(
        &mut self,
        input: Vec<EngineInput>,
        sink: &dyn EngineEventSink,
    ) -> Result<String, EngineError>;
}

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("codex engine error: {0}")]
    Other(String),
}

pub struct CodexCoreEngineOptions {
    pub api_key: String,
    /// 覆盖模型选择；`None` 用 provider 的默认模型。
    pub model: Option<String>,
    /// 自定义 OpenAI 兼容端点（企业内部网关/代理这类场景）；`None` 时用 codex
    /// 内建的 OpenAI 官方 provider。roc_desk 的 provider 配置（`api_base`）要
    /// 传到这里才会真正生效——不传的话 codex-core 一律打官方 `api.openai.com`，
    /// 和 roc_desk 这边配的地址没关系。
    pub base_url: Option<String>,
    /// roc_desk 自己管的 codex 状态目录（不是用户全局的 `~/.codex`）——每个
    /// workspace/session 一份，避免和用户真的装了官方 codex CLI 时的状态打架。
    pub codex_home: PathBuf,
    pub cwd: PathBuf,
    /// `RocDeskExecServer::websocket_url()`，例如 `ws://127.0.0.1:54321`。
    pub exec_server_url: String,
    /// 绑定这个 turn 到哪个 environment id——调用方要先用
    /// `EnvironmentManager::upsert_environment_with_options` 注册同名 environment。
    pub environment_id: String,
}

pub struct CodexCoreEngine {
    thread: Arc<CodexThread>,
    _thread_id: ThreadId,
    environment_id: String,
    cwd: PathBuf,
}

impl CodexCoreEngine {
    pub async fn new(options: CodexCoreEngineOptions) -> Result<Self, EngineError> {
        let config = build_config(&options)?;
        let state_db = init_state_db(&config).await;

        let auth = CodexAuth::from_api_key(&options.api_key);
        let auth_manager = AuthManager::from_auth_for_testing_with_home(
            auth,
            config.codex_home.clone().to_path_buf(),
        );

        let local_runtime_paths = ExecServerRuntimePaths::from_optional_paths(
            config.codex_self_exe.clone(),
            config.codex_linux_sandbox_exe.clone(),
        )
        .map_err(|e| EngineError::Other(format!("exec-server runtime paths: {e}")))?;
        let thread_store = thread_store_from_config(&config, state_db.clone());
        let environment_manager = Arc::new(
            EnvironmentManager::from_codex_home(
                config.codex_home.clone(),
                Some(local_runtime_paths),
                config.http_client_factory(),
            )
            .await
            .map_err(|e| EngineError::Other(format!("environment manager: {e}")))?,
        );
        environment_manager
            .upsert_environment_with_options(
                options.environment_id.clone(),
                RemoteEnvironmentOptions {
                    exec_server_url: options.exec_server_url.clone(),
                    connect_timeout: None,
                    http_headers: HashMap::new(),
                },
            )
            .map_err(|e| EngineError::Other(format!("register exec-server environment: {e}")))?;

        let installation_id = resolve_installation_id(&config.codex_home)
            .await
            .map_err(|e| EngineError::Other(format!("installation id: {e}")))?;
        let user_instructions_provider = Arc::new(CodexHomeUserInstructionsProvider::new(
            config.codex_home.clone(),
        ));
        let mut extensions = ExtensionRegistryBuilder::<Config>::new();
        install_image_generation_extension(
            &mut extensions,
            auth_manager.clone(),
            |config: &Config| Some(config.codex_home.clone()),
        );

        let models_manager = build_models_manager(&config, auth_manager.clone());
        let agent_graph_store = local_agent_graph_store_from_state_db(state_db.as_ref());
        let thread_manager = ThreadManager::new(
            &config,
            Arc::clone(&auth_manager),
            models_manager,
            CodexAppsToolsCache::default(),
            SessionSource::Exec,
            environment_manager,
            Arc::new(extensions.build()),
            user_instructions_provider,
            None,
            passthrough_image_store(),
            Arc::clone(&thread_store),
            agent_graph_store,
            installation_id,
            None,
            None,
        );

        let NewThread {
            thread_id, thread, ..
        } = thread_manager
            .start_thread(StartThreadOptions::new(config))
            .await
            .map_err(|e| EngineError::Other(format!("start thread: {e}")))?;

        Ok(Self {
            thread,
            _thread_id: thread_id,
            environment_id: options.environment_id,
            cwd: options.cwd,
        })
    }

    /// 供"停止"按钮用——`CodexThread::submit`/`next_event` 都只要 `&self`
    /// （`self.thread` 本来就是 `Arc<CodexThread>`），所以可以在
    /// `run_turn` 的事件循环还卡在 `next_event().await` 的同时，从外部
    /// （`CodexCoreEngineHandle::interrupt_current_turn`）并发调用这个方法
    /// 提交 `Op::Interrupt`，不需要等当前 turn 跑完。
    pub(crate) fn thread_handle(&self) -> Arc<CodexThread> {
        Arc::clone(&self.thread)
    }
}

#[async_trait]
impl ChatEngine for CodexCoreEngine {
    async fn run_turn(
        &mut self,
        input: Vec<EngineInput>,
        sink: &dyn EngineEventSink,
    ) -> Result<String, EngineError> {
        let input = input
            .into_iter()
            .map(|item| match item {
                EngineInput::Text(text) => UserInput::Text {
                    text,
                    text_elements: Vec::new(),
                },
                EngineInput::ImageDataUrl(image_url) => UserInput::Image {
                    image_url,
                    detail: None,
                },
            })
            .collect();
        let cwd_uri = PathUri::from_host_native_path(&self.cwd)
            .map_err(|e| EngineError::Other(format!("cwd path: {e}")))?;
        let legacy_fallback_cwd = AbsolutePathBuf::try_from(self.cwd.clone())
            .map_err(|e| EngineError::Other(format!("cwd absolute path: {e}")))?;
        let environments = TurnEnvironmentSelections::new(
            legacy_fallback_cwd,
            vec![TurnEnvironmentSelection {
                environment_id: self.environment_id.clone(),
                cwd: cwd_uri.clone(),
                workspace_roots: vec![cwd_uri],
                config: EnvironmentConfigState::FromThread,
            }],
        );

        let submission = self
            .thread
            .start_turn_if_idle(TurnInputRequest::user_input(input).with_thread_settings(
                ThreadSettingsOverrides {
                    environments: Some(environments),
                    ..Default::default()
                },
            ))
            .await
            .map_err(|e| EngineError::Other(format!("submit turn: {e}")))?;
        if let StartIfIdleSubmission::NotSubmitted { reason } = submission {
            return Err(EngineError::Other(format!(
                "turn not submitted: {reason:?}"
            )));
        }

        let mut final_text = String::new();
        // 2026-09 用户反馈：中转服务偶发 `server_is_overloaded`，codex-core 内建的
        // `UnboundedConnectionRetries` 特性会在后台静默重试（实测重试成功前能等上
        // 1~2 分钟），但 `next_event()` 在这期间不会产生任何事件——`EventMsg`
        // （这个"库嵌入"层面的事件面，见 vendor/codex 的 `protocol.rs`）本身也没有
        // 暴露"正在重试第几次"这类信号，没法精确复述 codex-core 的重试状态。
        // 折中方案：用 `tokio::select!` 给 `next_event()` 包一层心跳定时器，超过
        // `STALL_HEARTBEAT` 还没等到下一个事件就推一条"仍在等待"的状态提示，最多推
        // `MAX_STALL_HEARTBEATS` 次（避免长时间卡顿时刷屏）。`next_event()` 内部是
        // `async_channel::Receiver::recv()`（见 `codex-core` 的
        // `session/mod.rs::next_event`），这个 recv 是 cancel-safe 的，定时器先触发
        // 时丢弃这个 future 不会丢事件、下一轮循环重新 await 就能拿到。
        const STALL_HEARTBEAT: std::time::Duration = std::time::Duration::from_secs(10);
        const MAX_STALL_HEARTBEATS: u32 = 6;
        let mut stall_heartbeats: u32 = 0;
        loop {
            let event = tokio::select! {
                biased;
                event = self.thread.next_event() => {
                    stall_heartbeats = 0;
                    event.map_err(|e| EngineError::Other(format!("read event: {e}")))?
                }
                _ = tokio::time::sleep(STALL_HEARTBEAT) => {
                    if stall_heartbeats < MAX_STALL_HEARTBEATS {
                        stall_heartbeats += 1;
                        sink.on_reasoning_delta(
                            "（模型响应较慢，仍在等待——如果是接口偶发过载，客户端会自动重试）"
                        );
                    }
                    continue;
                }
            };
            match event.msg {
                EventMsg::AgentMessage(msg) => {
                    final_text.push_str(&msg.message);
                    sink.on_assistant_delta(&msg.message);
                }
                EventMsg::AgentReasoning(msg) => sink.on_reasoning_delta(&msg.text),
                EventMsg::ExecCommandBegin(begin) => {
                    sink.on_tool_progress(&format!("run_command: {}", begin.command.join(" ")))
                }
                EventMsg::PatchApplyBegin(begin) => {
                    for path in begin.changes.keys() {
                        sink.on_tool_progress(&format!("apply_patch: {}", path.display()));
                    }
                }
                EventMsg::ExecCommandEnd(_) | EventMsg::PatchApplyEnd(_) => {
                    sink.on_tool_progress_end();
                }
                EventMsg::TurnComplete(_) => break,
                EventMsg::Error(err) => return Err(EngineError::Other(err.message)),
                EventMsg::TurnAborted(_) => {
                    return Err(EngineError::Other("turn aborted".to_string()))
                }
                _ => {}
            }
        }
        Ok(final_text)
    }
}

/// `CodexCoreEngine` 本身**不能**安全地在 roc_desk 主进程默认的线程/tokio
/// worker 栈上使用——实测（`codex_smoke` 独立烟测程序）`ThreadManager::
/// start_thread`/`CodexThread::next_event` 这条调用链在默认栈大小下会直接
/// `STATUS_STACK_OVERFLOW`（Windows 上进程级致命错误，不是能 `catch` 的
/// panic），codex 自己的 TUI 那边也有一条"把线程栈调到 12MiB"的提交佐证这不是
/// roc_desk 这边独有的问题。**只调大 tokio worker 线程栈不够**——`block_on`
/// 挂的根 Future 实际跑在调用 `block_on` 的那个线程本身上，必须连那个线程的
/// 栈也一起调大。
///
/// `CodexCoreEngineHandle` 把这件事封装掉：内部另起一个手动指定大栈
/// （见 `ENGINE_THREAD_STACK_SIZE`）的专用线程，在上面建一个小 tokio
/// runtime、常驻跑 `CodexCoreEngine`，外部只通过 channel 收发请求——这样
/// `CodingSession`/Tauri command 处理函数所在的（默认栈大小的）异步上下文
/// 永远不会直接执行 codex-core 的深层调用链，完全不需要关心栈大小问题。
const ENGINE_THREAD_STACK_SIZE: usize = 64 * 1024 * 1024;
const ENGINE_RUNTIME_WORKER_STACK_SIZE: usize = 32 * 1024 * 1024;

enum EngineRequest {
    RunTurn {
        input: Vec<EngineInput>,
        sink: Arc<dyn EngineEventSink>,
        reply: tokio::sync::oneshot::Sender<Result<String, EngineError>>,
    },
}

pub struct CodexCoreEngineHandle {
    tx: tokio::sync::mpsc::UnboundedSender<EngineRequest>,
    /// 独立于 `tx` 那条"一次只处理一个 RunTurn"的串行队列——`interrupt_current_turn`
    /// 直接在这个 `Arc<CodexThread>` 上调用 `submit(Op::Interrupt)`，不需要排队
    /// 等当前正在跑的 turn 处理完，这正是"停止"按钮能在长时间卡住时也生效的关键。
    thread: Arc<CodexThread>,
    _worker: std::thread::JoinHandle<()>,
}

impl CodexCoreEngineHandle {
    /// 起专用线程构造 `CodexCoreEngine` 并常驻跑对话循环；这个函数本身可以在
    /// 普通（默认栈大小的）async 上下文里安全调用——真正吃栈的初始化工作全部
    /// 发生在新线程上，这里只是等一个"初始化完成/失败"的信号。
    pub async fn spawn(options: CodexCoreEngineOptions) -> Result<Self, EngineError> {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<EngineRequest>();
        let (ready_tx, ready_rx) =
            tokio::sync::oneshot::channel::<Result<Arc<CodexThread>, EngineError>>();

        let worker = std::thread::Builder::new()
            .name("codex-core-engine".to_string())
            .stack_size(ENGINE_THREAD_STACK_SIZE)
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .thread_stack_size(ENGINE_RUNTIME_WORKER_STACK_SIZE)
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        let _ = ready_tx.send(Err(EngineError::Other(format!(
                            "build codex engine runtime: {e}"
                        ))));
                        return;
                    }
                };
                runtime.block_on(async move {
                    let mut engine = match CodexCoreEngine::new(options).await {
                        Ok(engine) => {
                            let _ = ready_tx.send(Ok(engine.thread_handle()));
                            engine
                        }
                        Err(e) => {
                            let _ = ready_tx.send(Err(e));
                            return;
                        }
                    };
                    while let Some(req) = rx.recv().await {
                        match req {
                            EngineRequest::RunTurn { input, sink, reply } => {
                                let result = engine.run_turn(input, sink.as_ref()).await;
                                let _ = reply.send(result);
                            }
                        }
                    }
                });
            })
            .map_err(|e| EngineError::Other(format!("spawn codex engine thread: {e}")))?;

        let thread = ready_rx.await.map_err(|_| {
            EngineError::Other("codex engine thread exited before signaling ready".to_string())
        })??;
        Ok(Self {
            tx,
            thread,
            _worker: worker,
        })
    }

    /// 中断当前正在跑的 turn（如果有）——"停止"按钮用。`Op::Interrupt` 提交给
    /// codex-core 自己的 session 循环后，正在等待的 `next_event()` 会收到
    /// `EventMsg::TurnAborted`，`run_turn` 那边会正常返回一个错误（不是被强行
    /// kill 掉），codex-core 内部状态不会被留在半途的不一致状态。如果这时候
    /// 根本没有 turn 在跑，`submit` 大概率也会成功（提交一个此刻没有对应 turn
    /// 的中断请求），按 no-op 处理，不当成错误。
    pub async fn interrupt_current_turn(&self) -> Result<(), EngineError> {
        self.thread
            .submit(Op::Interrupt)
            .await
            .map(|_| ())
            .map_err(|e| EngineError::Other(format!("interrupt turn: {e}")))
    }

    /// 和 `ChatEngine::run_turn` 语义一样，只是 `sink` 要跨线程传递，换成
    /// `Arc<dyn EngineEventSink>`（trait 已经要求 `Send + Sync + 'static`）
    /// 而不是裸引用。
    pub async fn run_turn(
        &self,
        input: Vec<EngineInput>,
        sink: Arc<dyn EngineEventSink>,
    ) -> Result<String, EngineError> {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        self.tx
            .send(EngineRequest::RunTurn {
                input,
                sink,
                reply: reply_tx,
            })
            .map_err(|_| EngineError::Other("codex engine worker thread is gone".to_string()))?;
        reply_rx.await.map_err(|_| {
            EngineError::Other("codex engine worker thread dropped the reply".to_string())
        })?
    }
}

fn build_config(options: &CodexCoreEngineOptions) -> Result<Config, EngineError> {
    let cwd = AbsolutePathBuf::try_from(options.cwd.clone())
        .map_err(|e| EngineError::Other(format!("cwd: {e}")))?;
    let codex_home = AbsolutePathBuf::try_from(options.codex_home.clone())
        .map_err(|e| EngineError::Other(format!("codex_home: {e}")))?;
    // `ExecServerRuntimePaths::from_optional_paths` 硬性要求 `codex_self_exe`
    // 非空——即使我们从不绑定本地 environment、从不触发 codex 自己的 arg0
    // 自派发（apply_patch/沙箱 helper 模式），这个字段仍然是构造它时的必填项
    // （运行时实测发现的：`CodexCoreEngine::new` 会在这一步直接报错
    // "Codex executable path is not configured"）。用当前进程自己的可执行文件
    // 路径占位——只要不触发 `--codex-run-as-apply-patch` 这类特殊 argv 分发，
    // 这个路径的实际内容不影响任何行为。
    let codex_self_exe =
        std::env::current_exe().map_err(|e| EngineError::Other(format!("current_exe: {e}")))?;
    // `model_providers`（复数，`Config` 要求的完整映射表）和这次会话实际选用的
    // `model_provider`（单数）分开处理：自定义 `base_url` 时把它作为一个额外
    // provider 插进映射表，同时选中它；否则用内建 OpenAI provider，映射表就是
    // codex 自带的那份。
    let mut model_providers = codex_core_api::built_in_model_providers(None);
    let (model_provider_id, model_provider): (String, ModelProviderInfo) = match &options.base_url {
        Some(base_url) => {
            let model_provider_id = "roc-desk-custom".to_string();
            let model_provider = ModelProviderInfo {
                name: "roc_desk custom provider".to_string(),
                base_url: Some(base_url.clone()),
                wire_api: codex_model_provider_info::WireApi::Responses,
                experimental_bearer_token: Some(options.api_key.as_str().into()),
                ..Default::default()
            };
            model_providers.insert(model_provider_id.clone(), model_provider.clone());
            (model_provider_id, model_provider)
        }
        None => {
            let model_provider_id = OPENAI_PROVIDER_ID.to_string();
            let model_provider = model_providers
                .get(&model_provider_id)
                .cloned()
                .ok_or_else(|| {
                    EngineError::Other("OpenAI model provider unavailable".to_string())
                })?;
            (model_provider_id, model_provider)
        }
    };

    let mut config = Config {
        config_layer_stack: Default::default(),
        startup_warnings: Vec::new(),
        bypass_hook_trust: false,
        model: options.model.clone(),
        service_tier: None,
        review_model: None,
        model_context_window: None,
        model_auto_compact_token_limit: None,
        model_auto_compact_token_limit_scope: Default::default(),
        model_provider_id,
        model_provider,
        personality: None,
        // 不让 codex 自己弹审批、也不套它自己的沙箱——真正的把关全在
        // `RocDeskExecServer` 那一层做（见文件头注释）。
        permissions: Permissions::from_approval_and_profile(
            Constrained::allow_any(AskForApproval::Never),
            Constrained::allow_any(PermissionProfile::Disabled),
        )
        .map_err(|e| EngineError::Other(format!("permissions: {e}")))?,
        explicit_permission_profile_mode: false,
        custom_permission_profiles: Vec::new(),
        approvals_reviewer: Default::default(),
        enforce_residency: Constrained::allow_any(None),
        hide_agent_reasoning: false,
        show_raw_agent_reasoning: false,
        base_instructions: None,
        base_instructions_provenance: None,
        developer_instructions: None,
        guardian_policy_config: None,
        include_permissions_instructions: false,
        include_apps_instructions: false,
        include_collaboration_mode_instructions: false,
        include_skill_instructions: false,
        skill_max_context_tokens: None,
        orchestrator_skills_enabled: false,
        orchestrator_mcp_enabled: false,
        include_environment_context: false,
        compact_prompt: None,
        notify: None,
        tui_notifications: Default::default(),
        animations: false,
        tui_whimsy: false,
        show_tooltips: false,
        tui_show_server_version_notice: false,
        tui_auto_recap: false,
        model_availability_nux: Default::default(),
        tui_vim_mode_default: false,
        tui_question_esc_back: false,
        tui_raw_output_mode: false,
        tui_alternate_screen: Default::default(),
        tui_status_line: None,
        tui_status_line_use_colors: false,
        tui_terminal_title: None,
        tui_theme: None,
        tui_pet: None,
        tui_pet_anchor: Default::default(),
        tui_session_picker_view: Default::default(),
        tui_resume_cwd: None,
        terminal_resize_reflow: Default::default(),
        tui_keymap: Default::default(),
        cwd: cwd.clone(),
        workspace_roots: vec![cwd],
        workspace_roots_explicit: false,
        cli_auth_credentials_store_mode: Default::default(),
        mcp_servers: Constrained::allow_any(HashMap::new()),
        non_prefixed_mcp_tool_servers: None,
        mcp_oauth_credentials_store_mode: Default::default(),
        mcp_oauth_callback_port: None,
        mcp_oauth_callback_url: None,
        mcp_optional_startup_grace: std::time::Duration::from_secs(1),
        model_providers,
        project_doc_max_bytes: 32 * 1024,
        project_doc_fallback_filenames: Vec::new(),
        tool_output_token_limit: None,
        agents_enabled: false,
        agent_max_threads: None,
        agent_default_subagent_model: None,
        agent_default_subagent_reasoning_effort: None,
        agent_interrupt_message_enabled: false,
        agent_max_depth: 1,
        agent_roles: Default::default(),
        max_goal_token_budget: None,
        memories: Default::default(),
        codex_home: codex_home.clone(),
        sqlite: codex_core_api::SqliteConfig::from_sqlite_home(codex_home.clone()),
        log_dir: codex_home.join("log").to_path_buf(),
        history: Default::default(),
        ephemeral: true,
        extra_config: None,
        file_opener: UriBasedFileOpener::None,
        codex_self_exe: Some(codex_self_exe),
        codex_linux_sandbox_exe: None,
        main_execve_wrapper_exe: None,
        zsh_path: None,
        model_reasoning_effort: None,
        plan_mode_reasoning_effort: None,
        model_reasoning_summary: None,
        model_catalog: None,
        model_verbosity: None,
        chatgpt_base_url: "https://chatgpt.com/backend-api/".to_string(),
        respect_system_proxy: false,
        apps_mcp_product_sku: None,
        responses_api_metadata: Default::default(),
        realtime_audio: Default::default(),
        experimental_realtime_ws_base_url: None,
        experimental_realtime_webrtc_call_base_url: None,
        experimental_realtime_ws_model: None,
        realtime: Default::default(),
        experimental_realtime_ws_backend_prompt: None,
        experimental_realtime_ws_startup_context: None,
        experimental_realtime_start_instructions: None,
        experimental_thread_store: Default::default(),
        forced_chatgpt_workspace_id: None,
        forced_login_method: None,
        web_search_mode: Constrained::allow_any(Default::default()),
        web_search_config: None,
        experimental_request_user_input_enabled: false,
        update_plan_enabled: true,
        tool_registry: Default::default(),
        code_mode: Default::default(),
        background_terminal_max_timeout: 300_000,
        thread_unload_delay: std::time::Duration::from_secs(60),
        ghost_snapshot: Default::default(),
        multi_agent_v2: Default::default(),
        token_budget: None,
        token_budget_startup_config: None,
        rollout_budget: None,
        current_time_reminder: None,
        sleep_tool_mode: Default::default(),
        features: Default::default(),
        suppress_unstable_features_warning: true,
        active_project: ProjectConfig { trust_level: None },
        notices: Default::default(),
        check_for_update_on_startup: false,
        disable_paste_burst: true,
        analytics_enabled: Some(false),
        feedback_enabled: false,
        tool_suggest: Default::default(),
        otel: Default::default(),
    };
    config
        .features
        .set(codex_core_api::Features::with_defaults())
        .map_err(|e| EngineError::Other(format!("features: {e}")))?;
    // `UnifiedExec`（连同 `UnifiedExecTty`/`UnifiedExecZshFork`）是 codex-core
    // 维护一个常驻交互式 shell 会话、通过 `process/write`（`write_stdin`）往里
    // 喂命令、`process/read` 流式读输出的工具，不是每条命令独立一次
    // `process/start`——`RocDeskExecServer` 一直只实现了"同步等命令跑完才应答"
    // 这一种模型（见 `exec_server.rs` 文件头注释），跟这套协议对不上（2026-09
    // 用户实测复现：SSH 远程目标下简单命令后端其实已经跑完，codex-core 自己却
    // 一直以为还在跑，反复 `write_stdin` 轮询拿不到"已完成"的信号）。
    // 上游默认会无视这里的 `.disable()`、强制重新打开 `UnifiedExec`（见
    // `vendor/codex/codex-rs/core/src/config/managed_features.rs::
    // normalize_candidate` 原本的逻辑），这条限制本来是"只有服务端下发的
    // managed requirements 能关掉"——因为 roc_desk 直接嵌入 vendor 源码、不需要
    // 跟上游同步，已经把那条强制重新打开的逻辑从 vendor 里删掉了，这里的
    // `.disable()` 现在是真的生效的：关闭后 `tools/spec_plan.rs::add_shell_tools`
    // 会改注册 `ExecCommandHandler::one_shot(...)`（codex-core 自带的"跑完就
    // 返回、不支持恢复"模式），正好匹配 exec-server 的实际能力。
    for feature in [
        codex_core_api::Feature::UnifiedExec,
        codex_core_api::Feature::UnifiedExecTty,
        codex_core_api::Feature::UnifiedExecZshFork,
    ] {
        config
            .features
            .disable(feature)
            .map_err(|e| EngineError::Other(format!("disable feature {feature:?}: {e}")))?;
    }
    Ok(config)
}
