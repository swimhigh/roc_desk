//! 诊断用集成测试：直接模拟客户端（不经过 roc_desk.exe 的 UI/TOFU 弹窗那一层）
//! 对本进程内启动的 Agent 服务器完整走一遍 TLS 握手 -> `Handshake` -> `ListRoots`，
//! 用来定位"客户端卡在'加载中...'"到底是 Agent 服务器本身的帧协议处理有 bug，
//! 还是客户端那边（TOFU 证书确认弹窗）没接上。
//!
//! `agent` 是纯二进制 crate，没有 `[lib]` target，`tests/` 目录下的集成测试没法
//! `use` 到 `crate::` 里的模块——所以这个诊断测试直接放在二进制 crate 内部，
//! 用 `#[cfg(test)] mod selftest;` 引入。

use std::sync::Arc;
use std::time::Duration;

use roc_desk_protocol::{
    decode_json, encode_json, read_frame, write_frame, FrameType, Request, Response, ResponseBody, PROTOCOL_VERSION,
};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, Error as TlsError, SignatureScheme};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

/// 测试专用：无条件放行证书，不关心指纹（TOFU 判断是客户端 UI 层的逻辑，
/// 这个诊断测试只关心 Agent 服务器自己的协议处理对不对）。
#[derive(Debug)]
struct AcceptAllVerifier;

impl ServerCertVerifier for AcceptAllVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, TlsError> {
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(&self, _m: &[u8], _c: &CertificateDer<'_>, _d: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, TlsError> {
        Ok(HandshakeSignatureValid::assertion())
    }
    fn verify_tls13_signature(&self, _m: &[u8], _c: &CertificateDer<'_>, _d: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, TlsError> {
        Ok(HandshakeSignatureValid::assertion())
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![
            SignatureScheme::RSA_PKCS1_SHA256,
            SignatureScheme::RSA_PKCS1_SHA384,
            SignatureScheme::RSA_PKCS1_SHA512,
            SignatureScheme::ECDSA_NISTP256_SHA256,
            SignatureScheme::ECDSA_NISTP384_SHA384,
            SignatureScheme::ECDSA_NISTP521_SHA512,
            SignatureScheme::RSA_PSS_SHA256,
            SignatureScheme::RSA_PSS_SHA384,
            SignatureScheme::RSA_PSS_SHA512,
            SignatureScheme::ED25519,
        ]
    }
}

