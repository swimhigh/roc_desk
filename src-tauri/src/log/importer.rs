use std::sync::Arc;

use serde::Serialize;
use uuid::Uuid;

use super::engine::LogSearchEngine;
use crate::error::AppError;
use crate::fsops::encoding::decode_text_detect;
use roc_desk_ssh::fsops::remote::RemoteFileOps;
use roc_desk_ssh::fsops::FileOps;

/// 一批路径导入完成后的汇总——单个文件失败（权限/编码之外的意外错误）不该让
/// 整批操作直接报错中断，跳过继续导入其它文件，把失败明细带回去，前端汇总
/// 成"已导入 N 行，M 个文件失败"，而不是"哪个文件坏了就全部前功尽弃"。
#[derive(Debug, Clone, Serialize)]
pub struct LogImportOutcome {
    pub lines_imported: usize,
    pub files_imported: usize,
    pub failed: Vec<LogImportFailure>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LogImportFailure {
    pub path: String,
    pub error: String,
}

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

    /// 按 UTF-8/GBK/UTF-16 自动探测解码（和编辑器读文件用的是同一套
    /// `fsops::encoding::decode_text_detect`），不是原来 `BufRead::lines()` 那样
    /// 强制假定 UTF-8——非 UTF-8 编码的日志文件（GBK 是国产运维环境里最常见的
    /// 情况）原来会直接报"stream did not contain valid UTF-8"，一整份都导不进去
    /// （2026-09 用户反馈）。
    fn import_local_file_decoded(&self, path: &str, host_name: &str) -> Result<usize, AppError> {
        let bytes = std::fs::read(path)?;
        let (text, _encoding) = decode_text_detect(&bytes);
        self.engine.import_lines(path, host_name, text.lines())
    }

    /// 单文件导入——保留原有签名给内部单文件场景复用，实现已经切到上面的解码
    /// 版本。
    pub fn import_local_file(&self, path: &str, host_name: &str) -> Result<usize, AppError> {
        self.import_local_file_decoded(path, host_name)
    }

    /// 展开一批本地路径为具体文件列表：`recursive` 为真时目录会被递归展开（跳过
    /// 以 `.` 开头的隐藏文件/目录，避免把 `.git`/`.rock_desk` 这类仓库元数据当日志
    /// 导进索引）；为假时传入目录直接报错——不知道用户是想导入目录下哪个具体文件，
    /// 静默展开可能导入一堆意料之外的内容。用显式栈做迭代遍历，不用递归函数
    /// （避免大目录树深度不可控时的调用栈风险，也不需要为递归 async fn 做
    /// `Box::pin`）。
    fn expand_local_paths(
        paths: &[String],
        recursive: bool,
    ) -> Result<Vec<std::path::PathBuf>, AppError> {
        let mut queue: Vec<std::path::PathBuf> =
            paths.iter().map(std::path::PathBuf::from).collect();
        let mut files = Vec::new();
        while let Some(p) = queue.pop() {
            let meta = std::fs::metadata(&p)?;
            if meta.is_dir() {
                if !recursive {
                    return Err(AppError::Internal(format!(
                        "{} 是一个目录，请勾选“按目录递归导入”",
                        p.display()
                    )));
                }
                for entry in std::fs::read_dir(&p)? {
                    let entry = entry?;
                    if entry.file_name().to_string_lossy().starts_with('.') {
                        continue;
                    }
                    queue.push(entry.path());
                }
            } else {
                files.push(p);
            }
        }
        files.sort();
        Ok(files)
    }

