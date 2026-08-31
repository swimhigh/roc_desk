mod audit;
mod auth;
mod cert;
mod config;
mod handlers;
mod pathguard;
#[cfg(test)]
mod selftest;
mod server;
#[cfg(windows)]
mod service;

use std::sync::Arc;

use clap::{Parser, Subcommand};

use config::AgentConfig;

#[derive(Parser)]
#[command(name = "roc_desk_agent", version, about = "roc_desk 远程 Windows Agent（AGENT_DESIGN.md）")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// 前台运行，日志打印到控制台（测试/临时场景）
    Run,
    /// 生成/重新生成配对令牌，打印给管理员
    Pair,
    /// 注册为 Windows 服务（开机自启、后台常驻）
    InstallService,
    /// 移除 Windows 服务
    UninstallService,
    /// 查看当前监听地址、配对状态
    Status,
    /// SCM 启动服务时实际执行的命令；不面向用户，只由 `install-service` 注册的
    /// 启动命令行使用，因此从 `--help` 里隐藏。
    #[command(hide = true)]
    ServiceRun,
}

/// 没有这个 hook 时，任何线程 panic 都是"无声消失"——`run`/`service-run` 都没有
/// 交互式控制台盯着看，默认 panic hook 打印到 stderr 基本等于打进黑洞，用户只会看到
/// "进程不在了"，什么线索都留不下（和 src-tauri/src/lib.rs::install_panic_hook 是
/// 同一个理由、同一个写法）。`tokio::spawn` 出去的每连接任务 panic 默认不会带崩
/// 整个进程（只是那一个任务失败），但装上这个 hook 至少能在日志里看到到底崩在哪。
fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "unknown".to_string());
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "非字符串 panic payload".to_string());
        let backtrace = std::backtrace::Backtrace::force_capture();
        tracing::error!(target: "panic", %location, %message, %backtrace, "roc_desk_agent 发生 panic");
        default_hook(info);
    }));
}

fn init_logging() {
    let log_dir = config::exe_dir().join("logs");
    let _ = std::fs::create_dir_all(&log_dir);
    let file_writer = tracing_appender::rolling::daily(&log_dir, "roc_desk_agent.log");
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    use tracing_subscriber::prelude::*;
    let _ = tracing_subscriber::registry()
        .with(env_filter)
        .with(tracing_subscriber::fmt::layer().with_writer(file_writer).with_ansi(false))
        .try_init();
}

fn main() {
    // 显式安装 `ring` 作为进程级默认加密提供方——只编译了 `ring` 一种 provider
    // （见 Cargo.toml 注释），但 rustls 0.23 的 `ServerConfig::builder()`/
    // `ClientConfig::builder()` 仍然要求进程启动时安装过一次默认 provider，
    // 不装会在第一次建 TLS 配置时 panic。重复安装（比如测试里多次调用 main）
    // 会返回 Err，直接忽略即可。
    let _ = rustls::crypto::ring::default_provider().install_default();
    let cli = Cli::parse();
    match cli.command.unwrap_or(Command::Run) {
        Command::Run => cmd_run(),
        Command::Pair => cmd_pair(),
        Command::InstallService => cmd_install_service(),
        Command::UninstallService => cmd_uninstall_service(),
        Command::Status => cmd_status(),
        Command::ServiceRun => cmd_service_run(),
    }
}

/// `run`/`service-run` 共用的启动逻辑：加载配置、加载/生成证书、起 TLS 监听。
pub async fn run_agent() -> Result<(), String> {
    let config = AgentConfig::load_or_default(&config::config_path()).map_err(|e| e.to_string())?;
    let cert = cert::load_or_generate(&config::exe_dir()).map_err(|e| e.to_string())?;
    let audit = Arc::new(audit::AuditLog::new(&config::exe_dir()));
    server::run(config, cert, audit).await.map_err(|e| e.to_string())
}

fn cmd_run() {
    init_logging();
    install_panic_hook();
    // 首次运行且 `agent.toml` 不存在时，把默认配置落盘——这样管理员后续能直接
    // 编辑同目录下的 `agent.toml` 调整 `allowed_roots`/端口，而不用先猜文件格式。
    let path = config::config_path();
    if !path.exists() {
        let _ = AgentConfig::default().save(&path);
    }
    if AgentConfig::load_or_default(&path).map(|c| c.security.token_hash.is_none()).unwrap_or(true) {
        println!("尚未配对：请先执行 `roc_desk_agent.exe pair` 生成配对令牌，再重新运行 `run`。");
        return;
    }

    println!("roc_desk_agent 正在前台运行，Ctrl+C 退出。日志同时写入 logs\\ 子目录。");
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("创建异步运行时失败: {e}");
            return;
        }
    };
    runtime.block_on(async {
        tokio::select! {
            result = run_agent() => {
                if let Err(e) = result {
                    eprintln!("Agent 运行出错: {e}");
                }
            }
            _ = tokio::signal::ctrl_c() => {
                println!("收到 Ctrl+C，退出。");
            }
        }
    });
}

fn cmd_pair() {
    let path = config::config_path();
    let mut cfg = AgentConfig::load_or_default(&path).unwrap_or_default();
    let token = auth::generate_token();
    cfg.security.token_hash = Some(auth::hash_token(&token));
    if let Err(e) = cfg.save(&path) {
        eprintln!("写入 agent.toml 失败: {e}");
        return;
    }
    println!("配对令牌（请通过可信信道交给需要连接的 roc_desk 用户，仅展示这一次）：");
    println!();
    println!("    {token}");
    println!();
    println!("重新执行 pair 会让旧令牌立即失效，此前所有基于旧令牌的客户端信任状态一并失效。");
}

fn cmd_status() {
    let path = config::config_path();
    let cfg = AgentConfig::load_or_default(&path).unwrap_or_default();
    println!("监听地址: {}:{}", cfg.server.listen_addr, cfg.server.port);
    println!("已配对: {}", if cfg.security.token_hash.is_some() { "是" } else { "否" });
    println!(
        "路径访问范围: {}",
        if cfg.security.allowed_roots.is_empty() { "不限制（整机）".to_string() } else { cfg.security.allowed_roots.join(", ") }
    );
    let cert_exists = config::exe_dir().join("cert.pem").exists();
    println!("TLS 证书: {}", if cert_exists { "已生成" } else { "尚未生成（首次 run 时自动生成）" });
}

#[cfg(windows)]
fn cmd_install_service() {
    match service::install() {
        Ok(()) => println!("已注册 Windows 服务 RocDeskAgent（开机自启）。可用 `sc start RocDeskAgent` 或服务管理器启动。"),
        Err(e) => eprintln!("注册服务失败: {e}"),
    }
}
#[cfg(not(windows))]
fn cmd_install_service() {
    eprintln!("install-service 仅支持 Windows。");
}

#[cfg(windows)]
fn cmd_uninstall_service() {
    match service::uninstall() {
        Ok(()) => println!("已移除 Windows 服务 RocDeskAgent。"),
        Err(e) => eprintln!("移除服务失败: {e}"),
    }
}
#[cfg(not(windows))]
fn cmd_uninstall_service() {
    eprintln!("uninstall-service 仅支持 Windows。");
}

#[cfg(windows)]
fn cmd_service_run() {
    init_logging();
    install_panic_hook();
    if let Err(e) = service::run_dispatcher() {
        tracing::error!(error = %e, "service dispatcher 启动失败");
    }
}
#[cfg(not(windows))]
fn cmd_service_run() {
    eprintln!("service-run 仅支持 Windows。");
}
