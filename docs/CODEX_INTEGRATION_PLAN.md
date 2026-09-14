# 引入 codex-rs 重构 AI 编程助手模块 —— 方案（v2）

> 本文档设计如何把 OpenAI 开源的 Codex（`F:\code\开源\codex`，Rust workspace，Apache-2.0）深度嵌入 roc_desk，替换现有 AI 编程助手模块里"自己拼 HTTP 请求 + 手写工具循环"的对话引擎，同时保留 roc_desk 现有的 Diff/Accept/Undo 状态机、按轮次批量操作、完全授权模式、历史会话续聊、以及本地/SSH/Windows Agent 三态执行能力。
>
> 这是 v2 版本，基于两轮调研更新：第一轮确定了整体可行性和风险点，第二轮针对两边仓库的最新代码做了增量核实，其中一个发现改变了原方案的关键判断（见"§2 本轮最重要的新发现"）。v1 讨论过程中确定的选型：**双引擎架构**（codex 只接管 OpenAI/Azure/Bedrock/Ollama，国内 OpenAI 兼容渠道继续走现有自研实现）+ **深度源码嵌入**（`codex-core` 作为 path 依赖直接嵌入 roc_desk 进程，而不是包成 sidecar 子进程）。
>
> 本文档只是方案设计，尚未实现。落地后请按 REQUIREMENTS.md 的记录方式补充"已实现/部分实现/未实现"。
>
> **实现进度（持续更新）**：
> - **Phase 0（验证）已完成**——`codex-core`/`codex-exec-server-protocol` 已确认能作为 path 依赖编译进 roc_desk workspace，`Environment`/`EnvironmentManager` 的动态绑定 API 已找到并验证。
> - **Phase 1（`RocDeskExecServer` 骨架）已完成，编译通过**：`codex-engine` crate 的 `initialize`/`fs/readFile`/`fs/writeFile`/`fs/createDirectory`/`process/start`/`process/read`/`process/terminate` JSON-RPC/WebSocket server + `src-tauri/src/coding/codex_exec_target.rs` 的 `ExecTarget` 适配器，委派给现有 `ChangeStore`/`run_command_gated`，目前只验证过 `CodingTarget::Local`。
> - **Phase 3（`ChatEngine`/`CodexCoreEngine`）骨架 + 接入 `CodingSession` 均已完成，`cargo check -p roc_desk` 全量编译通过**：
  - `codex-engine/src/engine.rs` 里 `CodexCoreEngine` 参照官方 `thread-manager-sample` 驱动 `codex_core_api::ThreadManager`/`CodexThread`——构造近 130 字段的 `Config`（`Config` 不实现 `Default`，每个字段都要显式赋值）、用 `CodexAuth::from_api_key`+`AuthManager::from_auth_for_testing_with_home` 注入 roc_desk 自己管理的 API Key（绕开 codex 自己的 auth.json/ChatGPT 登录）、`EnvironmentManager::upsert_environment_with_options` 注册指向 `RocDeskExecServer` 的 environment、`ThreadSettingsOverrides.environments` 把 turn 绑定过去、`thread.start_turn_if_idle`/`thread.next_event()` 循环驱动一轮对话。
  - `commands/coding.rs::attach_codex_engine` + `coding::routes_to_codex_engine`（Phase 6 的路由启发式，按 `api_base` 字符串特征猜是不是 OpenAI/Azure/Bedrock/Ollama）在 `build_new_session` 里挂载：provider 命中路由、目标是 `CodingTarget::Local`、且能拿到 API Key 时才尝试挂载，任何一步失败都静默回退到自研引擎（不让这条新路径的问题拖垮"开始 AI 会话"本身）。
  - `CodingSession::send_message` 顶部分支：`codex_engine` 字段非空时整段绕开下面的自研 HTTP+工具循环，交给 `CodexCoreEngine::run_turn` 跑，通过 `TauriEventSink`（`impl codex_engine::EngineEventSink`）把助手文本/推理过程映射回和自研引擎完全一样的 `coding:assistant-note` 事件，前端不需要为这条路由改任何代码。
  - **一个重要的架构细节**：`codex_exec_target::SessionExecTarget`（codex-core 通过 exec-server 协议回调文件读写/命令执行时落地的地方）**不持有 `Arc<Mutex<CodingSession>>`**——因为这些回调发生在 `send_message` 已经用 `&mut self` 拿着会话、正 `await` `thread.next_event()` 的时候，再抢同一把锁会死锁。为此把 `CodingSession::run_command_gated` 的实际逻辑拆成自由函数 `run_command_gated_shared`（不需要 `&CodingSession`），`SessionExecTarget` 直接持有 `Arc<Mutex<ChangeStore>>`（本来就是独立加锁的）+ 一份 `Arc<StdMutex<Uuid>>` 共享 `turn_id` + 各种池/registry 的 `Arc` 克隆，绕开了整个重入问题。
  - **产物体积的真实数据**：`bin\roc_desk.exe` 从引入前的约 38MB 涨到约 179MB（`build-portable.ps1` 完整构建实测），这是深度嵌入 codex-core 全量依赖树（sqlx/gix/tonic/opentelemetry 等）静态链接进同一个二进制的直接代价，符合 v1 讨论时的预期方向但现在有了具体数字，如果后续觉得这个体积增量不可接受，是重新评估"深度嵌入 vs sidecar"这个选型的一个具体依据。
  - **已知限制（如实记录，不是"基本完成"）**：① 只支持 `CodingTarget::Local`——SSH/Agent 目标下 `PathUri::from_host_native_path` 对"远端路径"（可能是别的操作系统的路径语法）处理方式没有验证过，直接在 `build_new_session` 里用 `matches!(session.target, CodingTarget::Local)` 挡掉了，其余目标一律走自研引擎；② `auto_allow_readonly` 对 codex 路由固定传 `false`（只读命令也会弹确认框，不影响正确性，只是少一个便利开关）；③ codex 路由产生的会话历史，`coding_history_resume`（恢复历史会话继续对话）目前**不会正确工作**——codex-core 自己的对话状态存在 `ThreadManager`/`CodexThread` 内部，没有做到 roc_desk `messages_json` 的双向桥接（§2.4 已经分析过这个不能简单靠"转成 chat messages"解决），这是 Phase 5 遗留的真实缺口，不是遗漏。
