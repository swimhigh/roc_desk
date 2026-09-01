//! 单条到 Agent 的 TLS 连接，内部按 AGENT_DESIGN.md §3.2 的帧协议多路复用多个
//! 逻辑请求——结构上和 `ssh::session::SshSession` 对称（一条物理连接、`Channel`
//! 换成"流 ID"），读循环单线程解帧后按流 ID 分发，写操作全部经一个共享的
//! `mpsc` 队列交给唯一的写任务顺序写出，避免多个并发请求互相打断彼此的帧。

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use roc_desk_protocol::{
    decode_json, encode_json, read_frame, write_frame, FrameType, Request, Response, ResponseBody, DATA_CHUNK_SIZE, PROTOCOL_VERSION,
};
use rustls::pki_types::ServerName;
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio_rustls::TlsConnector;
use uuid::Uuid;

use super::handshake::{AgentCertVerifier, TofuCertCapture};
use crate::connection::ConnectionProfile;
use crate::error::AppError;

type OutMsg = (u32, FrameType, Vec<u8>);
type OutSender = mpsc::UnboundedSender<OutMsg>;

/// `request_streamed`（`ReadFile`/`ReadFileBounded`）、`open_shell`（交互式终端）
/// 逐帧收到的内容。
#[derive(Debug)]
pub enum StreamFrame {
    Control(Response),
    Data(Vec<u8>),
    End,
}

/// `AgentSession::test_connect` 的返回值——连接设置对话框"测试连接"按钮用它拼一句
/// 人类可读的成功提示。
pub struct TestConnectResult {
    pub hostname: String,
    pub server_version: String,
    pub fingerprint: String,
}

enum PendingKind {
    Oneshot(oneshot::Sender<Response>),
    /// `keep_open_on_control` 为 false 时（`ReadFile` 类一次性流）——收到一个不是
    /// `FileMeta` 的 Control 响应就当终态，摘掉注册；为 true 时（`OpenShell` 交互式
    /// 终端）——Control 帧（`ShellResize` 的回声/心跳之类，目前其实不会真收到，
    /// 纯粹是留了这个开关）不代表流结束，只有显式的 `StreamEnd` 才摘注册。
    Stream { tx: mpsc::UnboundedSender<StreamFrame>, keep_open_on_control: bool },
}

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

pub struct AgentSession {
    out_tx: OutSender,
    pending: Arc<Mutex<HashMap<u32, PendingKind>>>,
    next_stream_id: AtomicU32,
    /// 交互式终端：本地稳定 id（前端用它调用 write/resize/close）到内部流 ID 的映射，
    /// 和 `ssh::session::SshSession::channels` 是同一种角色。
    shell_channels: Mutex<HashMap<Uuid, u32>>,
}

impl AgentSession {
    pub async fn connect(profile: &ConnectionProfile, token: String, cert_verifier: &AgentCertVerifier) -> Result<Self, AppError> {
        let host = profile.host.trim().to_string();
        let (tls_stream, fingerprint) = Self::tls_connect(&host, profile.port).await?;

        let trusted = cert_verifier.verify(profile.id, &host, profile.port, &fingerprint).await?;
        if !trusted {
            return Err(AppError::HostKeyRejected("用户拒绝信任 Agent 证书指纹".into()));
        }

        let session = Self::from_stream(tls_stream);
        session.handshake(token).await?;
        Ok(session)
    }

    /// 连接设置对话框的"测试连接"用：只验证"TCP 能连上、TLS 握手能完成、配对令牌
    /// 校验能通过"，不落库、不做证书指纹 TOFU 持久化、也不经过 `ConnectionProfile`
    /// （表单里填的可能还没保存）。测试完就地断开——不返回 `Self`，调用方拿到结果
    /// 就够了，没有理由为了一次性验证保留这条连接。
    pub async fn test_connect(host: &str, port: u16, token: String) -> Result<TestConnectResult, AppError> {
        let host = host.trim().to_string();
        let (tls_stream, fingerprint) = Self::tls_connect(&host, port).await?;
        let session = Self::from_stream(tls_stream);
        let (server_version, hostname) = session.handshake(token).await?;
        Ok(TestConnectResult { hostname, server_version, fingerprint })
    }

