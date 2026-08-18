pub mod encoding;
pub mod local;
pub mod remote;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::AppError;
use encoding::decode_text_detect;

/// 本地/远程统一文件操作接口（CODE_DESIGN.md §3.5，DESIGN.md §3.1.4）。
///
/// 独立于 `coding`/`workspace` 之外的顶层模块：Explorer、SFTP 快捷工具、
/// AI 编程助手三处都要读写本地/远程文件，放在任何一处下面都会造成另外两处
/// 反向依赖它，因此单独成模块，三者都只依赖这个 trait。
///
/// `read_file`/`write_file` 是给"不关心编码"的调用方（AI 编程助手、SFTP 快捷工具）
/// 用的默认实现——按 UTF-8/GBK/UTF-16 自动探测读，固定 UTF-8 写；真正的读写字节
/// 只需要各实现各写一份 `read_file_raw`/`write_file_bytes`。编辑器的
/// "Reopen/Save with Encoding"（参考 VS Code）走 `commands/fs.rs` 里单独的
/// 强制编码命令，直接调用 raw 方法配合 `fsops::encoding` 转换，不占用 trait 的
/// 默认路径。
#[async_trait]
pub trait FileOps: Send + Sync {
    async fn read_file(&self, path: &str) -> Result<FileContent, AppError> {
        let (bytes, mtime) = self.read_file_raw(path).await?;
        let (text, encoding) = decode_text_detect(&bytes);
        Ok(FileContent { text, encoding: encoding.to_string(), mtime })
    }

    /// `expected_mtime` 为空表示不做冲突检测（例如新建文件）；非空时若远程/本地
    /// 当前 mtime 与之不一致，返回 `WriteOutcome::Conflict` 而不是直接覆盖
    /// （DESIGN.md §3.1.4 远程文件编辑冲突检测）。默认实现固定写 UTF-8。
    async fn write_file(
        &self,
        path: &str,
        content: &str,
        expected_mtime: Option<i64>,
    ) -> Result<WriteOutcome, AppError> {
        self.write_file_bytes(path, content.as_bytes(), expected_mtime).await
    }

    async fn list_dir(&self, path: &str) -> Result<Vec<FileEntry>, AppError>;

    /// 返回文件的原始字节 + mtime，不做任何编码假设。
    async fn read_file_raw(&self, path: &str) -> Result<(Vec<u8>, i64), AppError>;

    /// 和 `write_file` 语义一致（含冲突检测），只是接受任意字节而不是假定 UTF-8 字符串。
    async fn write_file_bytes(
        &self,
        path: &str,
        bytes: &[u8],
        expected_mtime: Option<i64>,
    ) -> Result<WriteOutcome, AppError>;

    /// Explorer 右键菜单的删除（DESIGN.md §3.1.2，参考 VS Code）。`is_dir` 由调用方
    /// （已经有 FileEntry）直接传入，避免再做一次 stat。远程非空目录暂不支持递归删除——
    /// SFTP 的 RMDIR 只能删空目录，真正的递归删除需要遍历整棵树逐个删，超出这里的范围，
    /// 遇到时明确报错而不是静默失败或半删一半。
    async fn delete(&self, path: &str, is_dir: bool) -> Result<(), AppError>;

    async fn rename(&self, from: &str, to: &str) -> Result<(), AppError>;

    /// 建目录（含中间目录，本地语义上等价 `mkdir -p`）；远程侧只建单层，中间目录
    /// 不存在时会失败——目前所有调用方（`copy` 递归、编程助手写文件）传的都是
    /// "父目录已经存在"的路径，用不到远程 `mkdir -p` 那种多层递归建目录。
    async fn create_dir(&self, path: &str) -> Result<(), AppError>;

    /// Explorer 右键"复制"（参考 VS Code）。文件直接读字节写字节；目录先建好目标
    /// 目录，再逐个子项递归——这是 trait 默认实现，本地/远程共用同一套递归逻辑，
    /// 各自只需要提供 `list_dir`/`create_dir`/`read_file_raw`/`write_file_bytes`
    /// 这几个更基础的原语。`self.copy(...)` 的递归调用之所以不会无限尺寸，是因为
    /// `#[async_trait]` 已经把每个方法的返回类型擦除成 `Pin<Box<dyn Future>>`了。
    async fn copy(&self, from: &str, to: &str, is_dir: bool) -> Result<(), AppError> {
        if !is_dir {
            let (bytes, _) = self.read_file_raw(from).await?;
            self.write_file_bytes(to, &bytes, None).await?;
            return Ok(());
        }
        self.create_dir(to).await?;
        let entries = self.list_dir(from).await?;
        for entry in entries {
            let child_to = format!("{}/{}", to.trim_end_matches('/'), entry.name);
            self.copy(&entry.path, &child_to, entry.is_dir).await?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileContent {
    pub text: String,
    /// 探测到（或调用方强制指定）的编码标签，供编辑器状态栏展示
    /// （参考 VS Code 右下角的编码指示器）。
    pub encoding: String,
    /// Unix 时间戳（秒），供保存时做冲突检测
    pub mtime: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum WriteOutcome {
    Written { mtime: i64 },
    Conflict { current_mtime: i64, current_preview: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: Option<u64>,
    pub modified: Option<i64>,
}