>
> **运行时验证（用真实自定义 OpenAI 兼容端点做的端到端烟测，`scratchpad/codex_smoke` 独立程序，不在仓库里）——过程中发现并修复了两个纯类型检查完全测不出来的真实 bug**：
> 1. **`codex_self_exe` 是硬性必填字段**：`ExecServerRuntimePaths::from_optional_paths` 在这个字段是 `None` 时直接报错"Codex executable path is not configured"，哪怕从不触发本地 arg0 自派发也不行——这是构造 `EnvironmentManager`/`ThreadManager` 链路上的一个必经步骤。修法是用 `std::env::current_exe()` 占位（只要不触发 `--codex-run-as-apply-patch` 这类特殊 argv，内容本身不影响行为）。
> 2. **栈溢出，进程级致命错误**（这一个是真正的坑，必须在合入生产代码前修掉）：`ThreadManager::start_thread`/`CodexThread::next_event` 这条 codex-core 内部深层异步调用链，在默认线程栈大小下会直接 `STATUS_STACK_OVERFLOW` 让整个进程崩溃——不是能 `catch`/`Result` 处理的错误。已确认**只调大 tokio worker 线程栈不够**（`Builder::thread_stack_size` 只影响 worker 线程，`block_on` 挂的根 Future 实际跑在调用它的那个线程本身上）；codex 自己的 TUI 那边也有一条"把线程栈调到 12MiB"的提交，说明这不是 roc_desk 这边独有的问题。**最终修法**：新增 `codex_engine::CodexCoreEngineHandle`（`codex-engine/src/engine.rs`），内部另起一个手动指定 64MiB 栈的专用线程 + 独立 tokio runtime（worker 线程栈也调到 32MiB）常驻跑 `CodexCoreEngine`，外部通过 channel（`tokio::sync::mpsc`/`oneshot`）收发请求——`CodingSession`/Tauri command 的正常异步上下文永远不直接执行 codex-core 的深层调用链，`session.rs`/`commands/coding.rs` 相应改成用这个 handle 而不是裸的 `CodexCoreEngine`。
> 3. **端到端结果**：修完上面两个问题后，`CodexCoreEngineHandle::spawn` 构造成功、`run_turn` 真正提交了一轮对话请求到用户提供的自定义端点（`http://10.202.81.216:8317/v1`，`gpt-5.6-sol`），**拿到了服务端返回的真实响应**——鉴权、Responses API 协议握手、HTTP 通信全部验证通过；返回内容是模型侧的"Selected model is at capacity"（服务当前繁忙），不是我们这边的错误，说明整条链路是通的，只是还没在这次验证里拿到一次真正的对话内容。**这是目前为止唯一一次真实的运行时验证**，验证范围仅覆盖"文本对话往返"，还没覆盖"模型实际调用 write_file/run_command 工具、触发 `RocDeskExecServer` 的 `fs/writeFile`/`process/start` 回调"这条路径。
>
> **Phase 3 踩的坑（新增）**：
> 1. **`rusqlite`/`libsqlite3-sys` 版本冲突**：`codex-core-api` 依赖的 `codex-state`（codex 自己的会话状态 SQLite 存储）需要 `libsqlite3-sys ^0.37`，roc_desk 原来的 `rusqlite = "0.32"` 只能到 `^0.30`——Cargo 的 `links = "sqlite3"` 约束整个依赖图只能有一个版本，直接编译失败。已把 `src-tauri/Cargo.toml` 的 `rusqlite` 提到 `0.39`、`r2d2_sqlite` 提到 `0.34`（这个精确组合是试出来的：`rusqlite` 0.39.x 依赖 `libsqlite3-sys ^0.37.0`，`r2d2_sqlite` 0.34.x 依赖 `rusqlite ^0.39`，跳到 0.40 会连带把 libsqlite3-sys 带到 `^0.38`，和 codex 对不上），rusqlite/r2d2_sqlite 在这几个版本间 API 不变，不需要动 `db/` 下任何调用代码。
> 2. **Cargo feature unification 的坑，和"版本漂移"是两类不同问题**：`codex-utils-pty`（codex-core 依赖图里的 ConPTY 支持，我们自己用不上但绕不开）里直接抄自 wezterm 的 Windows 代码假设 `winapi::ctypes::c_void` 就是 `std::ffi::c_void`——这只在 `winapi` crate 开了 `"std"` feature 时成立。codex-rs 自己的完整 workspace 里，別的 crate 顺带打开了这个 feature（Cargo 的 feature unification 是"整个依赖图里任何一处要，大家都得到"），我们只 path 依赖了一小部分 codex crate，没有那个"顺带"的 crate，这个 feature 就没被打开，编译报"两个长得像但不同的 c_void 类型"。修法是在 `codex-engine/Cargo.toml` 显式加一份 `winapi = { version = "0.3.9", features = ["std"] }`（`[target.'cfg(windows)'.dependencies]` 下），靠 feature unification 传给 `codex-utils-pty` 用的那份 winapi，不用碰 vendor 目录里的源码。**这类问题以后大概率还会遇到——只 path 依赖 codex-rs 一部分 crate、脱离它自己完整 workspace 的 feature 组合，是这条集成路线的结构性代价，不是一次性踩完的坑**。
> 3. `AuthManager::from_auth_for_testing_with_home`（`codex-login` crate）是目前唯一能"直接注入一个 API Key，不走 codex 自己的 auth.json/keyring/ChatGPT 登录流程"的公开入口，虽然命名是 `for_testing` 且明确写着"for testing only"，但没有 `#[cfg(test)]`门禁，可以正常调用。这是目前唯一可行的桥接方式，但要接受它可能随时被改名/收紧可见性的风险——跟着 codex 升级时优先检查这个 API 还在不在。
>
> **本地构建环境的一个坑**：验证过程中遇到过 `num-traits`（`chrono` 间接依赖，和这次改动无关的既有基础库）的构建脚本二进制稳定复现"拒绝访问"（os error 5）——反复重试、换 Bash/PowerShell、单独跑都无法绕开，Windows Defender 自身的检测 API 也查不到拦截记录，把 `num-traits` 换成 `0.2.18`（`cargo update -p num-traits --precise 0.2.18`，生成字节内容不同的二进制）后问题消失，符合"某个安全软件对特定文件哈希做了缓存级拦截"的特征。如果以后在同一台机器上重新拉 `build/` 目录全量构建时又遇到类似"某个无关基础库的 build script 稳定拒绝访问"，先按这个思路试一下换版本，而不是当成代码问题排查。

