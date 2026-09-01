use std::collections::HashMap;
use std::sync::Arc;

use russh::client::Handle;
use tauri::{AppHandle, Emitter};
use tokio::sync::{mpsc, Mutex};
use uuid::Uuid;

use super::handler::SshHandler;
use super::known_hosts::KnownHostsVerifier;
use crate::connection::{AuthMethod, ConnectionProfile};
use crate::error::AppError;

enum ChannelCommand {
    Data(Vec<u8>),
    Resize { rows: u16, cols: u16 },
}

/// 单条 SSH 物理连接，内部按 DESIGN.md §3.2.2 的多路复用要求管理多个 Channel
/// （终端 Shell / SFTP / exec 都在同一条连接上开各自的 Channel，不重复握手）。
///
/// 每个打开的 Channel 由独占它的后台任务驱动读写（russh 的 `Channel::wait()`
/// 需要 `&mut self`，不能塞进共享锁里跨 await 持有，否则会把并发写入卡死）；
/// 对外只暴露一个命令队列，`write`/`resize` 通过它转发给驱动任务。
pub struct SshSession {
    handle: Handle<SshHandler>,
    channels: Mutex<HashMap<Uuid, mpsc::UnboundedSender<ChannelCommand>>>,
}

impl SshSession {
    pub async fn connect(
        profile: &ConnectionProfile,
        secret: Option<String>,
        verifier: Arc<KnownHostsVerifier>,
    ) -> Result<Self, AppError> {
        let host = profile.host.trim();
        let config = Arc::new(russh::client::Config::default());
        let sh = SshHandler {
            host: host.to_string(),
            port: profile.port,
            verifier,
        };

        let mut handle = russh::client::connect(config, (host, profile.port), sh)
            .await
            .map_err(|e| AppError::Connection(e.to_string()))?;

        let authenticated = match profile.auth_method {
            AuthMethod::Password => {
                let password = secret.ok_or_else(|| AppError::Auth("missing password".into()))?;
                handle
                    .authenticate_password(&profile.username, password)
                    .await
                    .map_err(|e| AppError::Auth(e.to_string()))?
            }
            AuthMethod::Key => {
                let key_data = secret.ok_or_else(|| AppError::Auth("missing private key".into()))?;
                let key_pair = russh::keys::decode_secret_key(&key_data, None)
                    .map_err(|e| AppError::Auth(format!("invalid private key: {e}")))?;
                handle
                    .authenticate_publickey(&profile.username, Arc::new(key_pair))
                    .await
                    .map_err(|e| AppError::Auth(e.to_string()))?
            }
            AuthMethod::Agent => {
                return Err(AppError::Auth("SSH Agent 认证尚未实现".into()));
            }
        };

        if !authenticated {
            return Err(AppError::Auth("authentication rejected by server".into()));
        }

        Ok(Self { handle, channels: Mutex::new(HashMap::new()) })
    }

    /// 打开一个 Shell Channel（终端）。返回本地稳定 id，前端后续通过它调用
    /// `write`/`resize`；远端输出经 `ssh:data` 事件推送（CODE_DESIGN.md §七）。
    ///
    /// `cwd` 非空时，shell 起来后立即"帮用户敲一遍" `cd <cwd>`，让终端默认停在工作区
    /// 目录（参考 VS Code 打开项目后集成终端自动进到项目目录），而不是登录 shell 的
    /// 默认目录（通常是 $HOME）。SSH 的 `request_shell` 协议本身不支持指定初始工作
    /// 目录，只能开完 shell 之后当作一次普通输入发过去——所以这行 `cd` 命令和用户后续
    /// 手敲的命令没有本质区别，会正常出现在 shell 历史里，这是该方案本身的局限。
    pub async fn open_shell(&self, rows: u16, cols: u16, cwd: Option<&str>, app_handle: AppHandle) -> Result<Uuid, AppError> {
        let mut channel = self
            .handle
            .channel_open_session()
            .await
            .map_err(|e| AppError::Connection(e.to_string()))?;

        channel
            .request_pty(false, "xterm-256color", cols as u32, rows as u32, 0, 0, &[])
            .await
            .map_err(|e| AppError::Connection(e.to_string()))?;
        channel
            .request_shell(true)
            .await
            .map_err(|e| AppError::Connection(e.to_string()))?;

        if let Some(cwd) = cwd {
            if !cwd.is_empty() {
                let cmd = format!("cd {}\n", crate::log::remote::shell_quote(cwd));
                channel
                    .data(cmd.as_bytes())
                    .await
                    .map_err(|e| AppError::Connection(e.to_string()))?;
            }
        }

        let local_id = Uuid::new_v4();
        let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<ChannelCommand>();

        tokio::spawn(async move {
            // 打开 Channel 到这里为止全是同步返回给前端 `local_id` 之前的准备工作；
            // 前端拿到 `local_id` 之后还要经过一次 IPC 往返、Zustand 状态更新、
            // React 挂载 `TerminalView`、创建 xterm 实例，才会真正注册好
            // `listen("ssh:data", ...)` 监听——这段时间里如果这里已经开始
            // `channel.wait()` 读并 `emit`，服务器登录时的 MOTD/"Last login" 横幅
            // 大概率会在没人监听的窗口期被发出去，直接丢失（真实 bug：新开的 SSH
            // 终端永远只看到光秃秃的提示符，banner 从来没出现过）。Tauri 的事件
            // 没有"补发给迟到的监听者"这回事，丢了就是丢了。修法不是去猜一个刚好
            // 够用的延时，而是干脆不猜——服务器这时候发的数据只是安静地攒在 russh
            // 自己的 Channel 缓冲区里（`channel.wait()` 不调用它就不会被消费掉，
            // 不会丢），等前面这个小延时过去、前端十有八九已经把监听器挂上了，再
            // 一次性读出来往前端发，banner 自然就跟着第一批数据一起出现了。
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            loop {
                tokio::select! {
                    msg = channel.wait() => {
                        match msg {
                            Some(russh::ChannelMsg::Data { data }) => {
                                let _ = app_handle.emit("ssh:data", serde_json::json!({
                                    "channelId": local_id,
                                    "data": data.to_vec(),
                                }));
                            }
                            Some(russh::ChannelMsg::ExtendedData { data, .. }) => {
                                let _ = app_handle.emit("ssh:data", serde_json::json!({
                                    "channelId": local_id,
                                    "data": data.to_vec(),
                                }));
                            }
                            Some(russh::ChannelMsg::Eof) | Some(russh::ChannelMsg::Close) | None => {
                                let _ = app_handle.emit("ssh:status", serde_json::json!({
                                    "channelId": local_id,
                                    "status": "disconnected",
                                }));
                                break;
                            }
                            _ => {}
                        }
                    }
                    cmd = cmd_rx.recv() => {
                        match cmd {
                            Some(ChannelCommand::Data(bytes)) => {
                                if channel.data(&bytes[..]).await.is_err() {
                                    break;
                                }
                            }
                            Some(ChannelCommand::Resize { rows, cols }) => {
                                let _ = channel.window_change(cols as u32, rows as u32, 0, 0).await;
                            }
                            None => break, // 发送端（SshSession）已被丢弃
                        }
                    }
                }
            }
        });

        self.channels.lock().await.insert(local_id, cmd_tx);
        Ok(local_id)
    }

