//! 交互式终端（AGENT_DESIGN.md §四.4 Phase 2）：Windows 上 `portable-pty` 底层走
//! ConPTY，和 `src-tauri/src/pty/mod.rs`（本地终端）用的是同一个库/同一个版本，
//! 只是这边的 PTY 输出经网络帧传回客户端，不是 Tauri 事件。

use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};

use roc_desk_protocol::ErrorCode;

pub struct OpenedShell {
    pub master: Box<dyn MasterPty + Send>,
    pub writer: Box<dyn std::io::Write + Send>,
    pub child: Box<dyn portable_pty::Child + Send + Sync>,
    pub reader: Box<dyn std::io::Read + Send>,
}

/// `portable-pty` 在 Windows 上硬编码走 ConPTY（`CreatePseudoConsole`，Windows 10
/// 1809 / Server 2019 才引入），没有面向更老系统的 WinPTY 兜底——直接调用在缺
/// 这个 API 的系统上会从库内部 `panic!`（"this system does not support
/// conpty"），只探测 `kernel32.dll` 里有没有这个符号，不实际调用它，成本几乎为零。
fn conpty_available() -> bool {
    use windows::core::s;
    use windows::Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress};
    unsafe {
        let Ok(kernel32) = GetModuleHandleA(s!("kernel32.dll")) else { return false };
        GetProcAddress(kernel32, s!("CreatePseudoConsole")).is_some()
    }
}

/// 同步阻塞调用（开 ConPTY、起子进程），调用方负责 `spawn_blocking`。
pub fn open(cols: u16, rows: u16, cwd: &str) -> Result<OpenedShell, (ErrorCode, String)> {
    if !conpty_available() {
        return Err((
            ErrorCode::Internal,
            "此 Windows 版本不支持交互式终端（ConPTY 需要 Windows 10 1809 / Windows Server 2019 及以上）。\
             文件浏览、读写、命令执行不受影响，仅交互式终端功能在这台机器上不可用。"
                .to_string(),
        ));
    }

    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
        .map_err(|e| (ErrorCode::Internal, format!("open pty failed: {e}")))?;

    let mut cmd = CommandBuilder::new("powershell.exe");
    if !cwd.is_empty() {
        cmd.cwd(cwd);
    }
    let child = pair.slave.spawn_command(cmd).map_err(|e| (ErrorCode::Internal, format!("spawn shell failed: {e}")))?;
    // slave 端这边不再需要——不 drop 的话 master 侧的读端永远等不到 EOF
    // （和 src-tauri/src/pty/mod.rs 同一个坑）。
    drop(pair.slave);

    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| (ErrorCode::Internal, format!("clone pty reader failed: {e}")))?;
    let writer = pair.master.take_writer().map_err(|e| (ErrorCode::Internal, format!("take pty writer failed: {e}")))?;

    Ok(OpenedShell { master: pair.master, writer, child, reader })
}

pub fn resize(master: &dyn MasterPty, cols: u16, rows: u16) {
    let _ = master.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 });
}