    async fn tls_connect(host: &str, port: u16) -> Result<(tokio_rustls::client::TlsStream<TcpStream>, String), AppError> {
        let tcp = TcpStream::connect((host, port)).await.map_err(|e| AppError::Connection(e.to_string()))?;

        let capture = TofuCertCapture::new();
        let tls_config = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(capture.clone())
            .with_no_client_auth();
        let connector = TlsConnector::from(Arc::new(tls_config));
        let server_name = ServerName::try_from(host.to_string()).map_err(|e| AppError::Connection(format!("无效主机名: {e}")))?.to_owned();
        let tls_stream = connector.connect(server_name, tcp).await.map_err(|e| AppError::Connection(format!("TLS 握手失败: {e}")))?;

        let fingerprint = capture.take_fingerprint_sha256().ok_or_else(|| AppError::Connection("未能获取 Agent 证书".into()))?;
        Ok((tls_stream, fingerprint))
    }

    fn from_stream<S>(stream: S) -> Self
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let (mut reader, mut writer) = tokio::io::split(stream);
        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<OutMsg>();
        let pending: Arc<Mutex<HashMap<u32, PendingKind>>> = Arc::new(Mutex::new(HashMap::new()));

        tokio::spawn(async move {
            while let Some((stream_id, frame_type, payload)) = out_rx.recv().await {
                if write_frame(&mut writer, stream_id, frame_type, &payload).await.is_err() {
                    break;
                }
            }
            let _ = writer.shutdown().await;
        });

        let pending_reader = pending.clone();
        tokio::spawn(async move {
            loop {
                let frame = match read_frame(&mut reader).await {
                    Ok(f) => f,
                    Err(_) => break,
                };
                match frame.frame_type {
                    FrameType::Control => {
                        let Ok(response) = decode_json::<Response>(&frame.payload) else { continue };
                        let mut map = pending_reader.lock().await;
                        // `ReadFile`/`ReadFileBounded` 的首帧是 `FileMeta`，后面还有
                        // `DataChunk`/`StreamEnd`；其它一切响应（含流式请求的 Error）
                        // 都是终态，不会再有后续帧，收到就要把注册表项摘掉——除非这个
                        // 流显式声明"Control 帧不代表结束"（`OpenShell` 交互式终端）。
                        let looks_terminal = !matches!(response, Response::Ok(ResponseBody::FileMeta { .. }));
                        match map.get(&frame.stream_id) {
                            Some(PendingKind::Oneshot(_)) => {
                                if let Some(PendingKind::Oneshot(tx)) = map.remove(&frame.stream_id) {
                                    let _ = tx.send(response);
                                }
                            }
                            Some(PendingKind::Stream { tx, keep_open_on_control }) => {
                                let keep_open = *keep_open_on_control;
                                let _ = tx.send(StreamFrame::Control(response));
                                if looks_terminal && !keep_open {
                                    map.remove(&frame.stream_id);
                                }
                            }
                            None => {}
                        }
                    }
                    FrameType::DataChunk => {
                        let map = pending_reader.lock().await;
                        if let Some(PendingKind::Stream { tx, .. }) = map.get(&frame.stream_id) {
                            let _ = tx.send(StreamFrame::Data(frame.payload));
                        }
                    }
                    FrameType::StreamEnd => {
                        let mut map = pending_reader.lock().await;
                        if let Some(PendingKind::Stream { tx, .. }) = map.remove(&frame.stream_id) {
                            let _ = tx.send(StreamFrame::End);
                        }
                    }
                    FrameType::Error => {}
                }
            }
            pending_reader.lock().await.clear();
        });

