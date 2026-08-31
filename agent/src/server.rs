//! TCP 监听 + TLS + 每连接的帧分发循环（AGENT_DESIGN.md §2.1/§3.2）。
//!
//! 一条物理连接上复用多个逻辑"流"：单个读循环负责从 TLS 流里解帧，大多数请求
//! （`ListDir`/`Exec`/`ReadFile`...）各自 `tokio::spawn` 一个任务去处理、通过一个
//! 共享的 `out_tx` 通道把响应帧交给唯一的写任务顺序写回；`WriteFile`/`OpenShell`
//! 比较特殊——它们之后还会在同一个 stream_id 上持续收到 `DataChunk`/`Control`/
//! `StreamEnd` 帧，读循环用 `stream_waiters` 表把这些后续帧转发给专门等待它们的
//! 任务，而不是当成新请求解析。

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use rustls::pki_types::PrivateKeyDer;
use roc_desk_protocol::{
    decode_json, encode_json, read_frame, write_frame, ErrorCode, Frame, FrameType, Request, Response, ResponseBody, DATA_CHUNK_SIZE,
    PROTOCOL_VERSION,
};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_rustls::TlsAcceptor;

use crate::audit::AuditLog;
use crate::cert::AgentCert;
use crate::config::AgentConfig;
use crate::{auth, handlers, pathguard};

pub struct ServerContext {
    pub allowed_roots: Vec<String>,
    pub exec_timeout_secs: u32,
    pub token_hash: Option<String>,
    pub audit: Arc<AuditLog>,
    pub hostname: String,
}

type OutMsg = (u32, FrameType, Vec<u8>);
type OutSender = mpsc::UnboundedSender<OutMsg>;

pub async fn run(config: AgentConfig, cert: AgentCert, audit: Arc<AuditLog>) -> std::io::Result<()> {
    let addr = format!("{}:{}", config.server.listen_addr, config.server.port);
    let listener = TcpListener::bind(&addr).await?;
    tracing::info!(%addr, fingerprint = %cert.fingerprint_sha256, "roc_desk_agent listening");

    let tls_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert.cert_der], PrivateKeyDer::Pkcs8(cert.key_der))
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
    let acceptor = TlsAcceptor::from(Arc::new(tls_config));

    let ctx = Arc::new(ServerContext {
        allowed_roots: config.allowed_roots_normalized(),
        exec_timeout_secs: config.limits.exec_timeout_secs,
        token_hash: config.security.token_hash.clone(),
        audit,
        hostname: std::env::var("COMPUTERNAME").unwrap_or_else(|_| "roc_desk_agent".to_string()),
    });
    let semaphore = Arc::new(tokio::sync::Semaphore::new(config.limits.max_concurrent_connections.max(1)));

    loop {
        // 单次 accept 失败（例如瞬时的文件描述符耗尽）不该拖垮整个监听循环——
        // 之前用 `?` 直接把错误往上传，会导致这一次意外直接终止整个 Agent 进程，
        // 所有已连接的客户端跟着断线。记日志后继续下一轮 accept，而不是让服务
        // 整体死掉。
        let (stream, peer_addr) = match listener.accept().await {
            Ok(pair) => pair,
            Err(e) => {
                tracing::warn!(error = %e, "accept 失败，继续监听");
                continue;
            }
        };
        let acceptor = acceptor.clone();
        let ctx = ctx.clone();
        let semaphore = semaphore.clone();
        tokio::spawn(async move {
            let Ok(_permit) = semaphore.try_acquire_owned() else {
                tracing::warn!(%peer_addr, "拒绝连接：已达到 max_concurrent_connections 上限");
                return;
            };
            match acceptor.accept(stream).await {
                Ok(tls_stream) => {
                    if let Err(e) = handle_connection(tls_stream, peer_addr, ctx).await {
                        tracing::debug!(%peer_addr, error = %e, "连接结束");
                    }
                }
                Err(e) => tracing::warn!(%peer_addr, error = %e, "TLS 握手失败"),
            }
        });
    }
}

