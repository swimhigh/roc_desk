pub mod agent;
pub mod binary_info;
pub mod encoding;
pub mod jar_info;
pub mod local;
pub mod office_convert;
pub mod remote;

use async_trait::async_trait;
use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

use crate::error::AppError;
use encoding::decode_text_detect;

pub use binary_info::{BinaryInfo, SectionInfo};
pub use jar_info::{JarEntryInfo, JarInfo, ManifestAttribute};

/// `copy_between`/`download_recursive`/`upload_recursive` 在 `should_cancel()`
/// 命中时统一返回的错误信息——用一个共享常量而不是各处各写一份字面量字符串，
/// 是因为命令层（`commands::sftp`/`commands::agent`）要靠这个消息字符串区分
/// "用户主动停止" 和 "真的传输失败"，写进 `transfer_log` 的 `status` 字段
/// （'cancelled' vs 'failed'），两边对不上这条消息就会误判成失败。
pub const TRANSFER_CANCELLED_MESSAGE: &str = "传输已取消";

/// 全文搜索/替换单个目录内跳过的噪音目录名（构建产物/依赖/VCS 元数据），
/// 和常见二进制文件扩展名——不进这些目录、不读这些扩展名的文件，避免把
/// node_modules 里几万个文件也扫一遍，或者把图片/压缩包当文本读出乱码。
const SEARCH_EXCLUDED_DIRS: &[&str] = &[
    ".git", "node_modules", "target", "dist", "build", ".next", "__pycache__", ".venv", ".cargo",
];
const SEARCH_BINARY_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "ico", "webp", "bmp", "mp4", "mp3", "wav", "avi", "mov", "zip",
    "tar", "gz", "7z", "rar", "exe", "dll", "pdb", "so", "dylib", "pdf", "woff", "woff2", "ttf",
    "eot", "db", "sqlite", "class", "jar", "wasm",
];
const SEARCH_MAX_FILES: usize = 500;
const SEARCH_MAX_MATCHES: usize = 3000;
const SEARCH_MAX_MATCHES_PER_FILE: usize = 50;
/// 单文件超过这个大小就跳过，不读进内存做正则——避免误扫到大体积日志/数据文件。
const SEARCH_MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

/// 编辑器打开文件超过这个大小就不整篇读入内存，改走只读预览（2026-08-28 用户反馈：
/// >1GB 的文本文件在编辑器里打开会卡死——根因是 `read_file`/`read_file_raw` 默认实现
/// 无论文件多大都 `read_to_string`/`read_to_end` 整篇吸进内存，Monaco 再存一份，
/// 前端拿到超大字符串卡死渲染。DESIGN.md/UI_DESIGN.md 里早就设想过大文件分档处理，
/// 但一直没有落地，这里补上）。
const EDITOR_PREVIEW_THRESHOLD_BYTES: u64 = 10 * 1024 * 1024;
/// 超过阈值时只读这么多字节做预览——够看清文件内容，又不会因为单个巨大文件把
/// 内存/IPC 占满（尤其是远程 SFTP 场景，这些字节还要整个通过 IPC 序列化传给前端）。
const EDITOR_PREVIEW_MAX_BYTES: u64 = 2 * 1024 * 1024;

/// 二进制预览（图片/PDF/Word/Excel）最大字节数：这些格式不能像文本那样"只读一半"
/// （截断的 PNG/PDF/docx 解不出内容，不如不读），所以是"超过就直接拒绝"而不是
/// "超过就截断"。30MB 覆盖绝大多数截图/文档场景，真遇到更大的文件用户应该用
/// "系统默认程序打开"而不是编辑器里的预览（2026-08-28 用户反馈图片/Word/Excel/PDF
/// 在编辑器里被当文本打开显示乱码）。`pub` 是因为 `commands/fs.rs`/`commands/sftp.rs`
/// 的预览命令也要用同一个上限。
pub const BINARY_PREVIEW_MAX_BYTES: u64 = 30 * 1024 * 1024;

