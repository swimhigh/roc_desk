pub mod encoding;
pub mod local;
pub mod remote;

use async_trait::async_trait;
use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};

use crate::error::AppError;
use encoding::decode_text_detect;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchSummary {
    pub files: Vec<SearchFileResult>,
    /// 命中的文件数/总匹配数超过上限提前收手时置 true，前端据此提示"结果可能不全，
    /// 请缩小搜索范围"，而不是让用户误以为这就是全部结果。
    pub truncated: bool,
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

    /// 工作区全文搜索（左侧目录树的"搜索"功能，参考 VS Code 全局搜索面板，
    /// 2026-08-18 需求）。trait 默认实现，本地/远程共用同一套"用 `list_dir` 递归
    /// 遍历 + `read_file_raw` 逐文件读取再正则匹配"逻辑——和 `copy` 的默认实现是
    /// 同一个思路：只需要 `list_dir`/`read_file_raw` 这两个已经有的基础原语，不用
    /// 分别给本地/远程各写一套搜索。**已知取舍**：不是索引查询，每次搜索都要把
    /// 未被排除的文件全部读一遍——本地走 `std::fs` 很快，远程每个文件是一次独立的
    /// SFTP round trip，工作区文件数很大时会明显慢于 VS Code 的 ripgrep，但实现
    /// 简单、和已有的 `copy`/`delete` 递归模式一致，MVP 阶段够用；真要优化远程场景
    /// 可以换成对 SSH 侧跑 `grep -rn`（和日志模块的远程实时搜索是同一个思路），
    /// 属于后续按需再做的优化，不在这版范围内。
    async fn search_text(&self, root: &str, query: &str, options: &SearchOptions) -> Result<SearchSummary, AppError> {
        let matcher = build_matcher(query, options)?;

        let mut files = Vec::new();
        let mut total_matches = 0usize;
        let mut truncated = false;
        let mut stack = vec![root.to_string()];

        'walk: while let Some(dir) = stack.pop() {
            let entries = match self.list_dir(&dir).await {
                Ok(e) => e,
                Err(_) => continue, // 单个子目录列不出来（权限等）不影响其它目录，跳过就好
            };
            for entry in entries {
                if entry.is_dir {
                    if !is_excluded_dir(&entry.name) {
                        stack.push(entry.path.clone());
                    }
                    continue;
                }
                if is_binary_extension(&entry.name) {
                    continue;
                }
                if let Some(size) = entry.size {
                    if size > SEARCH_MAX_FILE_BYTES {
                        continue;
                    }
                }
                let Ok((bytes, _)) = self.read_file_raw(&entry.path).await else { continue };
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
                    files.push(SearchFileResult { path: entry.path.clone(), matches: file_matches });
                }
                if files.len() >= SEARCH_MAX_FILES || total_matches >= SEARCH_MAX_MATCHES {
                    truncated = true;
                    break 'walk;
                }
            }
        }

        Ok(SearchSummary { files, truncated })
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
