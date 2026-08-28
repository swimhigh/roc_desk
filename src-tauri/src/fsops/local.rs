use async_trait::async_trait;
use std::io::Read;
use std::time::UNIX_EPOCH;

use super::encoding::decode_text;
use super::{FileEntry, FileOps, WriteOutcome};
use crate::error::AppError;

pub struct LocalFileOps;

fn mtime_secs(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[async_trait]
impl FileOps for LocalFileOps {
    async fn read_file_raw(&self, path: &str) -> Result<(Vec<u8>, i64), AppError> {
        let path = path.to_string();
        tokio::task::spawn_blocking(move || {
            let meta = std::fs::metadata(&path)?;
            let bytes = std::fs::read(&path)?;
            Ok((bytes, mtime_secs(&meta)))
        })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
    }

    async fn file_size(&self, path: &str) -> Result<u64, AppError> {
        let path = path.to_string();
        tokio::task::spawn_blocking(move || Ok(std::fs::metadata(&path)?.len()))
            .await
            .map_err(|e| AppError::Internal(e.to_string()))?
    }

    async fn read_file_raw_bounded(&self, path: &str, max_bytes: u64) -> Result<(Vec<u8>, i64), AppError> {
        let path = path.to_string();
        tokio::task::spawn_blocking(move || {
            let meta = std::fs::metadata(&path)?;
            let file = std::fs::File::open(&path)?;
            let mut buf = Vec::with_capacity(max_bytes.min(meta.len()) as usize);
            file.take(max_bytes).read_to_end(&mut buf)?;
            Ok((buf, mtime_secs(&meta)))
        })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
    }

    async fn download_to_local_file(&self, path: &str, local_path: &str) -> Result<(), AppError> {
        let path = path.to_string();
        let local_path = local_path.to_string();
        tokio::task::spawn_blocking(move || {
            std::fs::copy(&path, &local_path)?;
            Ok(())
        })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
    }

    /// 覆盖 trait 默认实现，合并成一次 `spawn_blocking`（默认版本是 `file_size()` 和
    /// `read_file_raw()`/`read_file_raw_bounded()` 各自单独起一个 blocking 任务，
    /// 本地场景下纯粹是多余的线程池调度开销，和 `RemoteFileOps` 那边为了修
    /// "SFTP 多一趟往返" 的理由一致，顺手也把本地这边合并了）。
    async fn read_bytes_for_editor(&self, path: &str) -> Result<(Vec<u8>, i64, u64, bool), AppError> {
        let path = path.to_string();
        tokio::task::spawn_blocking(move || {
            let meta = std::fs::metadata(&path)?;
            let total_size = meta.len();
            let truncated = total_size > super::EDITOR_PREVIEW_THRESHOLD_BYTES;
            if truncated {
                let file = std::fs::File::open(&path)?;
                let mut buf = Vec::with_capacity(super::EDITOR_PREVIEW_MAX_BYTES.min(total_size) as usize);
                file.take(super::EDITOR_PREVIEW_MAX_BYTES).read_to_end(&mut buf)?;
                Ok((buf, mtime_secs(&meta), total_size, true))
            } else {
                let bytes = std::fs::read(&path)?;
                Ok((bytes, mtime_secs(&meta), total_size, false))
            }
        })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
    }

    /// 和 `read_bytes_for_editor` 同样的理由，合并成一次 `spawn_blocking`。
    async fn read_binary_for_preview(&self, path: &str, max_bytes: u64) -> Result<Vec<u8>, AppError> {
        let path = path.to_string();
        tokio::task::spawn_blocking(move || {
            let meta = std::fs::metadata(&path)?;
            if meta.len() > max_bytes {
                return Err(AppError::Internal(format!("文件过大（{:.1}MB），无法预览", meta.len() as f64 / 1024.0 / 1024.0)));
            }
            let bytes = std::fs::read(&path)?;
            Ok(bytes)
        })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
    }

    async fn write_file_bytes(
        &self,
        path: &str,
        bytes: &[u8],
        expected_mtime: Option<i64>,
    ) -> Result<WriteOutcome, AppError> {
        let path = path.to_string();
        let bytes = bytes.to_vec();
        tokio::task::spawn_blocking(move || {
            // 保存前冲突检测（DESIGN.md §3.1.4）：本地文件也可能被外部程序改过，
            // 与远程实现保持同样的检查逻辑。
            if let Some(expected) = expected_mtime {
                if let Ok(meta) = std::fs::metadata(&path) {
                    let current = mtime_secs(&meta);
                    if current != expected {
                        let preview = std::fs::read(&path)
                            .map(|b| decode_text(&b))
                            .unwrap_or_default()
                            .lines()
                            .take(5)
                            .collect::<Vec<_>>()
                            .join("\n");
                        return Ok(WriteOutcome::Conflict {
                            current_mtime: current,
                            current_preview: preview,
                        });
                    }
                }
            }

            std::fs::write(&path, &bytes)?;
            let meta = std::fs::metadata(&path)?;
            Ok(WriteOutcome::Written {
                mtime: mtime_secs(&meta),
            })
        })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
    }

    async fn list_dir(&self, path: &str) -> Result<Vec<FileEntry>, AppError> {
        let path = path.to_string();
        tokio::task::spawn_blocking(move || {
            let mut entries = Vec::new();
            for entry in std::fs::read_dir(&path)? {
                let entry = entry?;
                let meta = entry.metadata()?;
                let full_path = entry.path();
                entries.push(FileEntry {
                    name: entry.file_name().to_string_lossy().to_string(),
                    path: full_path.to_string_lossy().replace('\\', "/"),
                    is_dir: meta.is_dir(),
                    size: if meta.is_dir() { None } else { Some(meta.len()) },
                    modified: Some(mtime_secs(&meta)),
                });
            }
            // 目录优先，同类型按名称排序 — 与 Explorer 树的显示顺序保持一致（UI_DESIGN.md §3.3）
            entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            });
            Ok(entries)
        })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
    }

    async fn delete(&self, path: &str, is_dir: bool) -> Result<(), AppError> {
        let path = path.to_string();
        tokio::task::spawn_blocking(move || {
            if is_dir {
                std::fs::remove_dir_all(&path)?;
            } else {
                std::fs::remove_file(&path)?;
            }
            Ok(())
        })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
    }

    async fn rename(&self, from: &str, to: &str) -> Result<(), AppError> {
        let from = from.to_string();
        let to = to.to_string();
        tokio::task::spawn_blocking(move || {
            std::fs::rename(&from, &to)?;
            Ok(())
        })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
    }

    async fn create_dir(&self, path: &str) -> Result<(), AppError> {
        let path = path.to_string();
        tokio::task::spawn_blocking(move || {
            std::fs::create_dir_all(&path)?;
            Ok(())
        })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 没有 `#[tokio::test]`（Cargo.toml 里 tokio 没开 "macros"/"test-util" feature，
    /// 这两个方法本身用到的 `spawn_blocking` 只需要 "rt-multi-thread"，已经开着），
    /// 手动起一个 runtime 跑 `block_on` 就够了，不用为一个测试多引一个 feature。
    fn block_on<F: std::future::Future>(fut: F) -> F::Output {
        tokio::runtime::Runtime::new().unwrap().block_on(fut)
    }

    /// 验证 `read_file_raw_bounded` 真的"只读前 N 字节"——不是读完整个文件再截断，
    /// 这是 2026-08-28 大文件保护的核心断言：如果这里退化成读全部再切片，大文件卡死
    /// 的问题就没修。用文件大小是否等于 max_bytes（而不是等于源文件大小）来验证。
    #[test]
    fn read_file_raw_bounded_stops_at_max_bytes() {
        let dir = std::env::temp_dir().join(format!("roc_desk_test_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("large.txt");
        let content = "0123456789".repeat(1000); // 10_000 bytes
        std::fs::write(&path, &content).unwrap();
        let path_str = path.to_string_lossy().replace('\\', "/");

        let ops = LocalFileOps;
        let total = block_on(ops.file_size(&path_str)).unwrap();
        assert_eq!(total, 10_000);

        let (bytes, _mtime) = block_on(ops.read_file_raw_bounded(&path_str, 100)).unwrap();
        assert_eq!(bytes.len(), 100);
        assert_eq!(&bytes, &content.as_bytes()[..100]);

        // max_bytes 超过文件实际大小时不应该越界/报错，就正常返回全部内容。
        let (bytes, _mtime) = block_on(ops.read_file_raw_bounded(&path_str, 1_000_000)).unwrap();
        assert_eq!(bytes.len(), 10_000);

        std::fs::remove_dir_all(&dir).ok();
    }
}