/// EXE/DLL/SO/JAR 解析（`fsops::binary_info`/`fsops::jar_info`）用单独的、大得多的
/// 上限，不能复用 `BINARY_PREVIEW_MAX_BYTES`（2026-08-29 用户反馈：Linux 下一个
/// 62.5MB 的可执行文件报"文件过大，无法预览"，之前两者共用 30MB 这个为图片/PDF/
/// Word/Excel 预览设计的上限）——真实世界的可执行文件/JAR 包（尤其是 Go/Rust 静态
/// 链接产物、fat jar）动辄几十到上百 MB，比典型图片/文档大一个数量级，而且解析
/// 依赖完整字节（ELF 的节区表、动态符号表可能在文件任意偏移，goblin/zip 都需要
/// 能随机访问整个文件，不能像文本预览那样只读开头一截）。200MB 覆盖绝大多数真实
/// 场景，真遇到更大的直接引导去"用系统程序打开"。
pub const EXECUTABLE_INSPECT_MAX_BYTES: u64 = 200 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchOptions {
    #[serde(default)]
    pub case_sensitive: bool,
    #[serde(default)]
    pub whole_word: bool,
    #[serde(default)]
    pub use_regex: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchMatch {
    /// 1-based 行号（参考 VS Code 搜索结果的行号展示）。
    pub line_number: usize,
    pub line_text: String,
    /// 字符下标（不是字节下标），供前端直接 `Array.from(line).slice(start, end)` 高亮。
    pub match_start: usize,
    pub match_end: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchFileResult {
    pub path: String,
    pub matches: Vec<SearchMatch>,
}

/// 搜索文件名（只比对文件名，不读文件内容——目录树递归+字符串匹配，没有任何
/// I/O 读文件内容的开销）还是搜索文件内容（当前实现，逐文件读进来跑正则）。
/// 2026-08-18 用户反馈之一是"搜文件OR目录名还是搜索文本需要有个选项"——之前
/// 只有内容搜索一种模式，用户想按文件名找文件（比如这次报告问题时搜的
/// "kgms.xml" 本身就是个文件名）却被迫走内容搜索这条慢路径。
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchMode {
    Content,
    FileName,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplaceSummary {
    pub files_changed: usize,
    pub occurrences_replaced: usize,
}

fn build_matcher(query: &str, options: &SearchOptions) -> Result<Regex, AppError> {
    if query.is_empty() {
        return Err(AppError::Internal("搜索内容不能为空".into()));
    }
    let base = if options.use_regex { query.to_string() } else { regex::escape(query) };
    let pattern = if options.whole_word { format!(r"\b{base}\b") } else { base };
    RegexBuilder::new(&pattern)
        .case_insensitive(!options.case_sensitive)
        .build()
        .map_err(|e| AppError::Internal(format!("正则表达式无效：{e}")))
}

fn is_excluded_dir(name: &str) -> bool {
    SEARCH_EXCLUDED_DIRS.contains(&name)
}

fn is_binary_extension(name: &str) -> bool {
    match name.rsplit_once('.') {
        Some((_, ext)) => SEARCH_BINARY_EXTENSIONS.contains(&ext.to_lowercase().as_str()),
        None => false,
    }
}

/// 前 8KB 里出现 NUL 字节就当成二进制——常见的"这文件是不是文本"启发式判断
/// （git、ripgrep 等工具都用同一个思路），不追求 100% 准确，够用即可。
fn looks_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8192).any(|b| *b == 0)
}

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
        let total_size = bytes.len() as u64;
        let (text, encoding) = decode_text_detect(&bytes);
        Ok(FileContent { text, encoding: encoding.to_string(), mtime, total_size, truncated: false })
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

    /// 只 stat 拿文件总字节数，不读内容——编辑器打开文件前用它判断要不要走
    /// 只读预览分支，判断本身不能先把文件读一遍，否则大文件还是会卡在这一步。
    async fn file_size(&self, path: &str) -> Result<u64, AppError>;

    /// 和 `read_file_raw` 一样返回原始字节 + mtime，但最多只读 `max_bytes` 字节——
    /// 本地实现用 `Read::take`，远程实现用 SFTP 文件句柄的 `AsyncReadExt::take`，
    /// 都不会像 `read_file_raw` 那样把整个文件一次性吸进内存。
    async fn read_file_raw_bounded(&self, path: &str, max_bytes: u64) -> Result<(Vec<u8>, i64), AppError>;

    /// 给编辑器用的"体积感知"读取：超过 `EDITOR_PREVIEW_THRESHOLD_BYTES` 就只读前
    /// `EDITOR_PREVIEW_MAX_BYTES` 字节，`truncated` 告诉调用方这份内容是被截断的
    /// 预览，不能整篇编辑保存（保存会把截断内容覆盖回真实的大文件，等于删除数据）。
    async fn read_bytes_for_editor(&self, path: &str) -> Result<(Vec<u8>, i64, u64, bool), AppError> {
        let total_size = self.file_size(path).await?;
        if total_size > EDITOR_PREVIEW_THRESHOLD_BYTES {
            let (bytes, mtime) = self.read_file_raw_bounded(path, EDITOR_PREVIEW_MAX_BYTES).await?;
            Ok((bytes, mtime, total_size, true))
        } else {
            let (bytes, mtime) = self.read_file_raw(path).await?;
            Ok((bytes, mtime, total_size, false))
        }
    }

    /// `read_bytes_for_editor` 的字符串版本，直接产出编辑器/`fs_read_file` 命令要的
    /// `FileContent`，编码探测只在实际读到的那部分字节上跑（截断预览时同理，探测的是
    /// 预览片段的编码，不代表一定是整个文件的编码——这是"能看个大概"和"卡死"之间的取舍）。
    async fn read_file_for_editor(&self, path: &str) -> Result<FileContent, AppError> {
        let (bytes, mtime, total_size, truncated) = self.read_bytes_for_editor(path).await?;
        let (text, encoding) = decode_text_detect(&bytes);
        Ok(FileContent { text, encoding: encoding.to_string(), mtime, total_size, truncated })
    }

    /// 图片预览用：先 stat 判断大小，超过 `max_bytes` 直接拒绝（不像文本那样截断读
    /// 一部分——半张图片解不出来，截断预览对图片没有意义），没超就整份读回。
    async fn read_binary_for_preview(&self, path: &str, max_bytes: u64) -> Result<Vec<u8>, AppError> {
        let total_size = self.file_size(path).await?;
        if total_size > max_bytes {
            return Err(AppError::Internal(format!(
                "文件过大（{:.1}MB），无法预览",
                total_size as f64 / 1024.0 / 1024.0
            )));
        }
        let (bytes, _mtime) = self.read_file_raw(path).await?;
        Ok(bytes)
    }

    /// "用系统默认程序打开"（Word/Excel/PDF 等编辑器不支持预览或不支持编辑的文件类型，
    /// 2026-08-28 用户反馈）：系统程序不认识 SSH/SFTP 路径，必须先把内容落到本地磁盘
    /// 上的一个真实路径。默认实现直接整篇读入内存再写盘（本地场景足够快）；`RemoteFileOps`
    /// 覆盖成流式的 `download_to_local`，不会把大文件整个吸进内存。
    async fn download_to_local_file(&self, path: &str, local_path: &str) -> Result<(), AppError> {
        let (bytes, _mtime) = self.read_file_raw(path).await?;
        tokio::fs::write(local_path, &bytes).await.map_err(AppError::from)
    }

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

    /// 对指定的一批文件做"查找并替换全部"，直接写盘（没有类似 AI 编程助手那样的
    /// Diff 待确认流程——手动查找替换是用户自己敲的查询词，风险和"改一个文件后
    /// Ctrl+S"是同一量级，不需要额外的二次确认层；真出错了本来就该靠 Git/备份兜底，
    /// 和这个应用里其它"编辑即保存"的操作一致）。返回改了几个文件、替换了几处，
    /// 供前端展示结果摘要。固定按 UTF-8 写回——和 `write_file` 默认实现的既有约定
    /// 一致（trait 顶部文档注释已经说明这个取舍），非 UTF-8 原文件替换后会被转成
    /// UTF-8，不追求保留原始编码。
    async fn replace_text(
        &self,
        paths: &[String],
        query: &str,
        replacement: &str,
        options: &SearchOptions,
    ) -> Result<ReplaceSummary, AppError> {
        let matcher = build_matcher(query, options)?;
        let mut files_changed = 0usize;
        let mut occurrences_replaced = 0usize;

        for path in paths {
            let Ok((bytes, _)) = self.read_file_raw(path).await else { continue };
            let (text, _) = decode_text_detect(&bytes);
            let count = matcher.find_iter(&text).count();
            if count == 0 {
                continue;
            }
            // 非正则模式下替换文本是字面量，`$` 不该被解释成捕获组引用（比如替换成
            // "价格: $5"）；正则模式下按用户预期支持 `$1` 这类捕获组回填。
            let replaced = if options.use_regex {
                matcher.replace_all(&text, replacement).into_owned()
            } else {
                matcher.replace_all(&text, regex::NoExpand(replacement)).into_owned()
            };
            self.write_file_bytes(path, replaced.as_bytes(), None).await?;
            files_changed += 1;
            occurrences_replaced += count;
        }

        Ok(ReplaceSummary { files_changed, occurrences_replaced })
    }
}

