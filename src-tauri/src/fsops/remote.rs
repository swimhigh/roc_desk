use async_trait::async_trait;
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex;
use uuid::Uuid;

use super::encoding::decode_text;
use super::{FileEntry, FileOps, WriteOutcome};
use crate::error::AppError;
use crate::ssh::session::SshSession;
use std::sync::Arc;

/// 目录级传输的粗粒度进度反馈（DESIGN.md §3.3 双栏浏览器）：按"完成了第几个文件"
/// 报告，不做字节级百分比，见 `download_recursive`/`upload_recursive` 的文档注释。
fn emit_progress(progress: &Option<(AppHandle, Uuid)>, path: &str) {
    if let Some((app, request_id)) = progress {
        let _ = app.emit("sftp:transfer-progress", serde_json::json!({ "requestId": request_id, "path": path }));
    }
}

/// 远程文件操作（DESIGN.md §3.1.4、§3.3）：通过 SFTP 读写，`write_file` 的
/// `expected_mtime` 冲突检测逻辑与 `LocalFileOps` 保持一致的行为契约。
pub struct RemoteFileOps {
    session: Arc<SshSession>,
    /// SFTP 子系统握手有成本，懒建立后在这条 `RemoteFileOps` 的生命周期内复用。
    sftp: Mutex<Option<russh_sftp::client::SftpSession>>,
}

impl RemoteFileOps {
    pub fn new(session: Arc<SshSession>) -> Self {
        Self { session, sftp: Mutex::new(None) }
    }

    async fn with_sftp<F, T>(&self, f: F) -> Result<T, AppError>
    where
        F: for<'a> FnOnce(
            &'a russh_sftp::client::SftpSession,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<T, AppError>> + Send + 'a>>,
    {
        let mut guard = self.sftp.lock().await;
        if guard.is_none() {
            *guard = Some(self.session.open_sftp().await?);
        }
        f(guard.as_ref().unwrap()).await
    }

    /// 流式下载到本地磁盘（SFTP 快捷工具的批量传输，不经过 `FileContent` 的
    /// 整篇字符串转换，避免大文件把内容整个搬进 Rust 侧的 `String`）。
    pub async fn download_to_local(&self, remote_path: &str, local_path: &str) -> Result<(), AppError> {
        let remote_path = remote_path.to_string();
        let local_path = local_path.to_string();
        self.with_sftp(move |sftp| {
            Box::pin(async move {
                let mut remote_file = sftp
                    .open(&remote_path)
                    .await
                    .map_err(|e| AppError::NotFound(format!("open {remote_path} failed: {e}")))?;
                let mut local_file = tokio::fs::File::create(&local_path)
                    .await
                    .map_err(AppError::from)?;
                tokio::io::copy(&mut remote_file, &mut local_file)
                    .await
                    .map_err(|e| AppError::Internal(e.to_string()))?;
                Ok(())
            })
        })
        .await
    }

    pub async fn upload_from_local(&self, local_path: &str, remote_path: &str) -> Result<(), AppError> {
        let local_path = local_path.to_string();
        let remote_path = remote_path.to_string();
        self.with_sftp(move |sftp| {
            Box::pin(async move {
                let mut local_file = tokio::fs::File::open(&local_path).await.map_err(AppError::from)?;
                let mut remote_file = sftp
                    .create(&remote_path)
                    .await
                    .map_err(|e| AppError::PermissionDenied(format!("create {remote_path} failed: {e}")))?;
                tokio::io::copy(&mut local_file, &mut remote_file)
                    .await
                    .map_err(|e| AppError::Internal(e.to_string()))?;
                Ok(())
            })
        })
        .await
    }

    async fn is_remote_dir(&self, path: &str) -> Result<bool, AppError> {
        let path = path.to_string();
        self.with_sftp(move |sftp| {
            Box::pin(async move {
                let attrs = sftp
                    .metadata(&path)
                    .await
                    .map_err(|e| AppError::NotFound(format!("stat {path} failed: {e}")))?;
                Ok(attrs.is_dir())
            })
        })
        .await
    }