async fn handle_connection<S>(stream: S, peer_addr: SocketAddr, ctx: Arc<ServerContext>) -> Result<(), String>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (mut reader, mut writer) = tokio::io::split(stream);
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<OutMsg>();

    let writer_task = tokio::spawn(async move {
        while let Some((stream_id, frame_type, payload)) = out_rx.recv().await {
            if write_frame(&mut writer, stream_id, frame_type, &payload).await.is_err() {
                break;
            }
        }
        let _ = writer.shutdown().await;
    });

    if let Err(e) = do_handshake(&mut reader, &out_tx, peer_addr, &ctx).await {
        drop(out_tx);
        let _ = writer_task.await;
        return Err(e);
    }

    // 已经打开的双向流（目前有两种："WriteFile" 只关心后续的 DataChunk/StreamEnd；
    // "OpenShell" 打开的交互式终端还要关心后续的 Control 帧——那是 `ShellResize`，
    // 不是新请求）。收到任何帧类型都先看 stream_id 是不是已经注册在这张表里，
    // 是的话原样转发给对应任务，不当成新请求解析。
    let mut stream_waiters: HashMap<u32, mpsc::Sender<Frame>> = HashMap::new();

    loop {
        let frame = match read_frame(&mut reader).await {
            Ok(f) => f,
            Err(_) => break,
        };

        if let Some(sender) = stream_waiters.get(&frame.stream_id) {
            let is_end = frame.frame_type == FrameType::StreamEnd;
            let stream_id = frame.stream_id;
            let _ = sender.send(frame).await;
            if is_end {
                stream_waiters.remove(&stream_id);
            }
            continue;
        }

        match frame.frame_type {
            FrameType::Control => {
                let Ok(request) = decode_json::<Request>(&frame.payload) else {
                    send_error(&out_tx, frame.stream_id, ErrorCode::InvalidArgument, "无法解析请求".into());
                    continue;
                };
                match request {
                    Request::Handshake { .. } => {
                        send_error(&out_tx, frame.stream_id, ErrorCode::InvalidArgument, "已完成握手，不能重复握手".into());
                    }
                    Request::WriteFile { path, expected_mtime } => {
                        let (tx, rx) = mpsc::channel::<Frame>(8);
                        stream_waiters.insert(frame.stream_id, tx);
                        spawn_write_file(path, expected_mtime, frame.stream_id, rx, out_tx.clone(), ctx.clone());
                    }
                    Request::OpenShell { cols, rows, cwd } => {
                        let (tx, rx) = mpsc::channel::<Frame>(32);
                        stream_waiters.insert(frame.stream_id, tx);
                        spawn_shell(cols, rows, cwd, frame.stream_id, rx, out_tx.clone());
                    }
                    other => spawn_request(other, frame.stream_id, out_tx.clone(), ctx.clone()),
                }
            }
            // 到这里说明 stream_id 没有注册等待者——这两类帧只应该跟在一个已经打开的
            // 流后面出现，独立出现就是协议时序错误（客户端 bug 或恶意流量），直接丢弃。
            FrameType::DataChunk | FrameType::StreamEnd => {
                tracing::debug!(stream_id = frame.stream_id, "收到没有对应等待者的数据帧，丢弃");
            }
            FrameType::Error => {
                tracing::debug!(stream_id = frame.stream_id, "收到客户端 Error 帧");
            }
        }
    }

    drop(out_tx);
    let _ = writer_task.await;
    Ok(())
}