/// 跨后端复制（本地磁盘 <-> Agent 远程主机，AGENT_DESIGN.md §四.3 的双栏浏览器
/// 用它实现"下载到本地"/"上传到远程"）。和 trait 默认的 `copy` 方法不同——那个
/// 只能在同一个 `FileOps` 实现内部复制（`&self` 用了两次），这里两侧可以是不同的
/// `FileOps` 实现：文件直接读字节写字节，目录先在目的端建好、再逐个子项递归，
/// 复用两边现成的基础原语，和 `fsops::remote::download_recursive`/
/// `upload_recursive` 是同一个思路，只是那两个是 SFTP 专属（流式、走 SFTP
/// 文件句柄直接 `tokio::io::copy`），这个是给"源和目的分属两种协议"这个场景的
/// 通用版本，不追求流式（一次性整读整写，和 trait 默认 `copy` 的代价量级一致）。
///
/// `progress` 语义和 `download_recursive` 一致：按"完成了第几个文件"报告，
/// 不做字节级百分比，事件名固定 `agent:transfer-progress`（不和 SFTP 那边的
/// `sftp:transfer-progress` 共用，避免两种协议的传输进度事件互相干扰）。
pub async fn copy_between(
    src: &dyn FileOps,
    src_path: &str,
    dst: &dyn FileOps,
    dst_path: &str,
    is_dir: bool,
    progress: &Option<(AppHandle, Uuid)>,
    should_cancel: &(dyn Fn() -> bool + Send + Sync),
    file_count: &std::sync::atomic::AtomicU64,
) -> Result<(), AppError> {
    if should_cancel() {
        return Err(AppError::Internal(TRANSFER_CANCELLED_MESSAGE.into()));
    }
    if !is_dir {
        let (bytes, _) = src.read_file_raw(src_path).await?;
        dst.write_file_bytes(dst_path, &bytes, None).await?;
        file_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if let Some((app, request_id)) = progress {
            let _ = app.emit("agent:transfer-progress", serde_json::json!({ "requestId": request_id, "path": src_path }));
        }
        return Ok(());
    }
    dst.create_dir(dst_path).await?;
    for entry in src.list_dir(src_path).await? {
        let child_dst = format!("{}/{}", dst_path.trim_end_matches(['/', '\\']), entry.name);
        Box::pin(copy_between(src, &entry.path, dst, &child_dst, entry.is_dir, progress, should_cancel, file_count)).await?;
    }
    Ok(())
}