---

## 一、roc_desk 现状基线（这是"不能退化"的能力清单）

AI 编程助手模块最近做过一次锁拆分重构，现状比较成熟，重构 codex 引擎时必须原样保留或提供等价能力：

- **对话循环与文件改动状态机已经拆成两把独立锁**：`AppState.coding_sessions`（对话/LLM 上下文，`CodingSession`）和 `AppState.coding_changes`（文件改动状态机，`ChangeStore`，`src-tauri/src/coding/changes.rs`），两者用同一个 workspace id 做 key 但互不阻塞。这是为了修一个真实 bug——原来共用一把锁时，agentic loop 跑一两分钟会导致用户点"应用/拒绝"卡死。**任何重构都不能把这两把锁重新合并**。
- **`FileChange`/`ChangeStatus{Pending,Applied,Rejected,Undone}`/`FileSyncInfo`/`turn_id`** 是稳定的对外契约，前端强依赖，且明确要求**不依赖 git**（[[feedback-ai-undo-no-git]]，REQUIREMENTS.md 有原话）。`ChangeStore::stage/accept/reject/undo/redo/revert_turn` 是这套状态机的核心操作，`revert_turn` 按 `turn_id` 倒序批量撤销同一轮改动。
- **三态执行已经统一**：`CodingTarget::Local/Remote(SSH)/Agent(远程 Windows Agent)` 三种目标，命令执行统一走 `run_command_gated`（权限规则引擎 `guard.rs`/`permission.rs` + 命令确认弹窗），文件读写统一走 `FileOps` trait 的三个实现（`local.rs`/`remote.rs`/`fsops/agent.rs`）。`CodingSession`/`ChangeStore` 对三种执行后端完全无感知。**这一层要原样复用**，不要在引入 codex 后被绕过。
- **`full_auto`（完全授权模式）**：开启后 `stage_change` 立即写盘，不停留在 Pending。
- **消息级持久化**：`coding_history.messages_json`（迁移 `0017`），`CodingSession::messages_snapshot()/restore_messages()`，配合 `coding_history_resume` 命令支持"打开历史会话继续聊"，不再是只读回放。
- **内存保护**：`MAX_TOOL_RESULT_CHARS=50_000` 截断、`read_file_for_editor` 体积感知读取——修的是"大文件读满内存导致分配器 abort、窗口静默消失"的真实故障，重构后新引擎链路也要保留等价保护。
- **Provider 抽象是纯 OpenAI 兼容协议的最小公分母**（`AiProviderManager`：name/api_base/api_key_ref/model/is_local），豆包/DeepSeek/通义千问/本地 Ollama 都靠这个统一接入，这是需要双引擎架构的根本原因。

