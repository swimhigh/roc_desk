use std::io::{BufRead, BufReader};
use std::sync::Arc;

use uuid::Uuid;

use super::engine::LogSearchEngine;
use crate::error::AppError;
use crate::fsops::remote::RemoteFileOps;

/// 日志文件 → SQLite 增量导入（DESIGN.md §3.4.2）。
/// 远程文件先流式下载到本地缓存目录，再逐行导入——不会把整个文件内容读进
/// 一个 Rust `String`（那是 `fsops::FileOps::read_file` 的路径，对大日志文件不合适）。
pub struct LogImporter {
    engine: Arc<LogSearchEngine>,
    cache_dir: std::path::PathBuf,
}

impl LogImporter {
    pub fn new(engine: Arc<LogSearchEngine>, cache_dir: std::path::PathBuf) -> Self {
        Self { engine, cache_dir }
    }

    pub fn import_local_file(&self, path: &str, host_name: &str) -> Result<usize, AppError> {
        let file = std::fs::File::open(path)?;
        let reader = BufReader::new(file);
        let lines: Vec<String> = reader.lines().collect::<Result<_, _>>()?;
        self.engine
            .import_lines(path, host_name, lines.iter().map(|s| s.as_str()))
    }

    /// 远程日志获取：先经 SFTP 下载到本地缓存，再走本地导入路径
    /// （DESIGN.md §3.4.1"远程日志获取"）。
    pub async fn import_remote_file(
        &self,
        file_ops: &RemoteFileOps,
        remote_path: &str,
        host_name: &str,
    ) -> Result<usize, AppError> {
        std::fs::create_dir_all(&self.cache_dir)?;
        let local_path = self.cache_dir.join(Uuid::new_v4().to_string());
        file_ops
            .download_to_local(remote_path, local_path.to_str().unwrap())
            .await?;

        let count = self.import_local_file(local_path.to_str().unwrap(), host_name)?;

        // 索引已经建好，本地缓存副本没有继续存在的必要（DESIGN.md §十-3 磁盘配额）。
        let _ = std::fs::remove_file(&local_path);
        Ok(count)
    }
}
