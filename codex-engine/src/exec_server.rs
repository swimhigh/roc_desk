//! `RocDeskExecServer`：codex-core 的 `Environment` 抽象要求的 exec-server
//! JSON-RPC 协议的 server 端实现（协议本身来自 `codex-exec-server-protocol`，
//! 见 vendor/codex/codex-rs/exec-server/README.md）。
//!
//! codex 官方的 `codex-exec-server` crate 只导出了一个"焊死本地执行"的完整
//! 可执行入口（`run_main`），内部 handler/连接管理全是 `pub(crate)`，没有给
//! 第三方留可实现的 server trait——这是 docs/CODEX_INTEGRATION_PLAN.md Phase 0
//! 调研确认过的结论。所以这里的 JSON-RPC 分发、WebSocket 帧收发都是照着协议
//! 消息类型（`codex_exec_server_protocol`）自己写的，只把"读写文件/跑命令"这
//! 两件事委派给 `ExecTarget`。
//!
//! 当前实现的执行模型是"同步等到命令跑完才应答"（`process/start` 内部直接
//! `await` 完 `ExecTarget::run_command` 才回包），不支持真正的流式/交互式
//! 长驻进程（`process/write`/`process/signal` 目前是no-op）——这和 roc_desk
//! 现有自研引擎的 `run_command` 工具、以及 Windows Agent 的一次性命令执行
//! 原语（AGENT_DESIGN.md）语义一致，不是新增限制。后续如果需要交互式终端场景
//! 再扩展。
//!
//! **`process/start` 的响应本身不够**：2026-09 用户实测复现、定位到根因——
//! codex-core 客户端侧（`vendor/codex/codex-rs/core/src/unified_exec/
//! process.rs::spawn_exec_server_output_task`）读取命令输出/退出状态完全靠
//! 服务端**主动推送**的 JSON-RPC 通知（`process/output`/`process/exited`/
//! `process/closed`，见 `vendor/codex/codex-rs/exec-server/src/client.rs::
//! handle_server_notification`），`process/read`（`EXEC_READ_METHOD`）只是
//! 客户端检测到事件序号跳号时的补读兜底，不是主路径。之前这里只回了
//! `process/start` 的请求响应，从没发过这三个通知，客户端那边的输出收集任务
//! 卡在 `events.recv().await` 永远等不到东西，表现就是"命令其实跑完了，
//! 模型却一直说超时、没有输出"。现在 `run_exec`/`EXEC_METHOD` 里命令跑完之后
//! 立刻按顺序推送这三个通知（一次性同步执行，天然就是"跑完 = 立刻退出"，
//! 不需要真正的流式分片）。

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use base64::Engine as _;
use codex_exec_server_protocol::{
    EnvironmentConfigLayerStack, EnvironmentConfigReadParams, EnvironmentConfigReadResponse,
    EnvironmentInfo, ExecClosedNotification, ExecExitedNotification, ExecOutputDeltaNotification,
    ExecOutputStream, ExecParams, ExecResponse, FsCreateDirectoryParams, FsCreateDirectoryResponse,
    FsGetMetadataParams, FsGetMetadataResponse, FsReadFileParams, FsReadFileResponse,
    FsWriteFileParams, FsWriteFileResponse, InitializeResponse, JSONRPCError, JSONRPCErrorError,
    JSONRPCMessage, JSONRPCNotification, JSONRPCRequest, JSONRPCResponse, ProcessId,
    ProcessOutputChunk, ProcessSandboxType, ReadParams, ReadResponse, SignalResponse,
    TerminateParams, TerminateResponse, WriteResponse, WriteStatus, ENVIRONMENT_CONFIG_READ_METHOD,
    EXEC_CLOSED_METHOD, EXEC_EXITED_METHOD, EXEC_METHOD, EXEC_OUTPUT_DELTA_METHOD,
    EXEC_READ_METHOD, EXEC_SIGNAL_METHOD, EXEC_TERMINATE_METHOD, EXEC_WRITE_METHOD,
    FS_CREATE_DIRECTORY_METHOD, FS_GET_METADATA_METHOD, FS_READ_FILE_METHOD, FS_WRITE_FILE_METHOD,
    INITIALIZE_METHOD,
};
use futures_util::stream::SplitSink;
use futures_util::{SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

use crate::exec_target::ExecTarget;

type WsSink = SplitSink<WebSocketStream<TcpStream>, Message>;

#[derive(Debug, thiserror::Error)]
pub enum RocDeskExecServerError {
    #[error("exec-server io error: {0}")]
    Io(String),
}

/// 一个已经跑完的命令的结果——当前实现是"同步等完再回包"，所以进程一注册
/// 进 `processes` 表时就已经是终态了，`process/read` 只是把这份结果读出来。
struct ProcessState {
    chunk: Option<ProcessOutputChunk>,
    exit_code: Option<i32>,
    delivered: bool,
}

pub struct RocDeskExecServer {
    local_addr: SocketAddr,
}

impl RocDeskExecServer {
    /// 在 `127.0.0.1` 的一个随机端口上监听，返回后台任务已经在跑，调用方
    /// 只需要把 `websocket_url()` 交给
    /// `EnvironmentManager::upsert_environment_with_options` 注册即可。
    pub async fn bind(target: Arc<dyn ExecTarget>) -> Result<Self, RocDeskExecServerError> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| RocDeskExecServerError::Io(e.to_string()))?;
        let local_addr = listener
            .local_addr()
            .map_err(|e| RocDeskExecServerError::Io(e.to_string()))?;
        tokio::spawn(accept_loop(listener, target));
        Ok(Self { local_addr })
    }

    pub fn websocket_url(&self) -> String {
        format!("ws://{}", self.local_addr)
    }
}

