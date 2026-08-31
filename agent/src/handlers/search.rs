//! 工作区全文/文件名搜索（AGENT_DESIGN.md §一"远程内容搜索慢"一节）：这是 Agent
//! 方案相对 SFTP 的架构级提速——搜索逻辑在 Agent 进程本机跑（`std::fs`，不是
//! "客户端发起 N 次网络请求"），只有匹配结果经网络传回。逻辑上是
//! `fsops::search_stream`（src-tauri 侧）的独立镜像实现：两边故意不共享代码——
//! `protocol`/`agent` crate 不依赖 `src-tauri`，重复这几十行遍历逻辑比强行抽出一个
//! 三方共享 crate 更简单。

use std::path::Path;

use regex::{Regex, RegexBuilder};
use roc_desk_protocol::{ErrorCode, SearchFileResult, SearchMatch, SearchOptions};

const EXCLUDED_DIRS: &[&str] = &[".git", "node_modules", "target", "dist", "build", ".next", "__pycache__", ".venv", ".cargo"];
const BINARY_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "ico", "webp", "bmp", "mp4", "mp3", "wav", "avi", "mov", "zip", "tar", "gz", "7z", "rar", "exe",
    "dll", "pdb", "so", "dylib", "pdf", "woff", "woff2", "ttf", "eot", "db", "sqlite", "class", "jar", "wasm",
];
const MAX_FILES: usize = 500;
const MAX_MATCHES: usize = 3000;
const MAX_MATCHES_PER_FILE: usize = 50;
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

fn build_matcher(query: &str, options: &SearchOptions) -> Result<Regex, String> {
    if query.is_empty() {
        return Err("搜索内容不能为空".to_string());
    }
    let base = if options.use_regex { query.to_string() } else { regex::escape(query) };
    let pattern = if options.whole_word { format!(r"\b{base}\b") } else { base };
    RegexBuilder::new(&pattern).case_insensitive(!options.case_sensitive).build().map_err(|e| format!("正则表达式无效：{e}"))
}

fn is_excluded_dir(name: &str) -> bool {
    EXCLUDED_DIRS.contains(&name)
}

fn is_binary_extension(name: &str) -> bool {
    match name.rsplit_once('.') {
        Some((_, ext)) => BINARY_EXTENSIONS.contains(&ext.to_lowercase().as_str()),
        None => false,
    }
}

fn looks_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8192).any(|b| *b == 0)
}

/// 同步实现（阻塞 I/O），调用方负责 `spawn_blocking`——扫描一整个工作区目录树
/// 属于 CPU/IO 密集操作，不应该占用 tokio 的异步执行器线程。
pub fn search_content(root: &Path, query: &str, options: &SearchOptions) -> Result<Vec<SearchFileResult>, (ErrorCode, String)> {
    let matcher = build_matcher(query, options).map_err(|msg| (ErrorCode::InvalidArgument, msg))?;
    let mut results = Vec::new();
    let mut total_matches = 0usize;
    let mut stack = vec![root.to_path_buf()];

    'walk: while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            let Ok(file_type) = entry.file_type() else { continue };
            if file_type.is_dir() {
                if !is_excluded_dir(&name) {
                    stack.push(path);
                }
                continue;
            }
            if is_binary_extension(&name) {
                continue;
            }
            let Ok(metadata) = entry.metadata() else { continue };
            if metadata.len() > MAX_FILE_BYTES {
                continue;
            }
            let Ok(bytes) = std::fs::read(&path) else { continue };
            if looks_binary(&bytes) {
                continue;
            }
            let text = String::from_utf8_lossy(&bytes);

            let mut file_matches = Vec::new();
            'lines: for (idx, line) in text.lines().enumerate() {
                for m in matcher.find_iter(line) {
                    let match_start = line[..m.start()].chars().count();
                    let match_end = line[..m.end()].chars().count();
                    file_matches.push(SearchMatch { line_number: idx + 1, line_text: line.to_string(), match_start, match_end });
                    total_matches += 1;
                    if file_matches.len() >= MAX_MATCHES_PER_FILE || total_matches >= MAX_MATCHES {
                        break 'lines;
                    }
                }
            }
            if !file_matches.is_empty() {
                results.push(SearchFileResult { path: path.to_string_lossy().to_string(), matches: file_matches });
            }
            if results.len() >= MAX_FILES || total_matches >= MAX_MATCHES {
                break 'walk;
            }
        }
    }
    Ok(results)
}

pub fn search_filename(root: &Path, query: &str) -> Result<Vec<SearchFileResult>, (ErrorCode, String)> {
    let options = SearchOptions::default();
    let matcher = build_matcher(query, &options).map_err(|msg| (ErrorCode::InvalidArgument, msg))?;
    let mut results = Vec::new();
    let mut stack = vec![root.to_path_buf()];

    'walk: while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            let Ok(file_type) = entry.file_type() else { continue };
            if file_type.is_dir() {
                if !is_excluded_dir(&name) {
                    stack.push(path);
                }
                continue;
            }
            if let Some(m) = matcher.find(&name) {
                let match_start = name[..m.start()].chars().count();
                let match_end = name[..m.end()].chars().count();
                results.push(SearchFileResult {
                    path: path.to_string_lossy().to_string(),
                    matches: vec![SearchMatch { line_number: 1, line_text: name.clone(), match_start, match_end }],
                });
                if results.len() >= MAX_FILES {
                    break 'walk;
                }
            }
        }
    }
    Ok(results)
}