    async fn create_remote_dir(&self, path: &str) -> Result<(), AppError> {
        let path = path.to_string();
        self.with_sftp(move |sftp| {
            Box::pin(async move {
                match sftp.create_dir(&path).await {
                    Ok(()) => Ok(()),
                    // 目录已存在不算错误——上传一个已经部分传过的目录树时很常见。
                    Err(e) if e.to_string().to_lowercase().contains("failure") => Ok(()),
                    Err(e) => Err(AppError::PermissionDenied(format!("mkdir {path} failed: {e}"))),
                }
            })
        })
        .await
    }

    /// 递归下载整个远程目录（DESIGN.md §3.3 双栏 SFTP 浏览器）。SFTP 协议没有
    /// "打包传输整个目录"这回事，只能自己遍历——文件复用 `download_to_local`，
    /// 目录先在本地建好对应子目录再递归。用 `Box::pin` 打破递归 async fn 的
    /// 无限尺寸问题（标准写法，不需要额外的 crate）。
    ///
    /// `progress` 不做字节级百分比——那需要先完整遍历一遍算总大小，再在拷贝循环里
    /// 手动分块读写替换掉 `tokio::io::copy`，复杂度不小；退而求其次按"已完成第几个
    /// 文件"报进度，够让用户知道"还在传、没卡死"，这是够用和精确之间的取舍。
    pub async fn download_recursive(&self, remote_path: &str, local_path: &str, progress: Option<(AppHandle, Uuid)>) -> Result<(), AppError> {
        if !self.is_remote_dir(remote_path).await? {
            self.download_to_local(remote_path, local_path).await?;
            emit_progress(&progress, remote_path);
            return Ok(());
        }
        tokio::fs::create_dir_all(local_path).await.map_err(AppError::from)?;
        let entries = self.list_dir(remote_path).await?;
        for entry in entries {
            let local_child = format!("{}/{}", local_path.trim_end_matches('/'), entry.name);
            Box::pin(self.download_recursive(&entry.path, &local_child, progress.clone())).await?;
        }
        Ok(())
    }

    /// 递归上传整个本地目录，`upload_recursive` 与 `download_recursive` 对称。
    pub async fn upload_recursive(&self, local_path: &str, remote_path: &str, progress: Option<(AppHandle, Uuid)>) -> Result<(), AppError> {
        let meta = tokio::fs::metadata(local_path).await.map_err(AppError::from)?;
        if !meta.is_dir() {
            self.upload_from_local(local_path, remote_path).await?;
            emit_progress(&progress, local_path);
            return Ok(());
        }
        self.create_remote_dir(remote_path).await?;
        let mut read_dir = tokio::fs::read_dir(local_path).await.map_err(AppError::from)?;
        while let Some(entry) = read_dir.next_entry().await.map_err(AppError::from)? {
            let name = entry.file_name().to_string_lossy().to_string();
            let child_local = entry.path().to_string_lossy().replace('\\', "/");
            let child_remote = format!("{}/{}", remote_path.trim_end_matches('/'), name);
            Box::pin(self.upload_recursive(&child_local, &child_remote, progress.clone())).await?;
        }
        Ok(())
    }
}

fn mtime_of(attrs: &russh_sftp::protocol::FileAttributes) -> i64 {
    attrs.mtime.unwrap_or(0) as i64
}

#[async_trait]
impl FileOps for RemoteFileOps {
    async fn read_file_raw(&self, path: &str) -> Result<(Vec<u8>, i64), AppError> {
        let path = path.to_string();
        self.with_sftp(move |sftp| {
            Box::pin(async move {
                let attrs = sftp
                    .metadata(&path)
                    .await
                    .map_err(|e| AppError::NotFound(format!("stat {path} failed: {e}")))?;
                let mut file = sftp
                    .open(&path)
                    .await
                    .map_err(|e| AppError::NotFound(format!("open {path} failed: {e}")))?;
                let mut buf = Vec::new();
                file.read_to_end(&mut buf)
                    .await
                    .map_err(|e| AppError::Internal(e.to_string()))?;
                Ok((buf, mtime_of(&attrs)))
            })
        })
        .await
    }