// 必须是多线程运行时——生产环境的 main.rs 用的就是
// `tokio::runtime::Builder::new_multi_thread()`，`spawn_shell` 里给 PTY 写端用的
// `tokio::task::block_in_place` 只在多线程运行时下才不会 panic（单线程运行时下
// 调用它会直接 panic："can call blocking only when running on the multi-threaded
// runtime"）——用默认的单线程 `#[tokio::test]` 会得到一个和生产环境不一致、
// 误导人的失败。
#[tokio::test(flavor = "multi_thread")]
async fn handshake_then_list_roots_roundtrip() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let dir = std::env::temp_dir().join(format!("roc_desk_agent_selftest_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let cert = crate::cert::load_or_generate(&dir).expect("generate cert");
    let token = "selftest-token".to_string();
    let token_hash = crate::auth::hash_token(&token);

    let mut config = crate::config::AgentConfig::default();
    config.server.port = 17879; // 固定测试端口，避免依赖 run() 回传实际监听地址
    config.security.token_hash = Some(token_hash);

    let audit = Arc::new(crate::audit::AuditLog::new(&dir));

    tokio::spawn(async move {
        let _ = crate::server::run(config, cert, audit).await;
    });
    // 给 accept 循环一点时间起来——本地回环连接，不需要太久。
    tokio::time::sleep(Duration::from_millis(300)).await;

    let outcome = tokio::time::timeout(Duration::from_secs(20), async move {
        let tcp = TcpStream::connect(("127.0.0.1", 17879)).await.expect("tcp connect");

        let tls_config = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(AcceptAllVerifier))
            .with_no_client_auth();
        let connector = TlsConnector::from(Arc::new(tls_config));
        let server_name = ServerName::try_from("127.0.0.1").unwrap().to_owned();
        let tls_stream = connector.connect(server_name, tcp).await.expect("tls handshake");
        let (mut reader, mut writer) = tokio::io::split(tls_stream);

        // --- Handshake ---
        let handshake_req = Request::Handshake { token, protocol_version: PROTOCOL_VERSION, client_version: "selftest".into() };
        write_frame(&mut writer, 1, FrameType::Control, &encode_json(&handshake_req).unwrap()).await.expect("send handshake");
        let frame = read_frame(&mut reader).await.expect("read handshake response frame");
        assert_eq!(frame.stream_id, 1);
        let response: Response = decode_json(&frame.payload).expect("decode handshake response");
        match response {
            Response::Ok(ResponseBody::Handshake { .. }) => {}
            other => panic!("unexpected handshake response: {other:?}"),
        }

        // --- ListRoots ---
        write_frame(&mut writer, 2, FrameType::Control, &encode_json(&Request::ListRoots).unwrap()).await.expect("send list_roots");
        let frame = read_frame(&mut reader).await.expect("read list_roots response frame");
        assert_eq!(frame.stream_id, 2);
        let response: Response = decode_json(&frame.payload).expect("decode list_roots response");
        match response {
            Response::Ok(ResponseBody::Roots(roots)) => {
                println!("roots = {roots:?}");
                assert!(!roots.is_empty(), "expected at least one drive letter on the host running this test");

                // --- ListDir（Entries 也是数组形状，和 Roots 同一类 bug，一并验证）---
                let root = roots[0].clone();
                write_frame(&mut writer, 3, FrameType::Control, &encode_json(&Request::ListDir { path: root }).unwrap())
                    .await
                    .expect("send list_dir");
                let frame = read_frame(&mut reader).await.expect("read list_dir response frame");
                assert_eq!(frame.stream_id, 3);
                let response: Response = decode_json(&frame.payload).expect("decode list_dir response");
                match response {
                    Response::Ok(ResponseBody::Entries(entries)) => println!("entries = {} 项", entries.len()),
                    other => panic!("unexpected list_dir response: {other:?}"),
                }
            }
            other => panic!("unexpected list_roots response: {other:?}"),
        }

        // --- OpenShell（交互式终端，AGENT_DESIGN.md §四.4 Phase 2）---
        let open_shell_req = Request::OpenShell { cols: 80, rows: 24, cwd: String::new() };
        write_frame(&mut writer, 4, FrameType::Control, &encode_json(&open_shell_req).unwrap()).await.expect("send open_shell");
        let frame = read_frame(&mut reader).await.expect("read open_shell ack frame");
        assert_eq!(frame.stream_id, 4);
        let response: Response = decode_json(&frame.payload).expect("decode open_shell ack");
        match response {
            Response::Ok(ResponseBody::Empty) => {}
            other => panic!("unexpected open_shell response: {other:?}"),
        }

        // 敲一条能唯一识别的命令，从 PTY 输出里找回它——ConPTY 的输出里混着提示符/
        // ANSI 转义序列，不追求精确解析，只要这个标记字符串出现过就说明"键盘输入
        // 经 DataChunk 送进去、shell 输出经 DataChunk 传回来"这条双向链路是通的。
        write_frame(&mut writer, 4, FrameType::DataChunk, b"echo ROC_DESK_AGENT_SELFTEST_MARKER\r")
            .await
            .expect("send shell input");

        let mut collected = String::new();
        let found = loop {
            let frame = read_frame(&mut reader).await.expect("read shell output frame");
            assert_eq!(frame.stream_id, 4);
            match frame.frame_type {
                FrameType::DataChunk => {
                    collected.push_str(&String::from_utf8_lossy(&frame.payload));
                    if collected.contains("ROC_DESK_AGENT_SELFTEST_MARKER") {
                        break true;
                    }
                }
                FrameType::StreamEnd => break false,
                _ => {}
            }
        };
        assert!(found, "PTY 输出里没有找到回显的标记字符串，实际收到：{collected:?}");

        // --- 关闭终端：客户端发 StreamEnd，Agent 应该杀掉 PTY 子进程 ---
        write_frame(&mut writer, 4, FrameType::StreamEnd, &[]).await.expect("send shell close");
    })
    .await;

    outcome.expect("整个握手 + ListRoots 往返在 10 秒内没有完成——说明 Agent 服务器这一侧的帧协议处理卡住了");
}