    pub async fn write(&self, local_id: Uuid, data: Vec<u8>) -> Result<(), AppError> {
        let channels = self.channels.lock().await;
        let tx = channels
            .get(&local_id)
            .ok_or_else(|| AppError::NotFound(format!("channel not found: {local_id}")))?;
        tx.send(ChannelCommand::Data(data))
            .map_err(|_| AppError::Internal("channel task has stopped".into()))
    }

    pub async fn resize(&self, local_id: Uuid, rows: u16, cols: u16) -> Result<(), AppError> {
        let channels = self.channels.lock().await;
        let tx = channels
            .get(&local_id)
            .ok_or_else(|| AppError::NotFound(format!("channel not found: {local_id}")))?;
        tx.send(ChannelCommand::Resize { rows, cols })
            .map_err(|_| AppError::Internal("channel task has stopped".into()))
    }

    /// 关闭一个终端 Channel（多终端场景下用户主动关闭某个 Tab）。丢弃命令发送端
    /// 会让驱动任务的 `cmd_rx.recv()` 收到 `None` 从而退出循环，连带释放 `channel`。
    pub async fn close_channel(&self, local_id: Uuid) -> Result<(), AppError> {
        self.channels.lock().await.remove(&local_id);
        Ok(())
    }

    /// 执行一次性命令并收集完整输出（AI 编程助手 `run_command` / 高危命令确认后走这个接口，
    /// 不复用终端 Shell Channel，见 DESIGN.md §3.8.4）。
    pub async fn exec(&self, cmd: &str) -> Result<String, AppError> {
        let mut channel = self
            .handle
            .channel_open_session()
            .await
            .map_err(|e| AppError::Connection(e.to_string()))?;
        channel
            .exec(true, cmd)
            .await
            .map_err(|e| AppError::Connection(e.to_string()))?;

        let mut output = Vec::new();
        while let Some(msg) = channel.wait().await {
            match msg {
                russh::ChannelMsg::Data { data } => output.extend_from_slice(&data),
                russh::ChannelMsg::ExtendedData { data, .. } => output.extend_from_slice(&data),
                russh::ChannelMsg::Eof | russh::ChannelMsg::Close => break,
                _ => {}
            }
        }
        Ok(String::from_utf8_lossy(&output).to_string())
    }

    /// 打开 SFTP 子系统 Channel，供 `fsops::remote::RemoteFileOps` 使用
    /// （DESIGN.md §3.3，同一物理连接上开独立 Channel，不重复握手）。
    pub async fn open_sftp(&self) -> Result<russh_sftp::client::SftpSession, AppError> {
        let channel = self
            .handle
            .channel_open_session()
            .await
            .map_err(|e| AppError::Connection(e.to_string()))?;
        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(|e| AppError::Connection(e.to_string()))?;
        russh_sftp::client::SftpSession::new(channel.into_stream())
            .await
            .map_err(|e| AppError::Connection(format!("sftp init failed: {e}")))
    }

    pub async fn disconnect(&self) -> Result<(), AppError> {
        self.handle
            .disconnect(russh::Disconnect::ByApplication, "", "en")
            .await
            .map_err(|e| AppError::Connection(e.to_string()))
    }

    /// `SshConnectionPool::get_or_connect` 在把缓存的会话交给调用方之前先问一句
    /// （真实 bug：网络掉线/服务器重启后，缓存的 `Handle` 已经死了，之前
    /// `get_or_connect` 不管三七二十一直接把它返回出去，之后开新 Channel/SFTP
    /// 全部失败，"重新连接"点了跟没点一样，不重启整个 app 这个连接就永远废了）。
    /// `Handle::is_closed()` 反映的是驱动这条连接的后台任务是不是已经退出——
    /// 网络断开、服务器主动断开都会让那个任务的读/写出错退出。
    pub fn is_alive(&self) -> bool {
        !self.handle.is_closed()
    }
}
