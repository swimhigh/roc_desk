//! Windows 服务安装/卸载/常驻运行（AGENT_DESIGN.md §六.1）。只在 Windows 上编译——
//! Agent 本身就是给 Windows 目标机器用的，这个模块不需要跨平台。
//!
//! 服务安装的注册命令行是 `<exe路径> service-run`（一个不出现在 `--help` 里的隐藏
//! 子命令）：Windows 服务控制管理器（SCM）启动服务时执行的是这一整行命令，
//! `roc_desk_agent.exe run`（前台模式）和 `roc_desk_agent.exe service-run`
//! （SCM 启动的服务模式）走的是两条不同的代码路径——前者是一个普通的 tokio
//! 运行时 + Ctrl+C 退出；后者要调用 `service_dispatcher::start` 把控制权交给
//! Windows 服务框架，且不能有控制台（`service_main` 里也不能直接 `println!`，
//! 只能写日志文件）。

use std::ffi::OsString;
use std::sync::mpsc as std_mpsc;
use std::time::Duration;

use windows_service::service::{
    ServiceAccess, ServiceControl, ServiceControlAccept, ServiceErrorControl, ServiceExitCode, ServiceInfo, ServiceStartType,
    ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
use windows_service::{define_windows_service, service_dispatcher};

const SERVICE_NAME: &str = "RocDeskAgent";
const SERVICE_DISPLAY_NAME: &str = "roc_desk Agent";

pub fn install() -> Result<(), String> {
    let exe_path = std::env::current_exe().map_err(|e| e.to_string())?;
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CREATE_SERVICE).map_err(|e| e.to_string())?;
    let service_info = ServiceInfo {
        name: OsString::from(SERVICE_NAME),
        display_name: OsString::from(SERVICE_DISPLAY_NAME),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: exe_path,
        launch_arguments: vec![OsString::from("service-run")],
        dependencies: vec![],
        // 服务账户默认使用运行安装命令时的账户权限（AGENT_DESIGN.md §六.1）：
        // `None` 让 SCM 用调用者当前会话的账户,管理员应按最小权限原则自行决定
        // 是否改用专用服务账户（那需要另外用 `sc config` 或服务管理器 UI 设置密码，
        // 这里不代为决策）。
        account_name: None,
        account_password: None,
    };
    let service = manager
        .create_service(&service_info, ServiceAccess::CHANGE_CONFIG | ServiceAccess::START)
        .map_err(|e| e.to_string())?;
    service.set_description("roc_desk 远程 Windows Agent：常驻后台，供 roc_desk 客户端连接").map_err(|e| e.to_string())?;
    Ok(())
}

pub fn uninstall() -> Result<(), String> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT).map_err(|e| e.to_string())?;
    let service = manager
        .open_service(SERVICE_NAME, ServiceAccess::STOP | ServiceAccess::DELETE | ServiceAccess::QUERY_STATUS)
        .map_err(|e| e.to_string())?;

    if let Ok(status) = service.query_status() {
        if status.current_state != ServiceState::Stopped {
            service.stop().map_err(|e| e.to_string())?;
            std::thread::sleep(Duration::from_secs(2));
        }
    }
    service.delete().map_err(|e| e.to_string())
}

define_windows_service!(ffi_service_main, service_main);

/// SCM 通过 `service_dispatcher::start` 调用这个入口；一旦返回，Windows 认为
/// 服务已经停止。真正的监听逻辑在独立线程里跑一个 tokio 运行时，主线程只负责
/// 响应 SCM 的控制请求（Stop）并在收到时把 tokio 运行时敲停。
fn service_main(_arguments: Vec<OsString>) {
    let (shutdown_tx, shutdown_rx) = std_mpsc::channel::<()>();

    let event_handler = move |control_event| -> ServiceControlHandlerResult {
        match control_event {
            ServiceControl::Stop | ServiceControl::Shutdown => {
                let _ = shutdown_tx.send(());
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        }
    };

    let Ok(status_handle) = service_control_handler::register(SERVICE_NAME, event_handler) else { return };
    let set_status = |state: ServiceState, accept: ServiceControlAccept| {
        let _ = status_handle.set_service_status(ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: state,
            controls_accepted: accept,
            exit_code: ServiceExitCode::Win32(0),
            checkpoint: 0,
            wait_hint: Duration::default(),
            process_id: None,
        });
    };

    set_status(ServiceState::Running, ServiceControlAccept::STOP);

    let _runtime_thread = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("build tokio runtime");
        runtime.block_on(async {
            if let Err(e) = crate::run_agent().await {
                tracing::error!(error = %e, "Agent 服务运行出错");
            }
        });
    });

    // 阻塞等待 SCM 发来的 Stop——服务框架本身没有"优雅关闭 tokio 运行时里正在跑的
    // TCP 监听"这回事，这里选择最简单的做法：收到 Stop 就直接退出进程
    // （已建立的客户端连接会被操作系统关闭套接字，客户端按既有的断线重连逻辑处理）。
    let _ = shutdown_rx.recv();
    set_status(ServiceState::StopPending, ServiceControlAccept::empty());
    std::process::exit(0);
}

pub fn run_dispatcher() -> Result<(), String> {
    service_dispatcher::start(SERVICE_NAME, ffi_service_main).map_err(|e| e.to_string())
}
