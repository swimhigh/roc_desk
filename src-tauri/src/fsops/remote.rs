use async_trait::async_trait;
use russh_sftp::protocol::OpenFlags;
use std::io::SeekFrom;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncSeekExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::Mutex;
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

use super::encoding::decode_text;
use super::{FileEntry, FileOps, WriteOutcome};
use crate::error::AppError;
use crate::ssh::session::SshSession;
use std::sync::Arc;

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
    /// 整篇字符串转换，避免大文件把内容整个搬进 Rust 侧的 `String`）。一次性传输，
    /// 不需要断点续传/取消/进度回调的简单调用点（"用系统程序打开"、旧版 Office
    /// 转 PDF 的临时下载等）复用这个，等价于 `download_range_to_local` 从 0 开始。
    pub async fn download_to_local(&self, remote_path: &str, local_path: &str) -> Result<(), AppError> {
        self.download_range_to_local(remote_path, local_path, 0, Arc::new(|_| {}), Arc::new(|| false)).await
    }

    pub async fn upload_from_local(&self, local_path: &str, remote_path: &str) -> Result<(), AppError> {
        self.upload_range_from_local(local_path, remote_path, 0, Arc::new(|_| {}), Arc::new(|| false)).await
    }

    /// 断点续传的核心（用户 2026-09-07 需求）：从 `start_offset` 开始把远程文件剩余
    /// 部分续写到本地文件。调用方（`commands::sftp` 的重试编排）负责算好
    /// `start_offset`——通常就是本地文件当前已经写到的字节数——重试时只要重新读一次
    /// 本地文件大小再调一次这个函数，就能从断点接着传，这里本身不记录任何跨调用的
    /// 状态。分块读写（不用 `tokio::io::copy`）换来两个能力：每个 chunk 之间能检查
    /// 取消标记（之前整份 `tokio::io::copy` 中途没法取消，见 `download_recursive`
    /// 原来的"只在文件之间检查"），以及能按字节汇报进度。
    ///
    /// `start_offset == 0` 时会截断本地文件——不能假设本地没有同名旧文件，直接用
    /// "打开不截断"从 0 写会在新内容比旧文件短时留下一截旧数据在文件尾部。
    pub async fn download_range_to_local(
        &self,
        remote_path: &str,
        local_path: &str,
        start_offset: u64,
        on_progress: Arc<dyn Fn(u64) + Send + Sync>,
        should_cancel: Arc<dyn Fn() -> bool + Send + Sync>,
    ) -> Result<(), AppError> {
        if should_cancel() {
            return Err(AppError::Internal(super::TRANSFER_CANCELLED_MESSAGE.into()));
        }
        let remote_path = remote_path.to_string();
        let local_path = local_path.to_string();
        self.with_sftp(move |sftp| {
            Box::pin(async move {
                let mut remote_file = sftp
                    .open(&remote_path)
                    .await
                    .map_err(|e| AppError::NotFound(format!("open {remote_path} failed: {e}")))?;
                if start_offset > 0 {
                    remote_file
                        .seek(SeekFrom::Start(start_offset))
                        .await
                        .map_err(|e| AppError::Internal(format!("seek {remote_path} failed: {e}")))?;
                }
                let mut local_file = tokio::fs::OpenOptions::new()
                    .create(true)
                    .write(true)
                    .truncate(start_offset == 0)
                    .open(&local_path)
                    .await
                    .map_err(AppError::from)?;
                if start_offset > 0 {
                    local_file.seek(SeekFrom::Start(start_offset)).await.map_err(AppError::from)?;
                }
                copy_chunked(remote_file, local_file, start_offset, on_progress, should_cancel).await
            })
        })
        .await
    }

    /// 和 `download_range_to_local` 对称的续传上传。远程文件用 `WRITE`（续传时不带
    /// `TRUNCATE`）——`start_offset == 0` 才带上 `TRUNCATE`，理由和上面截断本地文件
    /// 一样：新内容比远程已有内容短时，不截断会留下一截旧数据在文件尾部。
    pub async fn upload_range_from_local(
        &self,
        local_path: &str,
        remote_path: &str,
        start_offset: u64,
        on_progress: Arc<dyn Fn(u64) + Send + Sync>,
        should_cancel: Arc<dyn Fn() -> bool + Send + Sync>,
    ) -> Result<(), AppError> {
        if should_cancel() {
            return Err(AppError::Internal(super::TRANSFER_CANCELLED_MESSAGE.into()));
        }
        let local_path = local_path.to_string();
        let remote_path = remote_path.to_string();
        self.with_sftp(move |sftp| {
            Box::pin(async move {
                let mut local_file = tokio::fs::File::open(&local_path).await.map_err(AppError::from)?;
                if start_offset > 0 {
                    local_file.seek(SeekFrom::Start(start_offset)).await.map_err(AppError::from)?;
                }
                let flags = if start_offset == 0 {
                    OpenFlags::CREATE | OpenFlags::TRUNCATE | OpenFlags::WRITE
                } else {
                    OpenFlags::CREATE | OpenFlags::WRITE
                };
                let mut remote_file = sftp
                    .open_with_flags(&remote_path, flags)
                    .await
                    .map_err(|e| AppError::PermissionDenied(format!("create {remote_path} failed: {e}")))?;
                if start_offset > 0 {
                    remote_file
                        .seek(SeekFrom::Start(start_offset))
                        .await
                        .map_err(|e| AppError::Internal(format!("seek remote {remote_path} failed: {e}")))?;
                }
                copy_chunked(local_file, remote_file, start_offset, on_progress, should_cancel).await
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

    async fn remote_size(&self, path: &str) -> Result<u64, AppError> {
        let path = path.to_string();
        self.with_sftp(move |sftp| Box::pin(async move {
            let attrs = sftp.metadata(&path).await.map_err(|e| AppError::NotFound(format!("stat {path} failed: {e}")))?;
            Ok(attrs.size.unwrap_or(0))
        })).await
    }

    pub async fn download_recursive(&self, remote_path: &str, local_path: &str, progress: Option<(AppHandle, Uuid)>, should_cancel: Arc<dyn Fn() -> bool + Send + Sync>, file_count: &std::sync::atomic::AtomicU64) -> Result<(), AppError> {
        if should_cancel() { return Err(AppError::Internal(super::TRANSFER_CANCELLED_MESSAGE.into())); }
        let entry = self.list_dir(remote_path).await;
        if let Ok(entries) = entry {
            tokio::fs::create_dir_all(local_path).await.map_err(AppError::from)?;
            for child in entries {
                let child_local = format!("{}/{}", local_path.trim_end_matches('/'), child.name);
                Box::pin(self.download_recursive(&child.path, &child_local, progress.clone(), should_cancel.clone(), file_count)).await?;
            }
            return Ok(());
        }
        let total_len = self.remote_size(remote_path).await.unwrap_or(0);
        let offset = match tokio::fs::metadata(local_path).await {
            Ok(meta) if meta.is_file() && meta.len() <= total_len => meta.len(),
            _ => 0,
        };
        let event_path = remote_path.to_string();
        let on_progress: Arc<dyn Fn(u64) + Send + Sync> = Arc::new(move |bytes| {
            if let Some((app, id)) = &progress { let _ = app.emit("sftp:transfer-progress", serde_json::json!({"requestId": id, "path": event_path, "bytes": bytes, "totalBytes": total_len})); }
        });
        self.download_range_to_local(remote_path, local_path, offset, on_progress, should_cancel.clone()).await?;
        file_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }

    pub async fn upload_recursive(&self, local_path: &str, remote_path: &str, progress: Option<(AppHandle, Uuid)>, should_cancel: Arc<dyn Fn() -> bool + Send + Sync>, file_count: &std::sync::atomic::AtomicU64) -> Result<(), AppError> {
        if should_cancel() { return Err(AppError::Internal(super::TRANSFER_CANCELLED_MESSAGE.into())); }
        let meta = tokio::fs::metadata(local_path).await.map_err(AppError::from)?;
        if meta.is_dir() {
            self.create_remote_dir(remote_path).await?;
            let mut dir = tokio::fs::read_dir(local_path).await.map_err(AppError::from)?;
            while let Some(e) = dir.next_entry().await.map_err(AppError::from)? {
                let child_local = e.path().to_string_lossy().replace('\\', "/");
                let child_remote = format!("{}/{}", remote_path.trim_end_matches('/'), e.file_name().to_string_lossy());
                Box::pin(self.upload_recursive(&child_local, &child_remote, progress.clone(), should_cancel.clone(), file_count)).await?;
            }
            return Ok(());
        }
        let total_len = tokio::fs::metadata(local_path).await.map(|m| m.len()).unwrap_or(0);
        let offset = match self.remote_size(remote_path).await {
            Ok(remote_len) if remote_len <= total_len => remote_len,
            _ => 0,
        };
        let event_path = local_path.to_string();
        let on_progress: Arc<dyn Fn(u64) + Send + Sync> = Arc::new(move |bytes| {
            if let Some((app, id)) = &progress { let _ = app.emit("sftp:transfer-progress", serde_json::json!({"requestId": id, "path": event_path, "bytes": bytes, "totalBytes": total_len})); }
        });
        self.upload_range_from_local(local_path, remote_path, offset, on_progress, should_cancel.clone()).await?;
        file_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }

}

/// 256KB——够大以避免每个 chunk 的 SFTP 往返开销主导传输速度，也够小以让取消/
/// 进度汇报的粒度对用户体感够用（几十 MB/s 下大约每几毫秒一个 chunk）。
const TRANSFER_CHUNK_BYTES: usize = 256 * 1024;

/// `download_range_to_local`/`upload_range_from_local` 共用的分块拷贝循环，
/// 泛型是因为下载和上传里"谁是 reader 谁是 writer"正好相反（远程 `File` 和
/// `tokio::fs::File` 都实现了 `AsyncRead`/`AsyncWrite`，各自套一次就行，不用重复
/// 写两遍循环体）。`on_progress` 收到的是"目前为止写入的总字节数"（含调用方传入的
/// `start_offset`），不是这次调用新写的字节数——续传场景下前端展示的是整体进度。
async fn copy_chunked<R, W>(
    mut reader: R,
    mut writer: W,
    start_offset: u64,
    on_progress: Arc<dyn Fn(u64) + Send + Sync>,
    should_cancel: Arc<dyn Fn() -> bool + Send + Sync>,
) -> Result<(), AppError>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut buf = vec![0u8; TRANSFER_CHUNK_BYTES];
    let mut total = start_offset;
    loop {
        if should_cancel() {
            return Err(AppError::Internal(super::TRANSFER_CANCELLED_MESSAGE.into()));
        }
        let n = reader.read(&mut buf).await.map_err(|e| AppError::Internal(e.to_string()))?;
        if n == 0 {
            break;
        }
        writer.write_all(&buf[..n]).await.map_err(|e| AppError::Internal(e.to_string()))?;
        total += n as u64;
        on_progress(total);
    }
    writer.flush().await.map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(())
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

    async fn file_size(&self, path: &str) -> Result<u64, AppError> {
        let path = path.to_string();
        self.with_sftp(move |sftp| {
            Box::pin(async move {
                let attrs = sftp
                    .metadata(&path)
                    .await
                    .map_err(|e| AppError::NotFound(format!("stat {path} failed: {e}")))?;
                Ok(attrs.size.unwrap_or(0))
            })
        })
        .await
    }

    async fn read_file_raw_bounded(&self, path: &str, max_bytes: u64) -> Result<(Vec<u8>, i64), AppError> {
        let path = path.to_string();
        self.with_sftp(move |sftp| {
            Box::pin(async move {
                let attrs = sftp
                    .metadata(&path)
                    .await
                    .map_err(|e| AppError::NotFound(format!("stat {path} failed: {e}")))?;
                let file = sftp
                    .open(&path)
                    .await
                    .map_err(|e| AppError::NotFound(format!("open {path} failed: {e}")))?;
                let mut buf = Vec::new();
                file.take(max_bytes)
                    .read_to_end(&mut buf)
                    .await
                    .map_err(|e| AppError::Internal(e.to_string()))?;
                Ok((buf, mtime_of(&attrs)))
            })
        })
        .await
    }

    async fn download_to_local_file(&self, path: &str, local_path: &str) -> Result<(), AppError> {
        // 复用已有的流式下载（SFTP 快捷工具批量传输那条路径），不走 trait 默认实现的
        // "整篇读进内存再写盘"，大文件也不会把内存吃满。
        self.download_to_local(path, local_path).await
    }

    /// 覆盖 trait 默认实现——默认版本是 `file_size()` 和 `read_file_raw()`/
    /// `read_file_raw_bounded()` 各自独立调用，对本地文件系统只是多两次系统调用，
    /// 但对 SFTP 意味着每次打开文件都要多一趟 stat 往返（`with_sftp` 各自加锁/解锁一次）。
    /// 2026-08-28 用户反馈"远程 SSH 工作区用着用着一些文本文件打开无内容"——大概率是
    /// 这个新增的额外 stat 往返把原本单次的 SFTP 交互变成两次，在这台服务器/连接上
    /// 触发了某种时序问题。这里合并回一次 `with_sftp`（一次 metadata + 一次 open/read），
    /// 和改动前的 `read_file_raw` 完全同构，从根上去掉这个新增的往返。
    async fn read_bytes_for_editor(&self, path: &str) -> Result<(Vec<u8>, i64, u64, bool), AppError> {
        let path = path.to_string();
        self.with_sftp(move |sftp| {
            Box::pin(async move {
                let attrs = sftp
                    .metadata(&path)
                    .await
                    .map_err(|e| AppError::NotFound(format!("stat {path} failed: {e}")))?;
                let total_size = attrs.size.unwrap_or(0);
                let truncated = total_size > super::EDITOR_PREVIEW_THRESHOLD_BYTES;

                let mut file = sftp
                    .open(&path)
                    .await
                    .map_err(|e| AppError::NotFound(format!("open {path} failed: {e}")))?;
                let mut buf = Vec::new();
                if truncated {
                    file.take(super::EDITOR_PREVIEW_MAX_BYTES)
                        .read_to_end(&mut buf)
                        .await
                        .map_err(|e| AppError::Internal(e.to_string()))?;
                } else {
                    file.read_to_end(&mut buf).await.map_err(|e| AppError::Internal(e.to_string()))?;
                }
                Ok((buf, mtime_of(&attrs), total_size, truncated))
            })
        })
        .await
    }

    /// 和 `read_bytes_for_editor` 同样的理由：合并成一次 `with_sftp`，不走 trait 默认的
    /// "先 file_size() 再 read_file_raw()" 两趟往返。
    async fn read_binary_for_preview(&self, path: &str, max_bytes: u64) -> Result<Vec<u8>, AppError> {
        let path = path.to_string();
        self.with_sftp(move |sftp| {
            Box::pin(async move {
                let attrs = sftp
                    .metadata(&path)
                    .await
                    .map_err(|e| AppError::NotFound(format!("stat {path} failed: {e}")))?;
                let total_size = attrs.size.unwrap_or(0);
                if total_size > max_bytes {
                    return Err(AppError::Internal(format!(
                        "文件过大（{:.1}MB），无法预览",
                        total_size as f64 / 1024.0 / 1024.0
                    )));
                }
                let mut file = sftp
                    .open(&path)
                    .await
                    .map_err(|e| AppError::NotFound(format!("open {path} failed: {e}")))?;
                let mut buf = Vec::new();
                file.read_to_end(&mut buf).await.map_err(|e| AppError::Internal(e.to_string()))?;
                Ok(buf)
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
