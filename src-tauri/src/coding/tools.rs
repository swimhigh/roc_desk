use serde::Deserialize;
use serde_json::json;

use crate::error::AppError;

/// AI 工具集定义（DESIGN.md §3.8.2 表格），按 OpenAI function-calling 的
/// `tools` 数组格式描述给 LLM。`undo_change`/`create_diff` 没有做成 LLM 可调用的
/// 工具——前者是用户在 UI 上点按钮的操作，后者是 `write_file`/`edit_file` 内部
/// 自动做的事，真实场景里几乎没有模型会主动"调用撤销"，参考 Aider/Cursor 的实现
/// 都是把这两个处理成宿主侧逻辑而不是暴露给模型的工具。
pub fn tool_schema() -> serde_json::Value {
    json!([
        {
            "type": "function",
            "function": {
                "name": "read_file",
                "description": "读取工作区内某个文件的完整内容",
                "parameters": {
                    "type": "object",
                    "properties": { "path": { "type": "string", "description": "相对或绝对路径" } },
                    "required": ["path"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "list_directory",
                "description": "列出某个目录下的文件和子目录",
                "parameters": {
                    "type": "object",
                    "properties": { "path": { "type": "string" } },
                    "required": ["path"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "search_files",
                "description": "在目录下按关键词搜索文件内容（类似 grep）",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "pattern": { "type": "string" },
                        "path": { "type": "string", "description": "搜索起始目录" }
                    },
                    "required": ["pattern", "path"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "write_file",
                "description": "创建新文件或整体覆盖已有文件的内容；只在 Build 模式下可用，改动会生成 Diff 等待用户确认后才真正落盘",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "content": { "type": "string" }
                    },
                    "required": ["path", "content"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "edit_file",
                "description": "对已有文件做精确的局部替换（old_text 必须在文件中唯一出现一次）；只在 Build 模式下可用，改动会生成 Diff 等待用户确认后才真正落盘",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "old_text": { "type": "string" },
                        "new_text": { "type": "string" }
                    },
                    "required": ["path", "old_text", "new_text"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "run_command",
                "description": "在目标主机上执行一条 Shell 命令；只在 Build 模式下可用，命中黑名单会被硬拦截，否则需要用户确认后才会真正执行",
                "parameters": {
                    "type": "object",
                    "properties": { "command": { "type": "string" } },
                    "required": ["command"]
                }
            }
        },
    ])
}

#[derive(Debug, Clone)]
pub enum ToolCall {
    ReadFile { path: String },
    ListDirectory { path: String },
    SearchFiles { pattern: String, path: String },
    WriteFile { path: String, content: String },
    EditFile { path: String, old_text: String, new_text: String },
    RunCommand { command: String },
}

#[derive(Deserialize)]
struct ReadFileArgs { path: String }
#[derive(Deserialize)]
struct ListDirectoryArgs { path: String }
#[derive(Deserialize)]
struct SearchFilesArgs { pattern: String, path: String }
#[derive(Deserialize)]
struct WriteFileArgs { path: String, content: String }
#[derive(Deserialize)]
struct EditFileArgs { path: String, old_text: String, new_text: String }
#[derive(Deserialize)]
struct RunCommandArgs { command: String }

pub fn parse_tool_call(name: &str, arguments_json: &str) -> Result<ToolCall, AppError> {
    let bad_args = |e: serde_json::Error| AppError::Internal(format!("invalid tool arguments for {name}: {e}"));
    match name {
        "read_file" => {
            let a: ReadFileArgs = serde_json::from_str(arguments_json).map_err(bad_args)?;
            Ok(ToolCall::ReadFile { path: a.path })
        }
        "list_directory" => {
            let a: ListDirectoryArgs = serde_json::from_str(arguments_json).map_err(bad_args)?;
            Ok(ToolCall::ListDirectory { path: a.path })
        }
        "search_files" => {
            let a: SearchFilesArgs = serde_json::from_str(arguments_json).map_err(bad_args)?;
            Ok(ToolCall::SearchFiles { pattern: a.pattern, path: a.path })
        }
        "write_file" => {
            let a: WriteFileArgs = serde_json::from_str(arguments_json).map_err(bad_args)?;
            Ok(ToolCall::WriteFile { path: a.path, content: a.content })
        }
        "edit_file" => {
            let a: EditFileArgs = serde_json::from_str(arguments_json).map_err(bad_args)?;
            Ok(ToolCall::EditFile { path: a.path, old_text: a.old_text, new_text: a.new_text })
        }
        "run_command" => {
            let a: RunCommandArgs = serde_json::from_str(arguments_json).map_err(bad_args)?;
            Ok(ToolCall::RunCommand { command: a.command })
        }
        other => Err(AppError::Internal(format!("unknown tool: {other}"))),
    }
}

/// 递归本地文件内容搜索（DESIGN.md `search_files` 工具，本地分支）。不引入 walkdir
/// 依赖——工具本身用途有限（供 AI 快速定位文件，不是给用户用的通用搜索），
/// 一个简单的递归 + 逐行 contains 匹配足够，同时主动跳过 `.git`/`node_modules`
/// 等大目录，避免一次调用扫描出几十万行结果拖垮工具循环。
pub fn search_files_local(root: &std::path::Path, pattern: &str, max_results: usize) -> Vec<String> {
    const SKIP_DIRS: &[&str] = &[".git", "node_modules", "target", "dist", "build", ".venv"];
    let mut results = Vec::new();
    let mut stack = vec![root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        if results.len() >= max_results {
            break;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            if results.len() >= max_results {
                break;
            }
            let path = entry.path();
            let is_dir = path.is_dir();
            let name = entry.file_name().to_string_lossy().to_string();
            if is_dir {
                if !SKIP_DIRS.contains(&name.as_str()) {
                    stack.push(path);
                }
                continue;
            }
            let Ok(content) = std::fs::read_to_string(&path) else { continue };
            for (i, line) in content.lines().enumerate() {
                if line.contains(pattern) {
                    results.push(format!("{}:{}:{}", path.display(), i + 1, line.trim()));
                    if results.len() >= max_results {
                        break;
                    }
                }
            }
        }
    }
    results
}