async fn accept_loop(listener: TcpListener, target: Arc<dyn ExecTarget>) {
    loop {
        let (stream, _peer) = match listener.accept().await {
            Ok(pair) => pair,
            Err(e) => {
                tracing::warn!("RocDeskExecServer 停止监听（accept 失败）: {e}");
                return;
            }
        };
        let target = target.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_connection(stream, target).await {
                tracing::warn!("RocDeskExecServer 连接结束: {e}");
            }
        });
    }
}

async fn handle_connection(
    stream: TcpStream,
    target: Arc<dyn ExecTarget>,
) -> Result<(), RocDeskExecServerError> {
    let ws = tokio_tungstenite::accept_async(stream)
        .await
        .map_err(|e| RocDeskExecServerError::Io(e.to_string()))?;
    let (write, mut read) = ws.split();
    let write = Arc::new(Mutex::new(write));
    let processes: Arc<Mutex<HashMap<ProcessId, ProcessState>>> =
        Arc::new(Mutex::new(HashMap::new()));

    while let Some(frame) = read.next().await {
        let frame = frame.map_err(|e| RocDeskExecServerError::Io(e.to_string()))?;
        let text = match frame {
            Message::Text(text) => text.to_string(),
            Message::Binary(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
            Message::Close(_) => break,
            _ => continue,
        };
        let message: JSONRPCMessage = match serde_json::from_str(&text) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!("RocDeskExecServer 收到无法解析的 JSON-RPC 消息: {e}");
                continue;
            }
        };
        if let JSONRPCMessage::Request(req) = message {
            let target = target.clone();
            let processes = processes.clone();
            let write = write.clone();
            tokio::spawn(async move {
                let reply = handle_request(req, &target, &processes, &write).await;
                send(&write, &reply).await;
            });
        }
        // Notification（比如 "initialized"）不需要应答，忽略。
    }
    Ok(())
}

async fn send(write: &Arc<Mutex<WsSink>>, message: &JSONRPCMessage) {
    let Ok(text) = serde_json::to_string(message) else {
        return;
    };
    let mut write = write.lock().await;
    let _ = write.send(Message::Text(text.into())).await;
}

/// 主动向 codex-core 推送一条 JSON-RPC 通知（没有 `id`，不需要对方应答）——
/// `process/output`/`process/exited`/`process/closed` 这三个都是这种通知，
/// 见文件头新增的说明。序列化失败就静默丢弃，跟 `send()` 对响应/错误的处理
/// 方式一致（`ok_response`/`error_response` 那条路已经有独立的错误处理）。
async fn notify<T: serde::Serialize>(write: &Arc<Mutex<WsSink>>, method: &str, params: &T) {
    let Ok(params) = serde_json::to_value(params) else {
        return;
    };
    send(
        write,
        &JSONRPCMessage::Notification(JSONRPCNotification {
            method: method.to_string(),
            params: Some(params),
        }),
    )
    .await;
}