## 二、codex 侧关键结论（含本轮最重要的新发现）

### 2.1 workspace 结构、License、依赖隔离（v1 结论，本轮复核依然成立）

- Rust workspace 在 `codex-rs/`，140+ crate，Apache-2.0，可合法 vendor 源码（保留 LICENSE/NOTICE 即可，无 copyleft 传染性）。
- `rusty_v8` 只被 `code-mode-runtime`/`v8-poc` 两个可选 crate 依赖，主链路（`codex-core`/`codex-protocol`/`codex-exec`/`codex-app-server`）不碰，可以完全规避 V8 编译负担。
- 模型协议层 `WireApi` **现在只剩一个变体 `Responses`**（`Chat` 早已被删除并显式报错），只支持 OpenAI 官方 / Azure OpenAI / Amazon Bedrock 兼容端点 / Ollama / LM Studio，**没有 Anthropic/Claude 支持，也没有通用 chat-completions 抽象**——这是选择双引擎架构的直接依据，本轮复核证据更充分（枚举定义已物理上只有一个成员）。

### 2.2 本轮最重要的新发现：执行/文件系统的可插拔扩展点是协议级的，不是 Rust trait 级的

v1 遗留的最大疑问——"codex-core 能不能把命令执行、文件读写请求拦下来转发给 roc_desk 自己的执行器"——本轮找到了确切答案，而且这个答案会改变"深度嵌入 vs sidecar"这个选型的成本收益比：

- codex 早就有一个专门的 crate `codex-rs/exec-server/`（`codex-exec-server`），定义了一套**独立的 JSON-RPC 协议**（`initialize`/`process/start`/`process/read`/`process/write`/`process/terminate`/`fs/readFile`/`fs/writeFile`/`fs/createDirectory` 等），通过 WebSocket（本地 `ws://IP:PORT`，远程走 Noise 加密中继或 AWS SigV4 直连）通信。
- `codex-core` 内部用 `Environment`/`EnvironmentManager`（`codex-rs/core/src/environment_selection.rs`）把"turn 在哪个环境里执行"抽象成一个可绑定的目标：默认是 `LOCAL_ENVIRONMENT_ID`（本地子进程 + 沙箱），也可以绑定到任意实现了这套协议的远程/自定义 environment。
- **也就是说：无论 codex-core 是被深度嵌入进 roc_desk 进程，还是作为独立 sidecar 进程跑，"接管执行"这件事的做法是完全一样的**——都要求 roc_desk 自己实现一个 `codex-exec-server` 协议的 server（WebSocket 服务，哪怕只在 `127.0.0.1` 上监听），把 `Environment` 绑定到这个 server，而不是去 hook `codex-core` 内部的 Rust 函数调用。这个扩展点不受"要不要深度嵌入"这个选择影响。

**这对原方案的影响**：v1 认为"深度嵌入才能拿到执行接管能力"这个假设不成立了——两条路径在执行接管这件事上成本相同。深度嵌入相对 sidecar 剩下的唯一优势是"省一次进程间通信、直接拿到 Rust 内部事件流"，但代价依然是：`codex-core` 内部 API 不稳定（`Conversation`→`Thread` 已经改过一次名字，两个月内又新增了 `codex-attachment-store`/`codex-guardian-context` 两个新依赖，说明这个 crate 演化很快）、依赖树重、且如果绑定 `LOCAL_ENVIRONMENT_ID` 还要复刻 arg0 自派发逻辑。**建议**：既然执行接管不再是深度嵌入的独占优势，深度嵌入的性价比比 v1 评估时更低了。本文档仍按你在 v1 选定的"深度嵌入"写方案（见 §四），但把 sidecar（跑官方 `codex-app-server` 二进制 + JSON-RPC）列为**备选降级路径**（§六），如果 Phase 0 验证下来 `codex-core` 直接依赖成本过高（编译时间/体积/API churn），可以随时切换到 sidecar 而不影响 §四 里"RocDeskExecServer"这部分设计——它对两条路径都成立。

### 2.3 一个连带的好消息：不绑定 `LOCAL_ENVIRONMENT_ID` 不需要 arg0 自派发和 codex 自己的沙箱（已验证）