async fn do_handshake<R: AsyncRead + Unpin>(reader: &mut R, out_tx: &OutSender, peer_addr: SocketAddr, ctx: &ServerContext) -> Result<(), String> {
    let frame = tokio::time::timeout(Duration::from_secs(10), read_frame(reader))
        .await
        .map_err(|_| "等待握手超时".to_string())?
        .map_err(|e| format!("读取握手帧失败: {e}"))?;

    let request = decode_json::<Request>(&frame.payload).map_err(|e| format!("握手帧不是合法请求: {e}"))?;
    let Request::Handshake { token, protocol_version, client_version } = request else {
        send_error(out_tx, frame.stream_id, ErrorCode::AuthFailed, "第一个请求必须是 Handshake".into());
        return Err("握手顺序错误".to_string());
    };

    if protocol_version != PROTOCOL_VERSION {
        send_error(
            out_tx,
            frame.stream_id,
            ErrorCode::ProtocolVersionMismatch,
            format!("协议版本不兼容：Agent={PROTOCOL_VERSION}，客户端={protocol_version}，请升级其中一方"),
        );
        return Err("协议版本不兼容".to_string());
    }

    let Some(expected_hash) = &ctx.token_hash else {
        send_error(out_tx, frame.stream_id, ErrorCode::AuthFailed, "Agent 尚未配对，请先在服务器上执行 pair 子命令".into());
        return Err("未配对".to_string());
    };
    if !auth::verify_token(&token, expected_hash) {
        ctx.audit.record("handshake_fail", &peer_addr.to_string(), &format!("client_version={client_version}")).await;
        send_error(out_tx, frame.stream_id, ErrorCode::AuthFailed, "配对令牌不正确".into());
        return Err("令牌校验失败".to_string());
    }

    ctx.audit.record("handshake_ok", &peer_addr.to_string(), &format!("client_version={client_version}")).await;
    let body = ResponseBody::Handshake { server_version: env!("CARGO_PKG_VERSION").to_string(), hostname: ctx.hostname.clone() };
    let _ = out_tx.send((frame.stream_id, FrameType::Control, encode_json(&Response::Ok(body)).unwrap_or_default()));
    Ok(())
}

fn send_error(out_tx: &OutSender, stream_id: u32, code: ErrorCode, message: String) {
    let payload = encode_json(&Response::Error { code, message }).unwrap_or_default();
    let _ = out_tx.send((stream_id, FrameType::Control, payload));
}

fn send_ok(out_tx: &OutSender, stream_id: u32, body: ResponseBody) {
    let payload = encode_json(&Response::Ok(body)).unwrap_or_default();
    let _ = out_tx.send((stream_id, FrameType::Control, payload));
}

fn guarded_path(raw: &str, ctx: &ServerContext) -> Result<PathBuf, ErrorCode> {
    pathguard::check_allowed(raw, &ctx.allowed_roots)
}