fn error_response(
    id: &codex_exec_server_protocol::RequestId,
    code: i64,
    message: String,
) -> JSONRPCMessage {
    JSONRPCMessage::Error(JSONRPCError {
        id: id.clone(),
        error: JSONRPCErrorError {
            code,
            data: None,
            message,
        },
    })
}

fn ok_response(
    id: &codex_exec_server_protocol::RequestId,
    result: serde_json::Value,
) -> JSONRPCMessage {
    JSONRPCMessage::Response(JSONRPCResponse {
        id: id.clone(),
        result,
    })
}

async fn handle_request(
    req: JSONRPCRequest,
    target: &Arc<dyn ExecTarget>,
    processes: &Arc<Mutex<HashMap<ProcessId, ProcessState>>>,
    write: &Arc<Mutex<WsSink>>,
) -> JSONRPCMessage {
    let id = req.id;
    let params = req.params.unwrap_or(serde_json::Value::Null);

    // `error_response`/`ok_response` 都只借用 `&id`——`RequestId` 不是 `Copy`，
    // `ok_response(id, to_result!(...))` 这种同一个调用里 `id` 既是外层参数、
    // 又可能被内层宏的 `return` 分支提前用一次的写法，如果两边都按值传，会被
    // borrow checker 判定成"同一个值可能用两次"（E0382，且 Rust 的参数求值
    // 顺序会让"先按值移动、宏内再 clone"这种修法依然报错）。全部只借用就没有
    // 这个问题，`id` 到函数结束都没被移动过。
    macro_rules! parse_params {
        ($ty:ty) => {
            match serde_json::from_value::<$ty>(params) {
                Ok(p) => p,
                Err(e) => return error_response(&id, -32602, format!("invalid params: {e}")),
            }
        };
    }

    macro_rules! to_result {
        ($value:expr) => {
            match serde_json::to_value($value) {
                Ok(v) => v,
                Err(e) => {
                    return error_response(&id, -32603, format!("failed to encode result: {e}"))
                }
            }
        };
    }

    match req.method.as_str() {
        INITIALIZE_METHOD => {
            let session_id = uuid::Uuid::new_v4().to_string();
            let mut environment_info = EnvironmentInfo::local();
            // `EnvironmentInfo::local()` 探测的是"运行 roc_desk 的这台 Windows
            // 机器"的 shell/cwd——对 `CodingTarget::Local` 这本来就是对的，但对
            // Remote(SSH)/Agent 这种远程目标完全对不上：2026-09 用户实测复现，
            // SSH（Linux）远程目标下 codex-core 一直以为执行端是本机 PowerShell，
            // 把要跑的命令套成了 `powershell.exe -Command "<真正的命令>"` 发过来，
            // 而不是按 *nix shell 语法套壳——因为 `Shell::from_environment_shell_info`
            // 就是按这里申报的 `shell.name` 决定怎么套壳的（见
            // `vendor/codex/codex-rs/core/src/shell.rs`）。按 `ExecTarget::
            // platform_hint()` 如实申报，让 codex-core 对远程目标按正确的 shell
            // 语法套壳。
            let hint = target.platform_hint();
            environment_info.platform_os = Some(hint.platform_os.to_string());
            environment_info.shell = codex_exec_server_protocol::ShellInfo {
                name: hint.shell_name.to_string(),
                path: hint.shell_path.to_string(),
            };
            if !hint.is_local_host {
                // 本机的 cwd/user_home_dir/temp_dir 对远程目标毫无意义，报了反而
                // 误导——这几个字段都是 `Option`，不报比报错的更安全。
                environment_info.cwd = None;
                environment_info.user_home_dir = None;
                environment_info.temp_dir = None;
                environment_info.temporary_directories = None;
                // 远程 shell 快照当前完全没实现（`RocDeskExecServer` 的
                // `process/write`/`process/signal` 是 no-op，见文件头注释），
                // 这个能力本来就该是 false——`EnvironmentInfo::local()` 默认值
                // 已经按 `cfg!(unix)` 在 Windows 宿主上算出 false，这里显式写一遍
                // 只是不想让未来改 `local()` 默认值时悄悄影响到这里。
                environment_info.capabilities.shell_snapshot_v2 = false;
            }
            // 2026-09 临时诊断日志：用户质疑"明明告诉了 codex-core 是 Windows，
            // 模型却还先试 uname/ps"，需要拿证据确认这里申报的 platform_os/shell
            // 是不是真的如预期——确认后应该删掉，不是长期保留的诊断设施。
            tracing::info!(
                platform_os = ?environment_info.platform_os,
                shell_name = %environment_info.shell.name,
                shell_path = %environment_info.shell.path,
                "RocDeskExecServer INITIALIZE: 申报的环境信息"
            );
            let response = InitializeResponse {
                session_id,
                environment_info: Some(environment_info),
            };
            ok_response(&id, to_result!(response))
        }
        FS_READ_FILE_METHOD => {
            let p: FsReadFileParams = parse_params!(FsReadFileParams);
            let path = p.path.to_path_buf();
            match target.read_file(&path).await {
                Ok(data) => {
                    let response = FsReadFileResponse {
                        data_base64: base64::engine::general_purpose::STANDARD.encode(data),
                    };
                    ok_response(&id, to_result!(response))
                }
                Err(e) => error_response(&id, -32000, e.to_string()),
            }
        }
        FS_WRITE_FILE_METHOD => {
            let p: FsWriteFileParams = parse_params!(FsWriteFileParams);
            let path = p.path.to_path_buf();
            let data = match base64::engine::general_purpose::STANDARD.decode(p.data_base64) {
                Ok(d) => d,
                Err(e) => return error_response(&id, -32602, format!("invalid base64: {e}")),
            };
            match target.stage_write(&path, data).await {
                Ok(()) => ok_response(&id, to_result!(FsWriteFileResponse {})),
                Err(e) => error_response(&id, -32000, e.to_string()),
            }
        }
        FS_CREATE_DIRECTORY_METHOD => {
            let p: FsCreateDirectoryParams = parse_params!(FsCreateDirectoryParams);
            let path = p.path.to_path_buf();
            match target
                .create_directory(&path, p.recursive.unwrap_or(false))
                .await
            {
                Ok(()) => ok_response(&id, to_result!(FsCreateDirectoryResponse {})),
                Err(e) => error_response(&id, -32000, e.to_string()),
            }
        }
        FS_GET_METADATA_METHOD => {
            let p: FsGetMetadataParams = parse_params!(FsGetMetadataParams);
            let path = p.path.to_path_buf();
            match target.get_metadata(&path).await {
                Ok(meta) => {
                    let response = FsGetMetadataResponse {
                        is_directory: meta.is_directory,
                        is_file: meta.is_file,
                        is_symlink: meta.is_symlink,
                        size: meta.size,
                        created_at_ms: meta.created_at_ms,
                        modified_at_ms: meta.modified_at_ms,
                    };
                    ok_response(&id, to_result!(response))
                }
                Err(e) => error_response(&id, -32000, e.to_string()),
            }
        }
        // roc_desk 没有 codex 那套"项目级 TOML 配置分层"（`.codex/config.toml`
        // 之类跟着代码走的执行端配置），MCP 服务器/权限规则都是走 roc_desk 自己
        // 的数据库+UI 配的，不是跟工作区一起分发的文件。这里如实回一个空分层——
        // `EnvironmentInfo::local()` 的 `capabilities.environment_config_read`
        // 本来就报了 `true`（见 `INITIALIZE_METHOD`），之前这个方法没实现，
        // codex-core 每次都会因为方法不存在而拿不到"没有可用的执行端配置"这个
        // 明确答案，退化成"探测执行端本地 MCP 服务器失败"的 WARN——回一个空
        // 分层能让 codex-core 干净地得出"没有"，而不是报错后放弃。
        ENVIRONMENT_CONFIG_READ_METHOD => {
            let p: EnvironmentConfigReadParams = parse_params!(EnvironmentConfigReadParams);
            let response = EnvironmentConfigReadResponse {
                user_home_dir: None,
                codex_home_dir: p.cwd,
                hostname: None,
                config: EnvironmentConfigLayerStack {
                    layers: vec![],
                    cloud_insertion_index: 0,
                },
                requirements: EnvironmentConfigLayerStack {
                    layers: vec![],
                    cloud_insertion_index: 0,
                },
            };
            ok_response(&id, to_result!(response))
        }
        EXEC_METHOD => {
            let p: ExecParams = parse_params!(ExecParams);
            let process_id = p.process_id.clone();
            let cwd = p.cwd.to_path_buf();
            // 见文件头注释：这里同步等命令跑完才回包，`process_id` 一进
            // `processes` 表时就已经是终态了。
            let outcome = target.run_command(p.argv, &cwd, p.env).await;
            let (chunk, exit_code) = match outcome {
                Ok(outcome) => {
                    let chunk = if outcome.output.is_empty() {
                        None
                    } else {
                        Some(ProcessOutputChunk {
                            seq: 0,
                            stream: ExecOutputStream::Stdout,
                            chunk: outcome.output.into(),
                        })
                    };
                    (chunk, outcome.exit_code)
                }
                Err(e) => {
                    let message = format!("{e}\n");
                    (
                        Some(ProcessOutputChunk {
                            seq: 0,
                            stream: ExecOutputStream::Stderr,
                            chunk: message.into_bytes().into(),
                        }),
                        Some(-1),
                    )
                }
            };
            processes.lock().await.insert(
                process_id.clone(),
                ProcessState {
                    chunk: chunk.clone(),
                    exit_code,
                    delivered: true,
                },
            );
            // 见文件头新增的那段说明：codex-core 客户端读输出/退出状态靠这三个
            // 主动推送的通知，不靠对 `process/start` 请求本身的响应、也不会
            // 主动去调 `process/read`（那只是检测到序号跳号时的补读兜底）。
            // 这里的执行天然是"同步跑完 = 立刻退出"，所以三个通知背靠背按序号
            // 1/2/3 连续发出去，不需要真正的分片流式推送。`processes` 表里这条
            // 记录的 `delivered` 直接标 `true`——通知已经把内容送过去了，
            // `process/read` 只是兜底，不需要再把同样的内容重发一遍。
            if let Some(chunk) = &chunk {
                notify(
                    write,
                    EXEC_OUTPUT_DELTA_METHOD,
                    &ExecOutputDeltaNotification {
                        process_id: process_id.clone(),
                        seq: 1,
                        stream: chunk.stream,
                        chunk: chunk.chunk.clone(),
                    },
                )
                .await;
            }
            notify(
                write,
                EXEC_EXITED_METHOD,
                &ExecExitedNotification {
                    process_id: process_id.clone(),
                    seq: 2,
                    exit_code: exit_code.unwrap_or(-1),
                    sandbox_denied: Some(false),
                },
            )
            .await;
            notify(
                write,
                EXEC_CLOSED_METHOD,
                &ExecClosedNotification {
                    process_id: process_id.clone(),
                    seq: 3,
                },
            )
            .await;
            let response = ExecResponse {
                process_id,
                sandbox_type: Some(ProcessSandboxType::None),
            };
            ok_response(&id, to_result!(response))
        }
        EXEC_READ_METHOD => {
            let p: ReadParams = parse_params!(ReadParams);
            let mut guard = processes.lock().await;
            let Some(state) = guard.get_mut(&p.process_id) else {
                return ok_response(
                    &id,
                    to_result!(ReadResponse {
                        chunks: vec![],
                        next_seq: 0,
                        exited: true,
                        exit_code: None,
                        closed: true,
                        failure: Some("unknown process_id".to_string()),
                        sandbox_denied: false,
                    }),
                );
            };
            let chunks = if state.delivered {
                vec![]
            } else {
                state.delivered = true;
                state.chunk.clone().into_iter().collect()
            };
            let response = ReadResponse {
                chunks,
                next_seq: 1,
                exited: true,
                exit_code: state.exit_code,
                closed: true,
                failure: None,
                sandbox_denied: false,
            };
            ok_response(&id, to_result!(response))
        }
        EXEC_TERMINATE_METHOD => {
            let _p: TerminateParams = parse_params!(TerminateParams);
            // 当前实现里进程注册时已经是终态（同步等完才回 `process/start`），
            // 这里永远回"已经不在跑了"。
            ok_response(&id, to_result!(TerminateResponse { running: false }))
        }
        EXEC_WRITE_METHOD => ok_response(
            &id,
            to_result!(WriteResponse {
                status: WriteStatus::UnknownProcess
            }),
        ),
        EXEC_SIGNAL_METHOD => ok_response(&id, to_result!(SignalResponse {})),
        other => error_response(&id, -32601, format!("method not implemented: {other}")),
    }
}