    async fn write_file_bytes(
        &self,
        path: &str,
        bytes: &[u8],
        expected_mtime: Option<i64>,
    ) -> Result<WriteOutcome, AppError> {
        let path = path.to_string();
        let content = bytes.to_vec();
        self.with_sftp(move |sftp| {
            Box::pin(async move {
                // 保存前冲突检测（DESIGN.md §3.1.4）：远程 mtime 与打开时不一致就拒绝覆盖。
                if let Some(expected) = expected_mtime {
                    if let Ok(attrs) = sftp.metadata(&path).await {
                        let current = mtime_of(&attrs);
                        if current != expected {
                            let preview = match sftp.open(&path).await {
                                Ok(mut f) => {
                                    let mut buf = Vec::new();
                                    let _ = f.read_to_end(&mut buf).await;
                                    decode_text(&buf)
                                        .lines()
                                        .take(5)
                                        .collect::<Vec<_>>()
                                        .join("\n")
                                }
                                Err(_) => String::new(),
                            };
                            return Ok(WriteOutcome::Conflict { current_mtime: current, current_preview: preview });
                        }
                    }
                }

                let mut file = sftp
                    .create(&path)
                    .await
                    .map_err(|e| AppError::PermissionDenied(format!("create {path} failed: {e}")))?;
                file.write_all(&content)
                    .await
                    .map_err(|e| AppError::Internal(e.to_string()))?;

                let attrs = sftp
                    .metadata(&path)
                    .await
                    .map_err(|e| AppError::Internal(e.to_string()))?;
                Ok(WriteOutcome::Written { mtime: mtime_of(&attrs) })
            })
        })
        .await
    }

    async fn list_dir(&self, path: &str) -> Result<Vec<FileEntry>, AppError> {
        let path = path.to_string();
        self.with_sftp(move |sftp| {
            Box::pin(async move {
                let entries = sftp
                    .read_dir(&path)
                    .await
                    .map_err(|e| AppError::NotFound(format!("list_dir {path} failed: {e}")))?;

                let mut result: Vec<FileEntry> = entries
                    .into_iter()
                    .filter(|e| e.file_name() != "." && e.file_name() != "..")
                    .map(|e| {
                        let is_dir = e.file_type().is_dir();
                        let full_path = format!("{}/{}", path.trim_end_matches('/'), e.file_name());
                        FileEntry {
                            name: e.file_name().to_string(),
                            path: full_path,
                            is_dir,
                            size: if is_dir { None } else { Some(e.metadata().size.unwrap_or(0)) },
                            modified: e.metadata().mtime.map(|m| m as i64),
                        }
                    })
                    .collect();

                result.sort_by(|a, b| match (a.is_dir, b.is_dir) {
                    (true, false) => std::cmp::Ordering::Less,
                    (false, true) => std::cmp::Ordering::Greater,
                    _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                });
                Ok(result)
            })
        })
        .await
    }

    /// 目录分支不能直接把整个递归逻辑塞进一个 `with_sftp` 闭包——`self.list_dir`/
    /// 递归的 `self.delete` 各自会再去抢 `self.sftp` 那把锁，`Mutex` 不可重入，
    /// 锁在闭包里没释放就再抢会直接死锁。所以先在 `with_sftp` 之外把子项递归删完，
    /// 最后单独开一次 `with_sftp` 删这个（此时已经空了的）目录本身。
    async fn delete(&self, path: &str, is_dir: bool) -> Result<(), AppError> {
        if is_dir {
            let entries = self.list_dir(path).await?;
            for entry in entries {
                self.delete(&entry.path, entry.is_dir).await?;
            }
            let path = path.to_string();
            self.with_sftp(move |sftp| {
                Box::pin(async move {
                    sftp.remove_dir(&path)
                        .await
                        .map_err(|e| AppError::PermissionDenied(format!("删除远程目录 {path} 失败：{e}")))
                })
            })
            .await
        } else {
            let path = path.to_string();
            self.with_sftp(move |sftp| {
                Box::pin(async move {
                    sftp.remove_file(&path)
                        .await
                        .map_err(|e| AppError::PermissionDenied(format!("delete {path} failed: {e}")))
                })
            })
            .await
        }
    }

    async fn rename(&self, from: &str, to: &str) -> Result<(), AppError> {
        let from = from.to_string();
        let to = to.to_string();
        self.with_sftp(move |sftp| {
            Box::pin(async move {
                sftp.rename(&from, &to)
                    .await
                    .map_err(|e| AppError::PermissionDenied(format!("rename {from} -> {to} failed: {e}")))
            })
        })
        .await
    }

    async fn create_dir(&self, path: &str) -> Result<(), AppError> {
        self.create_remote_dir(path).await
    }
}
