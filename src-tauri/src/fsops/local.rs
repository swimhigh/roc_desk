use async_trait::async_trait;
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