    /// 批量导入本地路径（文件和/或目录混在一起）——`progress` 在每个文件真正开始
    /// 导入前调用一次 `(path, 已完成数, 总数)`，供调用方（Tauri command）转发成
    /// 前端事件，展示"正在导入第 x/y 个文件"而不是导入完才一次性弹结果
    /// （2026-09 用户反馈：想要支持选多个文件/整个目录导入，体验要更好）。
    pub fn import_local_paths(
        &self,
        paths: &[String],
        host_name: &str,
        recursive: bool,
        mut progress: impl FnMut(&str, usize, usize),
    ) -> Result<LogImportOutcome, AppError> {
        let files = Self::expand_local_paths(paths, recursive)?;
        let total = files.len();
        let mut lines_imported = 0usize;
        let mut files_imported = 0usize;
        let mut failed = Vec::new();
        for (i, file) in files.iter().enumerate() {
            let path_str = file.to_string_lossy().to_string();
            progress(&path_str, i, total);
            match self.import_local_file_decoded(&path_str, host_name) {
                Ok(n) => {
                    lines_imported += n;
                    files_imported += 1;
                }
                Err(e) => failed.push(LogImportFailure {
                    path: path_str,
                    error: e.to_string(),
                }),
            }
        }
        Ok(LogImportOutcome {
            lines_imported,
            files_imported,
            failed,
        })
    }

    /// 远程侧展开目录——用 `list_dir` 探测每个路径是不是目录：能列出内容就当目录
    /// （递归关闭时报错，和本地展开语义一致），列不出来（`NotADirectory`/权限等）
    /// 就当成一个文件路径直接收进结果，真正的错误留到后面下载这一步再暴露，
    /// 报错信息更precise（"下载失败"而不是"探测目录失败"）。SFTP 没有单独的
    /// "只 stat 不列目录"接口给这里更省事地用，这个探测方式足够。
    async fn expand_remote_paths(
        file_ops: &RemoteFileOps,
        paths: &[String],
        recursive: bool,
    ) -> Result<Vec<String>, AppError> {
        let mut queue: Vec<String> = paths.to_vec();
        let mut files = Vec::new();
        while let Some(p) = queue.pop() {
            match file_ops.list_dir(&p).await {
                Ok(entries) => {
                    if !recursive {
                        return Err(AppError::Internal(format!(
                            "{p} 是一个目录，请勾选“按目录递归导入”"
                        )));
                    }
                    for entry in entries {
                        if entry.name.starts_with('.') {
                            continue;
                        }
                        if entry.is_dir {
                            queue.push(entry.path);
                        } else {
                            files.push(entry.path);
                        }
                    }
                }
                Err(_) => files.push(p),
            }
        }
        files.sort();
        Ok(files)
    }

    async fn download_and_import_remote(
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
        let result = self.import_local_file_decoded(local_path.to_str().unwrap(), host_name);
        // 索引已经建好，本地缓存副本没有继续存在的必要（DESIGN.md §十-3 磁盘配额）；
        // 导入失败时也要清掉，不留垃圾文件。
        let _ = std::fs::remove_file(&local_path);
        result
    }

    /// 远程日志获取：先经 SFTP 下载到本地缓存，再走本地导入路径
    /// （DESIGN.md §3.4.1"远程日志获取"）。
    pub async fn import_remote_file(
        &self,
        file_ops: &RemoteFileOps,
        remote_path: &str,
        host_name: &str,
    ) -> Result<usize, AppError> {
        self.download_and_import_remote(file_ops, remote_path, host_name)
            .await
    }

    /// 批量导入远程路径（文件和/或目录混在一起），语义和 `import_local_paths`
    /// 对称：先展开出完整文件列表拿到准确的总数，再逐个下载导入，单个文件失败
    /// 不中断整批。
    pub async fn import_remote_paths(
        &self,
        file_ops: &RemoteFileOps,
        paths: &[String],
        host_name: &str,
        recursive: bool,
        mut progress: impl FnMut(&str, usize, usize),
    ) -> Result<LogImportOutcome, AppError> {
        let files = Self::expand_remote_paths(file_ops, paths, recursive).await?;
        let total = files.len();
        let mut lines_imported = 0usize;
        let mut files_imported = 0usize;
        let mut failed = Vec::new();
        for (i, remote_path) in files.iter().enumerate() {
            progress(remote_path, i, total);
            match self
                .download_and_import_remote(file_ops, remote_path, host_name)
                .await
            {
                Ok(n) => {
                    lines_imported += n;
                    files_imported += 1;
                }
                Err(e) => failed.push(LogImportFailure {
                    path: remote_path.clone(),
                    error: e.to_string(),
                }),
            }
        }
        Ok(LogImportOutcome {
            lines_imported,
            files_imported,
            failed,
        })
    }
}