/// 工作区全文搜索（左侧目录树的"搜索"功能，参考 VS Code 全局搜索面板）。不是
/// `FileOps` trait 方法——为了能一边遍历一边把结果推给调用方（`on_file` 回调），
/// 用 trait 方法（尤其是要保持 object-safe）不好表达"边搜边报告进度"，索性写成
/// 一个吃 `&dyn FileOps` 的自由函数，一样复用 `list_dir`/`read_file_raw` 这两个
/// 基础原语，本地/远程共用同一套遍历逻辑（和 `copy`/`delete` 的默认实现是同一个
/// "只依赖基础原语"的思路，只是这次不适合放在 trait 里）。
///
/// **2026-08-18 从"跑完整个工作区才一次性返回"改成流式**（用户原话："这个搜索功能
/// 太慢了，半天转不出来，能否一个一个目录搜，搜到一部分先展示一部分"）：调用方在
/// 每找到一个命中文件时立刻调 `on_file` 上报（命令层再转成 Tauri 事件推给前端），
/// 不用等整棵树扫完；同时新增 `should_cancel` 钩子，每处理一个目录/文件都检查一次，
/// 用户输入新的搜索词时命令层会让上一次搜索的这个钩子返回 true，尽快中止正在跑的
/// 旧搜索，不会几个搜索并发着抢 CPU/IO。
///
/// **已知取舍**：不是索引查询，每次搜索都要把未被排除的文件全部读一遍——本地走
/// `std::fs` 很快，远程每个文件是一次独立的 SFTP round trip，工作区文件数很大时
/// 会明显慢于 VS Code 的 ripgrep；流式展示能缓解"感觉卡死"的体验问题，但不改变
/// 总耗时。真要提速的路径有两条：一是本函数新增的 `SearchMode::FileName`（只比对
/// 文件名，不读文件内容，同样的目录遍历几乎不产生额外 I/O，配合下面"限定子目录"
/// 用法能覆盖大多数"我在找一个文件"的场景）；二是远程内容搜索换成对 SSH 侧跑
/// `grep -rn`（和日志模块的远程实时搜索是同一个思路），属于后续按需再做的优化。
pub async fn search_stream(
    file_ops: &dyn FileOps,
    root: &str,
    query: &str,
    options: &SearchOptions,
    mode: SearchMode,
    mut on_file: impl FnMut(SearchFileResult),
    mut should_cancel: impl FnMut() -> bool,
) -> Result<bool, AppError> {
    let matcher = build_matcher(query, options)?;

    let mut files_found = 0usize;
    let mut total_matches = 0usize;
    let mut truncated = false;
    let mut stack = vec![root.to_string()];

    'walk: while let Some(dir) = stack.pop() {
        if should_cancel() {
            break;
        }
        let entries = match file_ops.list_dir(&dir).await {
            Ok(e) => e,
            Err(_) => continue, // 单个子目录列不出来（权限等）不影响其它目录，跳过就好
        };
        for entry in entries {
            if should_cancel() {
                break 'walk;
            }
            if entry.is_dir {
                if !is_excluded_dir(&entry.name) {
                    stack.push(entry.path.clone());
                }
                continue;
            }

            match mode {
                SearchMode::FileName => {
                    if let Some(m) = matcher.find(&entry.name) {
                        let match_start = entry.name[..m.start()].chars().count();
                        let match_end = entry.name[..m.end()].chars().count();
                        files_found += 1;
                        total_matches += 1;
                        on_file(SearchFileResult {
                            path: entry.path.clone(),
                            matches: vec![SearchMatch {
                                line_number: 1,
                                line_text: entry.name.clone(),
                                match_start,
                                match_end,
                            }],
                        });
                    }
                }
                SearchMode::Content => {
                    if is_binary_extension(&entry.name) {
                        continue;
                    }
                    if entry.size.map(|s| s > SEARCH_MAX_FILE_BYTES).unwrap_or(false) {
                        continue;
                    }
                    let Ok((bytes, _)) = file_ops.read_file_raw(&entry.path).await else { continue };
                    if bytes.len() as u64 > SEARCH_MAX_FILE_BYTES || looks_binary(&bytes) {
                        continue;
                    }
                    let (text, _) = decode_text_detect(&bytes);

                    let mut file_matches = Vec::new();
                    'lines: for (idx, line) in text.lines().enumerate() {
                        for m in matcher.find_iter(line) {
                            let match_start = line[..m.start()].chars().count();
                            let match_end = line[..m.end()].chars().count();
                            file_matches.push(SearchMatch { line_number: idx + 1, line_text: line.to_string(), match_start, match_end });
                            total_matches += 1;
                            if file_matches.len() >= SEARCH_MAX_MATCHES_PER_FILE || total_matches >= SEARCH_MAX_MATCHES {
                                break 'lines;
                            }
                        }
                    }
                    if !file_matches.is_empty() {
                        files_found += 1;
                        on_file(SearchFileResult { path: entry.path.clone(), matches: file_matches });
                    }
                }
            }

            if files_found >= SEARCH_MAX_FILES || total_matches >= SEARCH_MAX_MATCHES {
                truncated = true;
                break 'walk;
            }
        }
    }

    Ok(truncated)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileContent {
    pub text: String,
    /// 探测到（或调用方强制指定）的编码标签，供编辑器状态栏展示
    /// （参考 VS Code 右下角的编码指示器）。
    pub encoding: String,
    /// Unix 时间戳（秒），供保存时做冲突检测
    pub mtime: i64,
    /// 文件总字节数（不是 `text` 的长度——`truncated` 为 true 时 `text` 只是前一小段）。
    pub total_size: u64,
    /// 文件超过大小阈值、`text` 只是截断预览时为 true；前端应据此把编辑器切成只读，
    /// 禁止保存（截断内容写回会把真实的大文件截断成预览这么大，等于丢数据）。
    pub truncated: bool,
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

#[cfg(test)]
mod search_tests {
    use super::*;

    fn opts(case_sensitive: bool, whole_word: bool, use_regex: bool) -> SearchOptions {
        SearchOptions { case_sensitive, whole_word, use_regex }
    }

    #[test]
    fn plain_query_is_case_insensitive_by_default() {
        let m = build_matcher("Hello", &opts(false, false, false)).unwrap();
        assert!(m.is_match("say hello world"));
    }

    #[test]
    fn case_sensitive_option_respected() {
        let m = build_matcher("Hello", &opts(true, false, false)).unwrap();
        assert!(!m.is_match("say hello world"));
        assert!(m.is_match("say Hello world"));
    }

    #[test]
    fn whole_word_does_not_match_substring() {
        let m = build_matcher("log", &opts(false, true, false)).unwrap();
        assert!(!m.is_match("catalogue"));
        assert!(m.is_match("write to log now"));
    }

    #[test]
    fn plain_mode_treats_regex_syntax_as_literal() {
        // 非正则模式下 "a.b" 应该只匹配字面的 "a.b"，不能把 "." 当通配符吃掉 "axb"。
        let m = build_matcher("a.b", &opts(false, false, false)).unwrap();
        assert!(m.is_match("a.b"));
        assert!(!m.is_match("axb"));
    }

    #[test]
    fn regex_mode_honors_pattern_syntax() {
        let m = build_matcher(r"a\d+b", &opts(false, false, true)).unwrap();
        assert!(m.is_match("a123b"));
        assert!(!m.is_match("aXb"));
    }

    #[test]
    fn match_offsets_are_char_indices_not_byte_indices() {
        // "中文a" 里 "a" 前有两个多字节汉字（每个 3 字节），字符下标应该是 2，
        // 不是字节下标 6——否则前端按 JS 字符串下标切片高亮位置会错位。
        let m = build_matcher("a", &opts(true, false, false)).unwrap();
        let line = "中文a";
        let found = m.find(line).unwrap();
        let char_start = line[..found.start()].chars().count();
        assert_eq!(char_start, 2);
    }
}