只要 turn 绑定到 roc_desk 自己实现的 exec-server environment，`codex-core` 就不会走它自己的本地子进程沙箱和 arg0 自派发——这不是推测，是读源码验证过的：`exec_command`/`unified_exec`（`core/src/unified_exec/process_manager.rs`）和 `apply_patch`（`core/src/tools/runtimes/apply_patch.rs`）两条路径，都是通过`req.turn_environment.environment.get_exec_backend()`/`get_filesystem()` 拿后端句柄，没有任何硬编码走本地的分支；`--codex-run-as-apply-patch` 自派发只在"legacy 模式把 apply_patch 当普通 shell 命令跑"且**该 turn 绑定的就是本地环境**时才触发，与 roc_desk 的自定义 environment 完全无关。vendor 范围维持 §四表格：不需要引入 `codex-sandboxing`/`windows-sandbox-rs`/`codex-apply-patch`，也不需要在 `main()` 里复刻 arg0 分发。

### 2.4 Phase 0 核心 API 已确认（精读 `exec-server`/`environment_selection.rs` 源码得到的结论）

- **`Environment` 是一个 struct，不是 trait**，公开构造函数（`Environment::create`）只接受"WebSocket URL / 子进程 stdio 命令 / Noise rendezvous"三种连接方式，**没有"进程内直接注入自定义执行后端"的公开入口**。也就是说 §3.2 设想的 `RocDeskExecServer`，必须是一个真正实现 exec-server JSON-RPC 协议的 server（哪怕只监听 `127.0.0.1` 的 loopback WebSocket），不能靠实现某个 Rust trait 绕开协议——这一点排除了"更省事"的备选做法，直接按 §3.2 的协议 server 方案做。
- **动态绑定 API**（不需要 `environments.toml` 配置文件，纯编程方式）：
  ```rust
  environment_manager.upsert_environment_with_options(
      environment_id: String,           // 自定义 id，比如按 workspace_id 生成
      RemoteEnvironmentOptions { exec_server_url: "ws://127.0.0.1:<port>".into(), connect_timeout: None, http_headers: HashMap::new() },
  )?;
  ```
  然后通过提交携带 `TurnEnvironmentSelection { environment_id, cwd, workspace_roots, config }` 的 `Op`（`ThreadSettingsOverrides.environments: TurnEnvironmentSelections`）把某个 turn 绑定过去；找不到对应 id 时 `codex-core` 只会 `tracing::warn!` 跳过，**不会静默退化成本地环境**，roc_desk 必须保证先注册再引用。
- **`codex-exec-server` crate 不提供可实现的 server 端 trait**：它导出的是一个焊死"本地执行"的完整可执行入口（`run_main`），内部 handler/连接处理器全是 `pub(crate)`。roc_desk 能复用的只是 `codex-exec-server`/`codex-exec-server-protocol` 导出的**协议消息类型**（`ExecParams`/`ExecResponse`/`FsReadFileParams`/`FsWriteFileParams`/`InitializeParams`/`ProcessOutputChunk` 等），JSON-RPC 分发、WebSocket 帧处理、连接管理都要 roc_desk 自己手写。
- **会话历史桥接比 v1 设想的更麻烦**：codex-core 内部对话历史的核心类型是 `codex_protocol::models::ResponseItem`（对齐 OpenAI **Responses API** 的 item 模型：`Message`/`Reasoning`/`FunctionCall`/`FunctionCallOutput`/...），不是 Chat Completions 那种扁平 `{role, content}` messages 数组，仓库里**没有现成的 `ResponseItem ⇄ chat messages JSON` 转换函数**。§3.3 的桥接方案改为：`CodexCoreEngine` 落库时直接序列化 `ResponseItem`/`RolloutItem` 结构本身（而不是转换成通用 chat messages 格式），`coding_history.messages_json` 这一列对不同引擎产生的数据允许是不同的内部格式，只要各自引擎能正确从自己写的格式里 `restore` 即可——不强求两个引擎的历史格式一致。

## 三、目标架构

### 3.1 双引擎路由（v1 设计不变）

```
CodingSession
  └── engine: Box<dyn ChatEngine>          // 按 provider 的 wire 协议特征选择实现
        ├── LegacyOpenAiCompatEngine        // 现状原样保留：豆包/DeepSeek/通义千问/自定义 OpenAI 兼容渠道
        └── CodexCoreEngine                 // 新增：OpenAI 官方/Azure OpenAI/Bedrock/Ollama/LM Studio
```

`ChatEngine::run_turn` 通过现有的 Tauri 事件广播机制对外表现一致（`coding:tool-call-start/end`、`coding:assistant-note`、`coding:file-change` 等），前端 `CodingAgentPanel.tsx`/`codingStore.ts`、Tauri command 层（`commands/coding.rs`）、`ChangeStore` 状态机**不需要感知引擎差异**。

### 3.2 RocDeskExecServer —— 把三态执行 + Pending 状态机塞进 codex 的"文件系统/进程"协议里