/// 大多数"一问一答"请求的处理（`ReadFile`/`WriteFile` 除外——前者要流式回传多帧，
/// 后者需要在读循环里注册等待后续 `DataChunk`，两者都不适合塞进这个通用分支）。
fn spawn_request(request: Request, stream_id: u32, out_tx: OutSender, ctx: Arc<ServerContext>) {
    tokio::spawn(async move {
        match request {
            Request::Handshake { .. } | Request::WriteFile { .. } | Request::OpenShell { .. } | Request::ShellResize { .. } => {
                unreachable!("由调用方在 spawn 之前处理")
            }
            Request::ListDir { path } => match guarded_path(&path, &ctx) {
                Ok(p) => match handlers::fs::list_dir(&p).await {
                    Ok(entries) => send_ok(&out_tx, stream_id, ResponseBody::Entries(entries)),
                    Err((code, msg)) => send_error(&out_tx, stream_id, code, msg),
                },
                Err(code) => send_error(&out_tx, stream_id, code, format!("路径不在允许访问的范围内: {path}")),
            },
            Request::Stat { path } => match guarded_path(&path, &ctx) {
                Ok(p) => match handlers::fs::stat(&p).await {
                    Ok((size, mtime, _is_dir)) => send_ok(&out_tx, stream_id, ResponseBody::FileMeta { size, mtime }),
                    Err((code, msg)) => send_error(&out_tx, stream_id, code, msg),
                },
                Err(code) => send_error(&out_tx, stream_id, code, format!("路径不在允许访问的范围内: {path}")),
            },
            Request::ReadFile { path } => handle_read_file(path, None, stream_id, &out_tx, &ctx).await,
            Request::ReadFileBounded { path, max_bytes } => handle_read_file(path, Some(max_bytes), stream_id, &out_tx, &ctx).await,
            Request::Delete { path, is_dir } => match guarded_path(&path, &ctx) {
                Ok(p) => match handlers::fs::delete(&p, is_dir).await {
                    Ok(()) => send_ok(&out_tx, stream_id, ResponseBody::Empty),
                    Err((code, msg)) => send_error(&out_tx, stream_id, code, msg),
                },
                Err(code) => send_error(&out_tx, stream_id, code, format!("路径不在允许访问的范围内: {path}")),
            },
            Request::Rename { from, to } => {
                let checked = guarded_path(&from, &ctx).and_then(|f| guarded_path(&to, &ctx).map(|t| (f, t)));
                match checked {
                    Ok((f, t)) => match handlers::fs::rename(&f, &t).await {
                        Ok(()) => send_ok(&out_tx, stream_id, ResponseBody::Empty),
                        Err((code, msg)) => send_error(&out_tx, stream_id, code, msg),
                    },
                    Err(code) => send_error(&out_tx, stream_id, code, "路径不在允许访问的范围内".into()),
                }
            }
            Request::CreateDir { path } => match guarded_path(&path, &ctx) {
                Ok(p) => match handlers::fs::create_dir(&p).await {
                    Ok(()) => send_ok(&out_tx, stream_id, ResponseBody::Empty),
                    Err((code, msg)) => send_error(&out_tx, stream_id, code, msg),
                },
                Err(code) => send_error(&out_tx, stream_id, code, format!("路径不在允许访问的范围内: {path}")),
            },
            Request::ListRoots => {
                let roots = handlers::fs::list_roots().await;
                send_ok(&out_tx, stream_id, ResponseBody::Roots(roots));
            }
            Request::Exec { command, args, cwd, timeout_secs } => {
                // 客户端可以要求更短的超时，但不能超过 Agent 配置的上限。
                let effective_timeout = if timeout_secs == 0 { ctx.exec_timeout_secs } else { timeout_secs.min(ctx.exec_timeout_secs) };
                ctx.audit.record("exec", "", &format!("{command} {args:?} (cwd={cwd})")).await;
                match handlers::exec::exec(&command, &args, &cwd, effective_timeout).await {
                    Ok((exit_code, output)) => send_ok(&out_tx, stream_id, ResponseBody::ExecResult { exit_code, output }),
                    Err((code, msg)) => send_error(&out_tx, stream_id, code, msg),
                }
            }
            Request::SearchContent { root, query, options } => match guarded_path(&root, &ctx) {
                Ok(p) => {
                    let result = tokio::task::spawn_blocking(move || handlers::search::search_content(&p, &query, &options))
                        .await
                        .map_err(|e| (ErrorCode::Internal, e.to_string()))
                        .and_then(|r| r);
                    match result {
                        Ok(results) => send_ok(&out_tx, stream_id, ResponseBody::SearchResults(results)),
                        Err((code, msg)) => send_error(&out_tx, stream_id, code, msg),
                    }
                }
                Err(code) => send_error(&out_tx, stream_id, code, format!("路径不在允许访问的范围内: {root}")),
            },
            Request::SearchFileName { root, query } => match guarded_path(&root, &ctx) {
                Ok(p) => {
                    let result = tokio::task::spawn_blocking(move || handlers::search::search_filename(&p, &query))
                        .await
                        .map_err(|e| (ErrorCode::Internal, e.to_string()))
                        .and_then(|r| r);
                    match result {
                        Ok(results) => send_ok(&out_tx, stream_id, ResponseBody::SearchResults(results)),
                        Err((code, msg)) => send_error(&out_tx, stream_id, code, msg),
                    }
                }
                Err(code) => send_error(&out_tx, stream_id, code, format!("路径不在允许访问的范围内: {root}")),
            },
        }
    });
}

async fn handle_read_file(path: String, max_bytes: Option<u64>, stream_id: u32, out_tx: &OutSender, ctx: &ServerContext) {
    let p = match guarded_path(&path, ctx) {
        Ok(p) => p,
        Err(code) => return send_error(out_tx, stream_id, code, format!("路径不在允许访问的范围内: {path}")),
    };
    match handlers::fs::read_file(&p, max_bytes).await {
        Ok((bytes, mtime)) => {
            send_ok(out_tx, stream_id, ResponseBody::FileMeta { size: bytes.len() as u64, mtime });
            for chunk in bytes.chunks(DATA_CHUNK_SIZE) {
                if out_tx.send((stream_id, FrameType::DataChunk, chunk.to_vec())).is_err() {
                    return;
                }
            }
            let _ = out_tx.send((stream_id, FrameType::StreamEnd, Vec::new()));
        }
        Err((code, msg)) => send_error(out_tx, stream_id, code, msg),
    }
}