        Self { out_tx, pending, next_stream_id: AtomicU32::new(1), shell_channels: Mutex::new(HashMap::new()) }
    }

    /// 返回 `(server_version, hostname)`——`connect()` 目前不需要这两个值，但
    /// `test_connect()` 需要拿它们拼一句"连接成功：主机 XXX，Agent 版本 XXX"给用户看。
    async fn handshake(&self, token: String) -> Result<(String, String), AppError> {
        let request = Request::Handshake { token, protocol_version: PROTOCOL_VERSION, client_version: env!("CARGO_PKG_VERSION").to_string() };
        match tokio::time::timeout(REQUEST_TIMEOUT, self.request(request)).await {
            Ok(Ok(Response::Ok(ResponseBody::Handshake { server_version, hostname }))) => Ok((server_version, hostname)),
            Ok(Ok(Response::Error { message, .. })) => Err(AppError::Auth(message)),
            Ok(Ok(_)) => Err(AppError::Internal("Agent 握手返回了意外的响应类型".into())),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(AppError::Connection("等待 Agent 握手响应超时".into())),
        }
    }

    fn alloc_stream_id(&self) -> u32 {
        self.next_stream_id.fetch_add(1, Ordering::Relaxed)
    }

    async fn send_control(&self, stream_id: u32, request: &Request) -> Result<(), AppError> {
        let payload = encode_json(request).map_err(|e| AppError::Internal(e.to_string()))?;
        self.out_tx
            .send((stream_id, FrameType::Control, payload))
            .map_err(|_| AppError::Connection("Agent 连接已断开".into()))
    }

    /// 一问一答的 RPC（`ListDir`/`Stat`/`Delete`/`Rename`/`CreateDir`/`ListRoots`/
    /// `Exec`/`SearchContent`/`SearchFileName`/`Handshake`）。
    pub async fn request(&self, request: Request) -> Result<Response, AppError> {
        let stream_id = self.alloc_stream_id();
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(stream_id, PendingKind::Oneshot(tx));
        if let Err(e) = self.send_control(stream_id, &request).await {
            self.pending.lock().await.remove(&stream_id);
            return Err(e);
        }
        match tokio::time::timeout(REQUEST_TIMEOUT, rx).await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(_)) => Err(AppError::Connection("Agent 连接已断开".into())),
            Err(_) => {
                self.pending.lock().await.remove(&stream_id);
                Err(AppError::Connection("等待 Agent 响应超时".into()))
            }
        }
    }

    /// `ReadFile`/`ReadFileBounded`：先收到一个 Control 帧（`FileMeta` 或 `Error`），
    /// 再收到若干 `DataChunk`，最后 `StreamEnd`。
    pub async fn request_streamed(&self, request: Request) -> Result<mpsc::UnboundedReceiver<StreamFrame>, AppError> {
        let stream_id = self.alloc_stream_id();
        let (tx, rx) = mpsc::unbounded_channel();
        self.pending.lock().await.insert(stream_id, PendingKind::Stream { tx, keep_open_on_control: false });
        if let Err(e) = self.send_control(stream_id, &request).await {
            self.pending.lock().await.remove(&stream_id);
            return Err(e);
        }
        Ok(rx)
    }

    /// `WriteFile`：发完 Control 请求后紧接着把 `bytes` 切成 `DataChunk` 帧发出，
    /// 最后发 `StreamEnd`，然后等 Agent 落盘完成后的单个 Control 响应。
    pub async fn write_stream(&self, request: Request, bytes: &[u8]) -> Result<Response, AppError> {
        let stream_id = self.alloc_stream_id();
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(stream_id, PendingKind::Oneshot(tx));
        if let Err(e) = self.send_control(stream_id, &request).await {
            self.pending.lock().await.remove(&stream_id);
            return Err(e);
        }
        for chunk in bytes.chunks(DATA_CHUNK_SIZE) {
            if self.out_tx.send((stream_id, FrameType::DataChunk, chunk.to_vec())).is_err() {
                self.pending.lock().await.remove(&stream_id);
                return Err(AppError::Connection("Agent 连接已断开".into()));
            }
        }
        if self.out_tx.send((stream_id, FrameType::StreamEnd, Vec::new())).is_err() {
            self.pending.lock().await.remove(&stream_id);
            return Err(AppError::Connection("Agent 连接已断开".into()));
        }
        match tokio::time::timeout(REQUEST_TIMEOUT, rx).await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(_)) => Err(AppError::Connection("Agent 连接已断开".into())),
            Err(_) => {
                self.pending.lock().await.remove(&stream_id);
                Err(AppError::Connection("等待 Agent 写入响应超时".into()))
            }
        }
    }

    /// `run_command`/AI 编程助手用：把整行命令交给 `cmd.exe /C` 执行（AGENT_DESIGN.md
    /// §四.4——Windows 语义原生按参数数组传递，不走"拼字符串再转义"那条路，这里的
    /// "拼一个字符串"只发生在 `cmd.exe /C` 这一层，和 SSH 那边把整行丢给远端 shell
    /// 解析是同一件事，只是 Windows 下没有引号转义规则失配的问题）。
    pub async fn exec(&self, command: &str, cwd: &str) -> Result<String, AppError> {
        self.exec_argv("cmd.exe", &["/C".to_string(), command.to_string()], cwd).await
    }

    /// 直接传参数数组给 `CreateProcess`，不经过 `cmd.exe` 解析——没有任何"拼字符串
    /// 再转义"的环节，`git_ops.rs` 的 Git 集成用这个而不是 `exec()`，从根上避免
    /// POSIX `shell_quote` 规则套在 Windows 目标上失配的问题（AGENT_DESIGN.md §一）。
    pub async fn exec_argv(&self, command: &str, args: &[String], cwd: &str) -> Result<String, AppError> {
        let request = Request::Exec { command: command.to_string(), args: args.to_vec(), cwd: cwd.to_string(), timeout_secs: 0 };
        match self.request(request).await? {
            Response::Ok(ResponseBody::ExecResult { output, .. }) => Ok(output),
            Response::Error { message, .. } => Err(AppError::Internal(message)),
            _ => Err(AppError::Internal("Agent 返回了意外的响应类型".into())),
        }
    }

    /// 打开一个交互式终端（AGENT_DESIGN.md §四.4 Phase 2）：返回本地稳定 id，前端
    /// 后续通过它调用 `write_shell`/`resize_shell`/`close_shell`；远端 PTY 输出经
    /// `agent:data` 事件推送——和 `ssh::session::SshSession::open_shell` 是同一套接口
    /// 形状，方便前端复用同一个 `TerminalView` 组件。
    pub async fn open_shell(&self, rows: u16, cols: u16, cwd: &str, app_handle: AppHandle) -> Result<Uuid, AppError> {
        let stream_id = self.alloc_stream_id();
        let (tx, mut rx) = mpsc::unbounded_channel::<StreamFrame>();
        self.pending.lock().await.insert(stream_id, PendingKind::Stream { tx, keep_open_on_control: true });
        let request = Request::OpenShell { cols, rows, cwd: cwd.to_string() };
        if let Err(e) = self.send_control(stream_id, &request).await {
            self.pending.lock().await.remove(&stream_id);
            return Err(e);
        }

        // 打开成功前先等 Agent 的 Ok/Error 确认，避免"看起来打开成功了"但其实
        // Agent 那边 ConPTY/进程起不来的情况下前端还是渲染出一个终端 Tab。
        match tokio::time::timeout(REQUEST_TIMEOUT, rx.recv()).await {
            Ok(Some(StreamFrame::Control(Response::Ok(ResponseBody::Empty)))) => {}
            Ok(Some(StreamFrame::Control(Response::Error { message, .. }))) => {
                self.pending.lock().await.remove(&stream_id);
                return Err(AppError::Internal(message));
            }
            Ok(Some(_)) | Ok(None) => {
                self.pending.lock().await.remove(&stream_id);
                return Err(AppError::Internal("Agent 返回了意外的响应类型".into()));
            }
            Err(_) => {
                self.pending.lock().await.remove(&stream_id);
                return Err(AppError::Connection("等待 Agent 打开终端超时".into()));
            }
        }

        let local_id = Uuid::new_v4();
        self.shell_channels.lock().await.insert(local_id, stream_id);

        tokio::spawn(async move {
            // 和 `ssh::session::SshSession::open_shell` 同一个理由（那边注释更详细）：
            // 前端拿到 `local_id` 后还要走一次 IPC 往返 + React 挂载才会挂上
            // `listen("agent:data", ...)`，这段窗口期里如果已经开始 `rx.recv()` 并
            // `emit`，PTY 起来后立刻打印的东西（比如自定义 PS1/欢迎信息）就会在没人
            // 监听时被发出去，丢失且不会重发。这里的 `rx` 是无界 mpsc，不消费也不会
            // 丢数据，稳妥地延后一小段再开始读，就能让第一批输出等到前端监听器
            // 挂上之后再流过去。
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            while let Some(item) = rx.recv().await {
                match item {
                    StreamFrame::Data(bytes) => {
                        let _ = app_handle.emit("agent:data", serde_json::json!({ "channelId": local_id, "data": bytes }));
                    }
                    StreamFrame::End => {
                        let _ = app_handle.emit("agent:status", serde_json::json!({ "channelId": local_id, "status": "disconnected" }));
                        break;
                    }
                    StreamFrame::Control(_) => {}
                }
            }
        });

        Ok(local_id)
    }

    pub async fn write_shell(&self, local_id: Uuid, data: Vec<u8>) -> Result<(), AppError> {
        let stream_id = *self
            .shell_channels
            .lock()
            .await
            .get(&local_id)
            .ok_or_else(|| AppError::NotFound(format!("agent shell channel not found: {local_id}")))?;
        self.out_tx
            .send((stream_id, FrameType::DataChunk, data))
            .map_err(|_| AppError::Connection("Agent 连接已断开".into()))
    }

    pub async fn resize_shell(&self, local_id: Uuid, cols: u16, rows: u16) -> Result<(), AppError> {
        let stream_id = *self
            .shell_channels
            .lock()
            .await
            .get(&local_id)
            .ok_or_else(|| AppError::NotFound(format!("agent shell channel not found: {local_id}")))?;
        let payload = encode_json(&Request::ShellResize { cols, rows }).map_err(|e| AppError::Internal(e.to_string()))?;
        self.out_tx
            .send((stream_id, FrameType::Control, payload))
            .map_err(|_| AppError::Connection("Agent 连接已断开".into()))
    }

    /// 关闭一个终端 Tab：给 Agent 发 `StreamEnd`（对应它那边杀掉 PTY 子进程），
    /// 并摘掉本地的流注册表项——找不到就当已经关过了，不算错误（和
    /// `SshSession::close_channel` 的宽松语义一致）。
    pub async fn close_shell(&self, local_id: Uuid) -> Result<(), AppError> {
        let stream_id = self.shell_channels.lock().await.remove(&local_id);
        if let Some(stream_id) = stream_id {
            self.pending.lock().await.remove(&stream_id);
            let _ = self.out_tx.send((stream_id, FrameType::StreamEnd, Vec::new()));
        }
        Ok(())
    }

    /// `AgentConnectionPool::get_or_connect` 在把缓存的会话交给调用方之前先问一句
    /// ——和 `SshSession::is_alive` 同一个理由：网络掉线/Agent 重启后，写任务
    /// （`from_stream` 里那个 `tokio::spawn`）写失败会 `break` 退出并丢掉
    /// `out_rx`，`out_tx.is_closed()` 就会变 true，据此判断这条 TLS 连接已经废了，
    /// 不能继续复用。
    pub fn is_alive(&self) -> bool {
        !self.out_tx.is_closed()
    }
}