这是整个方案里最关键、也是本轮调研后新确定下来的设计：**不去解析 codex 的 `apply_patch` diff 格式，也不对接它的 `execCommandApproval`/`applyPatchApproval` 审批 RPC**，而是在协议更底层的位置——`codex-exec-server` 的 `fs/*`、`process/*` handler 里——把 roc_desk 现有能力接进去：

```
codex-core 的 turn 绑定到 environment = ws://127.0.0.1:<port>/<session_id>
                                            │
                                            ▼
                              RocDeskExecServer（roc_desk 新增，本地 WebSocket server）
                                            │
                    ┌───────────────────────┼───────────────────────┐
                    ▼                       ▼                       ▼
             fs/readFile              fs/writeFile             process/start
                    │                       │                       │
                    ▼                       ▼                       ▼
     pending_content_for(path)      ChangeStore::stage(...)   run_command_gated(...)
     （复用现有语义：模型能看到     （不落盘！立即回复 codex   （复用现有权限规则引擎
      未落盘但已提议的最新内容）    "写入成功"，实际生成一条    +CommandConfirmDialog+
                                    Pending 的 FileChange，     三态路由 Local/SSH/Agent）
                                    真正落盘延后到用户点 Accept）
```

这样设计的好处：

1. **完全复用现有 `ChangeStore`/`FileChange`/`turn_id`/`revert_turn`/`full_auto` 状态机**，不需要再造一套"codex 版本的 diff 审批 UI"，也不需要写 codex patch 格式 ↔ roc_desk FileChange 格式的转换器。codex 眼里看到的就是一个普通、每次都成功的文件系统，"审批"这件事对它完全透明——真正的把关在 roc_desk 自己的 exec-server 实现里。
2. **三态执行（Local/SSH/Agent）原样复用**：`process/start` 直接调用已有的 `run_command_gated`，内部路由到 `CodingTarget` 对应的执行后端，`process/output` 通知把执行结果流式传回 codex。远程执行能力（SSH、Windows Agent）不需要重新实现，也不需要等 codex 自己的"remote environment"机制成熟。
3. **`full_auto` 模式**：`fs/writeFile` handler 检测到 `full_auto` 开启时跳过 Pending，直接调用 `ChangeStore` 里等价于"stage 后立即 accept"的路径，语义和现状完全一致。
4. **不需要 codex 自己的本地沙箱、arg0 自派发、`codex-apply-patch` 解析器**——如 §2.3 所述，只要 turn 不绑定本地环境，这些大概率都用不上，vendor 范围进一步缩小。
5. 这套 exec-server 实现是**双引擎共享的基础设施**，不管 Phase 0 之后最终是深度嵌入 `codex-core` 还是退化到 sidecar `codex-app-server`，`RocDeskExecServer` 都不用改。

### 3.3 消息/会话持久化桥接

codex 有自己的 `codex-rollout`/`codex-thread-store` 持久化机制，**不采用**——继续用 roc_desk 自己的 `coding_history.messages_json` + `CodingSession::messages_snapshot()/restore_messages()`。`CodexCoreEngine` 需要做的适配：

- 每轮结束后，把 codex 内部的 Thread/Item 历史转换成 roc_desk 现有的 `messages: Vec<serde_json::Value>` 格式落库（供 `coding_history_save` 复用，格式不必和 legacy 引擎产生的完全一致，但要能在 `coding_history_resume` 时正确还原上下文）。
- `coding_history_resume` 续聊一个由 `CodexCoreEngine` 产生的历史会话时，需要能把存量 `messages_json` 转换回 codex 能理解的上下文（大概率是重新构造一个 Thread 并把历史消息作为初始上下文注入，具体机制取决于 `codex-core` 的 Thread/Item API，需要 Phase 0 验证）。

## 四、vendor 范围（本轮更新，比 v1 更小）

| 引入 | 不引入 | 原因 |
|---|---|---|
| `codex-protocol`（Op/Event/Submission 数据结构） | `codex-code-mode-runtime`/`v8-poc` | V8 隔离，主链路不需要 |
| `codex-core`（核心引擎） | `codex-rollout`/`codex-thread-store` | roc_desk 自己的 SQLite 会话历史，不引入第二套持久化 |
| `codex-exec-server` 协议相关的**类型定义部分**（如果它导出了 server 端 scaffolding；否则只参照它的 JSON-RPC schema 自己实现协议，不依赖这个 crate） | `codex-sandboxing`/`codex-windows-sandbox-rs`/`bwrap` | 不绑定本地环境，大概率用不上（Phase 0 验证） |
| （按需）`model-provider-info` 里 OpenAI/Azure/Bedrock/Ollama 相关的 provider 元数据 | `codex-apply-patch` | 改走 exec-server fs 层拦截方案，不需要解析 codex 的 patch 格式 |
| | `codex-app-server*`、`codex-mcp*`、`codex-tui`、`codex-cli` | 深度嵌入路线用不上（sidecar 备选路径才需要 `codex-app-server`） |
| | plugin marketplace（`plugin/list\|install\|reconcile\|search\|share*`）、`user-verification`（experimental 生物识别/passkey）、`turn_cost_otel`（ChatGPT 账号成本遥测）、`remote_control`/`remote_thread_store`（多端会话同步） | 本轮调研确认这些都是和"接管执行/对接模型"无关的独立子系统（插件市场、身份验证、可观测性、多端协控），不在本次重构范围 |