/// `WriteFile` 的后续 `DataChunk`/`StreamEnd` 帧由读循环通过 `rx` 转发过来
/// （见 `handle_connection` 里的 `write_waiters`），这个任务只管攒字节、等结束、
/// 落盘、回一个 Control 响应。
fn spawn_write_file(path: String, expected_mtime: Option<i64>, stream_id: u32, mut rx: mpsc::Receiver<Frame>, out_tx: OutSender, ctx: Arc<ServerContext>) {
    tokio::spawn(async move {
        let p = match guarded_path(&path, &ctx) {
            Ok(p) => p,
            Err(code) => return send_error(&out_tx, stream_id, code, format!("路径不在允许访问的范围内: {path}")),
        };

        let mut buf = Vec::new();
        while let Some(frame) = rx.recv().await {
            match frame.frame_type {
                FrameType::DataChunk => buf.extend_from_slice(&frame.payload),
                FrameType::StreamEnd => break,
                _ => {}
            }
        }

        match handlers::fs::write_file(&p, &buf, expected_mtime).await {
            Ok(None) => match handlers::fs::stat(&p).await {
                Ok((_, mtime, _)) => send_ok(&out_tx, stream_id, ResponseBody::Written { mtime }),
                Err((code, msg)) => send_error(&out_tx, stream_id, code, msg),
            },
            Ok(Some((current_mtime, _expected, preview))) => {
                let payload = encode_json(&Response::Conflict {
                    current_mtime,
                    current_preview: String::from_utf8_lossy(&preview).into_owned(),
                })
                .unwrap_or_default();
                let _ = out_tx.send((stream_id, FrameType::Control, payload));
            }
            Err((code, msg)) => send_error(&out_tx, stream_id, code, msg),
        }
    });
}

/// `OpenShell`：开一个真正的 ConPTY + `powershell.exe`（AGENT_DESIGN.md §四.4
/// Phase 2）。和 `spawn_write_file` 同样的"注册等待者、读循环转发后续帧"模式，
/// 只是这里的流是长期双向的——PTY 输出经独立 OS 线程阻塞读、`out_tx` 推回去；
/// 客户端方向的帧（键盘输入 `DataChunk`/`ShellResize` Control/关闭 `StreamEnd`）
/// 由本任务消费 `rx` 处理。
fn spawn_shell(cols: u16, rows: u16, cwd: String, stream_id: u32, mut rx: mpsc::Receiver<Frame>, out_tx: OutSender) {
    tokio::spawn(async move {
        let opened = match tokio::task::spawn_blocking(move || handlers::shell::open(cols, rows, &cwd)).await {
            Ok(Ok(o)) => o,
            Ok(Err((code, msg))) => return send_error(&out_tx, stream_id, code, msg),
            Err(e) => return send_error(&out_tx, stream_id, ErrorCode::Internal, e.to_string()),
        };
        send_ok(&out_tx, stream_id, ResponseBody::Empty);

        let handlers::shell::OpenedShell { master, mut writer, mut child, reader } = opened;

        // 阻塞读用独立 OS 线程，不占 tokio 工作线程池——PTY 读端在 shell 退出前会
        // 一直阻塞在 read() 上（和 src-tauri/src/pty/mod.rs 同一个理由）。
        let out_tx_reader = out_tx.clone();
        std::thread::spawn(move || {
            let mut reader = reader;
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if out_tx_reader.send((stream_id, FrameType::DataChunk, buf[..n].to_vec())).is_err() {
                            break;
                        }
                    }
                }
            }
            let _ = out_tx_reader.send((stream_id, FrameType::StreamEnd, Vec::new()));
        });

        while let Some(frame) = rx.recv().await {
            match frame.frame_type {
                FrameType::DataChunk => {
                    if tokio::task::block_in_place(|| writer.write_all(&frame.payload)).is_err() {
                        break;
                    }
                }
                FrameType::Control => {
                    if let Ok(Request::ShellResize { cols, rows }) = decode_json::<Request>(&frame.payload) {
                        handlers::shell::resize(master.as_ref(), cols, rows);
                    }
                }
                FrameType::StreamEnd => break,
                FrameType::Error => {}
            }
        }
        let _ = tokio::task::spawn_blocking(move || child.kill()).await;
    });
}
