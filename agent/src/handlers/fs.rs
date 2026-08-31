//! 文件系统原语（AGENT_DESIGN.md §2.1）：直接用 Agent 进程本机的 `std::fs`/
//! `tokio::fs`——这是 Agent 方案相对 SFTP 的核心优势之一，不需要另外的协议
//! 语义转换层，Windows 路径（含盘符）原生传输，不假设 POSIX 语义。

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use roc_desk_protocol::{ErrorCode, FileEntry};

pub fn mtime_of(modified: std::io::Result<SystemTime>) -> i64 {
    modified
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn err_pair(e: std::io::Error) -> (ErrorCode, String) {
    let code = match e.kind() {
        std::io::ErrorKind::NotFound => ErrorCode::NotFound,
        std::io::ErrorKind::PermissionDenied => ErrorCode::PermissionDenied,
        _ => ErrorCode::Internal,
    };
    (code, e.to_string())
}

pub async fn list_dir(path: &Path) -> Result<Vec<FileEntry>, (ErrorCode, String)> {
    let mut read_dir = tokio::fs::read_dir(path).await.map_err(err_pair)?;
    let mut entries = Vec::new();
    while let Some(entry) = read_dir.next_entry().await.map_err(err_pair)? {
        let Ok(metadata) = entry.metadata().await else { continue };
        let name = entry.file_name().to_string_lossy().to_string();
        let full_path = entry.path().to_string_lossy().to_string();
        entries.push(FileEntry {
            is_dir: metadata.is_dir(),
            size: if metadata.is_dir() { None } else { Some(metadata.len()) },
            modified: Some(mtime_of(metadata.modified())),
            name,
            path: full_path,
        });
    }
    entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });
    Ok(entries)
}

/// Windows 盘符列表（Explorer 树的"根"概念）：逐个尝试 `A:\`..`Z:\` 是否存在——
/// 不引入 `windows` crate 的 `GetLogicalDrives` API，一个简单的存在性探测足够，
/// 且天然跨盘符类型（本地盘/映射网络盘/可移动介质都能探测到）。
pub async fn list_roots() -> Vec<String> {
    let mut roots = Vec::new();
    for letter in b'A'..=b'Z' {
        let root = format!("{}:\\", letter as char);
        if tokio::fs::metadata(&root).await.is_ok() {
            roots.push(root);
        }
    }
    roots
}

pub async fn stat(path: &Path) -> Result<(u64, i64, bool), (ErrorCode, String)> {
    let metadata = tokio::fs::metadata(path).await.map_err(err_pair)?;
    Ok((metadata.len(), mtime_of(metadata.modified()), metadata.is_dir()))
}

pub async fn read_file(path: &Path, max_bytes: Option<u64>) -> Result<(Vec<u8>, i64), (ErrorCode, String)> {
    let metadata = tokio::fs::metadata(path).await.map_err(err_pair)?;
    let mtime = mtime_of(metadata.modified());
    let bytes = match max_bytes {
        Some(limit) => {
            use tokio::io::AsyncReadExt;
            let file = tokio::fs::File::open(path).await.map_err(err_pair)?;
            let mut buf = Vec::new();
            file.take(limit).read_to_end(&mut buf).await.map_err(|e| (ErrorCode::Internal, e.to_string()))?;
            buf
        }
        None => tokio::fs::read(path).await.map_err(err_pair)?,
    };
    Ok((bytes, mtime))
}

/// 返回 `Ok(Some(mtime))` 表示写入成功；`Ok(None)` 表示 mtime 冲突（调用方负责
/// 组装 `Response::Conflict`，这里只做纯粹的读写逻辑）。
pub async fn write_file(path: &Path, bytes: &[u8], expected_mtime: Option<i64>) -> Result<Option<(i64, i64, Vec<u8>)>, (ErrorCode, String)> {
    if let Some(expected) = expected_mtime {
        if let Ok(metadata) = tokio::fs::metadata(path).await {
            let current = mtime_of(metadata.modified());
            if current != expected {
                let preview = tokio::fs::read(path).await.unwrap_or_default();
                let preview: Vec<u8> = preview.into_iter().take(4096).collect();
                return Ok(Some((current, expected, preview)));
            }
        }
    }
    if let Some(parent) = path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    tokio::fs::write(path, bytes).await.map_err(err_pair)?;
    Ok(None)
}

pub async fn delete(path: &Path, is_dir: bool) -> Result<(), (ErrorCode, String)> {
    if is_dir {
        tokio::fs::remove_dir_all(path).await.map_err(err_pair)
    } else {
        tokio::fs::remove_file(path).await.map_err(err_pair)
    }
}

pub async fn rename(from: &Path, to: &Path) -> Result<(), (ErrorCode, String)> {
    tokio::fs::rename(from, to).await.map_err(err_pair)
}

pub async fn create_dir(path: &Path) -> Result<(), (ErrorCode, String)> {
    tokio::fs::create_dir_all(path).await.map_err(err_pair)
}