## 五、分阶段计划

- **Phase 0（验证，不动 roc_desk 主代码）**：
  1. 确认 `codex-exec-server` 有没有导出 server 端可直接实现的 trait/scaffolding，还是只有协议文档、需要手写 WebSocket + JSON-RPC server（决定 `RocDeskExecServer` 的实现成本）。
  2. 验证 `codex-core` 能否在编程方式下把一个 turn/thread 绑定到自定义 environment（而不是通过 `environments.toml` 配置文件），确认所需的 API 调用序列。
  3. 验证绑定自定义 environment 后，`codex-core` 是否真的跳过本地沙箱/arg0 分发（§2.3 假设）。
  4. 用一个独立 spike 项目把 `codex-core`/`codex-protocol` 作为 path 依赖引入，实测编译时间增量、`roc_desk.exe` 体积增量、有无版本冲突（`tokio`/`reqwest`/`serde` 等）。
  5. 确认 codex 内部 Thread/Item 历史结构，评估和 `messages_json` 互转的可行性（§3.3）。
  
  这一步结束时要能回答清楚 1、2、3——回答不了就不要进 Phase 1，或者考虑切到 §六 的 sidecar 备选路径。

- **Phase 1（RocDeskExecServer 骨架）**：先只支持 `CodingTarget::Local`，实现 `fs/readFile`（接 `pending_content_for`）、`fs/writeFile`（接 `ChangeStore::stage`）、`process/start`（接 `run_command_gated`），脱离 codex 独立自测（用任意 JSON-RPC 客户端手工验证协议行为）。

- **Phase 2（双引擎骨架）**：抽出 `ChatEngine` trait，`LegacyOpenAiCompatEngine` 包一层现有 `session.rs` 逻辑，先不改行为，确认现状零回归。

- **Phase 3（CodexCoreEngine 接入，本地场景跑通）**——**骨架已完成并编译通过**（`codex-engine/src/engine.rs`），**剩余工作**：`CodexCoreEngine` 目前是独立可构造、可编译的单元，还没有接进 `CodingSession`（需要定义 `ChatEngine` 双引擎路由、`CodingSession` 持有 `Box<dyn ChatEngine>`）；也还没有用真实 API Key 端到端跑过一次对话（只验证过类型检查，`ThreadManager::new`/`start_thread`/`start_turn_if_idle`/`next_event` 这条链路的运行时行为还未实际验证过）；`RocDeskExecServer` 和 `CodexCoreEngine` 也还没有真正串起来跑通"发消息 → 工具调用 → 文件写入被拦截进 `ChangeStore`（Pending）→ 用户点 Accept → 真正落盘 → 编辑器 buffer 同步"全链路，仅 Local 目标。

- **Phase 4（三态执行补全）**：`RocDeskExecServer` 的 `process/start`/`fs/*` 扩展到 SSH/Agent 两种目标，复用现有 `FileOps` 三个实现，交叉测试确认远程场景下语义一致。

- **Phase 5（既有功能对齐）**：`turn_id` 分组/`revert_turn`/`full_auto`/历史续聊（`messages_json` 互转）在 `CodexCoreEngine` 下逐项验证，和 `LegacyOpenAiCompatEngine` 做对比测试（同一 prompt 分别在两个引擎下跑，比较 `FileChange` 记录是否等价）。

- **Phase 6（默认路由，不设开关）**：`AiProviderManager` 按 provider 的 `api_base`/协议特征直接路由到对应引擎，代码内置默认行为，不加手动开关——OpenAI 官方/Azure/Bedrock/Ollama/LM Studio 渠道默认直接走 `CodexCoreEngine`，豆包/DeepSeek/通义千问/自定义 OpenAI 兼容渠道默认直接走 `LegacyOpenAiCompatEngine`。

- **Phase 7（收尾）**：更新 `CLAUDE.md`/`build-portable.ps1`（如果 vendor codex 源码后编译流程变复杂，比如需要额外的构建步骤或工具链要求，要写进构建脚本或文档）、`vendor/codex` 下保留 LICENSE/NOTICE 声明、`RELEASING.md` 视情况补充说明。

## 六、备选路径：sidecar + 官方 `codex-app-server`

如果 Phase 0 验证下来深度嵌入 `codex-core` 的成本（编译时间、体积、API churn 频率）超出预期，可以整体切到这条路径而不影响 §3.2 的 `RocDeskExecServer` 设计：

