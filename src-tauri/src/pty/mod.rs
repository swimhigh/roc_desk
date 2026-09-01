use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::Arc;

use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use tauri::{AppHandle, Emitter};
use tokio::sync::{mpsc, RwLock};
use uuid::Uuid;

use crate::error::AppError;

enum PtyCommand {
    Data(Vec<u8>),
    Resize { rows: u16, cols: u16 },
}

struct PtyChannel {
    cmd_tx: mpsc::UnboundedSender<PtyCommand>,
    /// `Child` 被 drop 不会杀死子进程（和 `std::process::Child` 语义一致），必须显式
    /// `kill()`，否则关掉终端 Tab 只是不再监听输出，PowerShell/bash 进程会一直挂着——
    /// 这点和 SSH Channel 关闭时服务端会话跟着结束不一样，本地 PTY 没有这层保证。
    child: Box<dyn portable_pty::Child + Send + Sync>,
}

/// 本地终端（DESIGN.md §3.2 终端面板，本地工作区分支）：远程工作区走 SSH Channel，
/// 本地工作区没有 SSH 连接可复用，需要单独起一个真正的本地 PTY（Windows 上底层是
/// ConPTY）。读写都是同步系统调用，不能直接塞进 `tokio::select!`（那是给 async I/O
/// 设计的）——读走一个独立 OS 线程阻塞 `read()` 再把数据经 `AppHandle::emit` 推给
/// 前端，写走一个 `mpsc` 队列 + 单独任务串行处理，和 `ssh::session::SshSession`
/// 里"一个任务独占 Channel"的思路一致，只是读写各自换成了适合同步 API 的形式。
pub struct LocalPtyManager {
    channels: RwLock<HashMap<Uuid, PtyChannel>>,
}

impl Default for LocalPtyManager {
    fn default() -> Self {
        Self { channels: RwLock::new(HashMap::new()) }
    }
}

fn default_shell() -> CommandBuilder {
    #[cfg(target_os = "windows")]
    {
        CommandBuilder::new("powershell.exe")
    }
    #[cfg(not(target_os = "windows"))]
    {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
        CommandBuilder::new(shell)
    }
}

impl LocalPtyManager {
    /// 打开一个本地终端，默认工作目录就是当前工作区根目录（参考 VS Code 打开项目后
    /// 集成终端自动进到项目目录，而不是用户主目录）。
    pub async fn open(&self, cwd: String, rows: u16, cols: u16, app_handle: AppHandle) -> Result<Uuid, AppError> {
        let id = Uuid::new_v4();
        let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<PtyCommand>();

        let (master, writer, child) = tokio::task::spawn_blocking(move || -> Result<_, AppError> {
            let pty_system = native_pty_system();
            let pair = pty_system
                .openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
                .map_err(|e| AppError::Internal(format!("open pty failed: {e}")))?;

            let mut cmd = default_shell();
            cmd.cwd(&cwd);
            let child = pair
                .slave
                .spawn_command(cmd)
                .map_err(|e| AppError::Internal(format!("spawn shell failed: {e}")))?;
            // slave 端父进程这边不再需要了，子进程内部持有自己的一份；不 drop 的话
            // master 侧的读端永远等不到 EOF（slave 还有一个引用活着）。
            drop(pair.slave);

            let reader = pair
                .master
                .try_clone_reader()
                .map_err(|e| AppError::Internal(format!("clone pty reader failed: {e}")))?;
            let writer = pair
                .master
                .take_writer()
                .map_err(|e| AppError::Internal(format!("take pty writer failed: {e}")))?;

            spawn_reader_thread(id, reader, app_handle.clone());
            Ok((pair.master, writer, child))
        })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))??;

        let mut writer = writer;
        tokio::spawn(async move {
            while let Some(cmd) = cmd_rx.recv().await {
                match cmd {
                    PtyCommand::Data(bytes) => {
                        if tokio::task::block_in_place(|| writer.write_all(&bytes)).is_err() {
                            break;
                        }
                    }
                    PtyCommand::Resize { rows, cols } => {
                        let _ = master.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 });
                    }
                }
            }
        });

        self.channels.write().await.insert(id, PtyChannel { cmd_tx, child });
        Ok(id)
    }

    pub async fn write(&self, id: Uuid, data: Vec<u8>) -> Result<(), AppError> {
        let channels = self.channels.read().await;
        let ch = channels.get(&id).ok_or_else(|| AppError::NotFound(format!("pty channel not found: {id}")))?;
        ch.cmd_tx.send(PtyCommand::Data(data)).map_err(|_| AppError::Internal("pty task has stopped".into()))
    }

    pub async fn resize(&self, id: Uuid, rows: u16, cols: u16) -> Result<(), AppError> {
        let channels = self.channels.read().await;
        let ch = channels.get(&id).ok_or_else(|| AppError::NotFound(format!("pty channel not found: {id}")))?;
        ch.cmd_tx
            .send(PtyCommand::Resize { rows, cols })
            .map_err(|_| AppError::Internal("pty task has stopped".into()))
    }

    pub async fn close(&self, id: Uuid) -> Result<(), AppError> {
        if let Some(mut ch) = self.channels.write().await.remove(&id) {
            let _ = tokio::task::spawn_blocking(move || ch.child.kill()).await;
        }
        Ok(())
    }
}

fn spawn_reader_thread(id: Uuid, mut reader: Box<dyn Read + Send>, app_handle: AppHandle) {
    // 阻塞读用独立 OS 线程，不占 tokio 的工作线程池——PTY 读端在 shell 退出前会
    // 一直阻塞在 read() 上，扔进 tokio::task::spawn_blocking 也可以，但那个池子是
    // 有限且共享的，专用线程更简单直接，生命周期就是这一个终端 Tab 的生命周期。
    std::thread::spawn(move || {
        // 和 SSH/Agent 的 open_shell 同一个理由：前端拿到 channelId 后还要走一次 IPC
        // 往返 + React 挂载才会挂上 `listen("pty:data", ...)`，本地 shell 起来几乎
        // 是瞬间的事，比这个窗口期快得多，第一行提示符/MOTD 很容易在没人监听时被
        // `emit` 掉、白白丢失。这段时间里没人读，字节就安静地留在 PTY 主端的内核
        // 缓冲区里，不会丢——延后一小会儿再开始读，等前端监听器大概率已经挂上。
        std::thread::sleep(std::time::Duration::from_millis(200));
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let _ = app_handle.emit(
                        "pty:data",
                        serde_json::json!({ "channelId": id, "data": buf[..n].to_vec() }),
                    );
                }
                Err(_) => break,
            }
        }
        let _ = app_handle.emit("pty:status", serde_json::json!({ "channelId": id, "status": "disconnected" }));
    });
}

pub type SharedLocalPtyManager = Arc<LocalPtyManager>;