- 把官方 `codex-app-server`（或 `codex` 主二进制的 `app-server` 子命令）编译成独立可执行文件，作为 Tauri sidecar 放进 `bin\`（类似现有 `roc_desk_agent.exe`/`wfreerdp.exe` 的模式），Tauri 后端通过 stdio + JSON-RPC(JSONL) 与它通信，用官方 `codex app-server generate-ts` 产出类型定义。
- `ChatEngine` 的 `CodexCoreEngine` 实现改为通过这套 JSON-RPC 协议驱动 Thread/Turn，而不是直接调 Rust API；`RocDeskExecServer` 不变，只是现在 environment 被一个独立进程而非同进程内的 codex-core 绑定。
- 这条路径的优势是跟随官方升级成本更低（app-server 协议比 `codex-core` 内部 Rust API 稳定得多，VSCode 插件在用同一套协议），代价是多一层 IPC。

## 七、明确不做的事情

- 不使用 codex 自己的本地沙箱（Landlock/seccomp/`windows-sandbox-rs`/`sandbox-exec`）和 arg0 自派发——继续用 roc_desk 现有的权限规则引擎 + 确认弹窗 + 本地/SSH/Agent 三态执行。
- 不使用 codex 的 `apply_patch` diff 格式解析——改在 `RocDeskExecServer` 的 `fs/writeFile` 层拦截，复用现有 `ChangeStore`。
- 不使用 codex 的 `codex-rollout`/`codex-thread-store` 持久化——继续用 roc_desk 自己的 SQLite（`coding_history.messages_json`）。
- 不做 Claude/Anthropic 支持的协议转换代理——codex 协议层写死 OpenAI Responses API，这块需求留给 `LegacyOpenAiCompatEngine`；如果以后要让 Claude 走 codex 引擎，需要单独立项做协议转换代理，不在本次范围。
- 不引入 codex 的插件市场（plugin marketplace）、`user-verification`（experimental 生物识别/passkey）、`turn_cost_otel`（ChatGPT 账号成本遥测）、`remote_control`/`remote_thread_store`（多端会话协控/同步）——这些是和"接管执行/对接模型"无关的独立子系统。
- 不引入 `code-mode`/V8、企业云任务、SSO 等和"本地编程助手"无关的 codex 子系统。

## 八、风险清单

| 风险 | 状态 | 说明 |
|---|---|---|
| codex-core 内部 Rust API 不稳定 | **未解决，深度嵌入路线的主要长期成本** | 两个月内已发生一次 `Conversation→Thread` 重命名 + 新增两个内部依赖，需要接受"跟随上游改动要持续维护"的现实，或走 §六 sidecar 路径规避 |
| exec-server server 端实现成本 | **待 Phase 0 验证** | 取决于该 crate 是否导出可直接用的 server scaffolding |
| 自定义 environment 是否真能跳过本地沙箱/arg0 分发 | **待 Phase 0 验证** | 直接决定 vendor 范围能不能按 §四 缩小 |
| Cargo workspace 相容性（版本冲突/编译时间/体积） | **已验证，有两类必踩的坑** | 1) 把 `vendor/codex` 嵌套进 roc_desk 自己的仓库目录树后，roc_desk 根 `Cargo.toml` 的 `[workspace]` 必须加 `exclude = ["vendor/codex"]`，否则 cargo 会把 codex-rs 自己的（嵌套）workspace 误当成本 workspace 隐式成员，`edition.workspace = true` 之类的继承字段会对着错误的 workspace 根解析，直接报错。2) 因为我们不消费 codex-rs 自己钉死版本的 `Cargo.lock`、而是在自己的 workspace 里独立解析依赖，凡是 codex-rs 依赖树里带 `-alpha`/`-beta` 之类预发布版本号、又被其他 crate 用宽松 semver（如 `^0.3.0`）要求的库（实测踩到的是 `rama-*` 系列 16 个 crate，全部钉在 `0.3.0-alpha.4`），我们自己解析时会飘到语义上"更新"但 API 不兼容的正式版（如 `0.3.0`），导致编译报"找不到某个类型"这种看起来毫不相关的错误。修法是照着 `vendor/codex/codex-rs/Cargo.lock` 里的版本号，对每个飘掉的 crate 跑 `cargo update -p <crate> --precise <version>` 手动摁回去——跟着 codex 升级 vendor 版本后，如果编译报奇怪的类型/方法找不到的错，先去对一遍这个文件里 pin 的版本号，而不是当成 roc_desk 自己代码的 bug 排查。 |
| codex 内部会话历史结构 ↔ roc_desk `messages_json` 互转 | **待 Phase 0/Phase 5 验证** | 影响历史续聊功能在 codex 引擎下的可行性 |
| License/合规 | **已确认无阻碍** | Apache-2.0，vendor 时保留 LICENSE/NOTICE 即可 |
| 多渠道兼容性（豆包/DeepSeek/通义千问/Ollama） | **已通过双引擎架构规避** | codex 只接管 OpenAI 系渠道，其余渠道行为零变化 |
