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
                "name": "read_evidence",
                "description": "按 evidence_id 读取之前搜索/读取过的证据片段，适合上下文压缩后恢复已定位内容",
                "parameters": {
                    "type": "object",
                    "properties": { "evidence_id": { "type": "string" }, "start_line": { "type": "integer" }, "end_line": { "type": "integer" } },
                    "required": ["evidence_id"]
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
                "name": "web_search",
                "description": "访问互联网搜索最新网页、新闻和公开资料。用户询问今天、最新、新闻、股价或需要外部事实时必须优先调用；结果包含标题、摘要和 URL，回答时引用 URL。",
                "parameters": {
                    "type": "object",
                    "properties": { "query": { "type": "string", "description": "互联网搜索关键词，用空格分隔，尽量包含公司、主题和时间范围；不要直接照抄用户的整句提问（如“你能搜索一下…吗”），否则搜索引擎会按虚词分词导致结果不相关" } },
                    "required": ["query"]
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
        {
            "type": "function",
            "function": {
                "name": "glob",
                "description": "按文件名/路径模式查找文件（不读取文件内容），适合\"这个项目里有哪些 .rs 文件\"这类按文件名定位的场景；和 search_files（按内容搜索）互补",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "pattern": { "type": "string", "description": "文件名匹配的关键词/片段" },
                        "path": { "type": "string", "description": "搜索起始目录" }
                    },
                    "required": ["pattern", "path"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "webfetch",
                "description": "抓取一个指定 URL 的网页内容并转成文本；和 web_search 不同——这个工具用于\"已经知道具体网址、需要读取其内容\"的场景。只在 Build 模式下可用",
                "parameters": {
                    "type": "object",
                    "properties": { "url": { "type": "string", "description": "完整 URL，含 http(s):// 前缀" } },
                    "required": ["url"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "todo_write",
                "description": "创建/更新一份结构化任务清单，用于向用户展示多步任务的实时进度；每次调用传入完整的最新清单（不是增量），界面会实时渲染",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "todos": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "id": { "type": "string" },
                                    "content": { "type": "string" },
                                    "status": { "type": "string", "enum": ["pending", "in_progress", "completed"] }
                                },
                                "required": ["id", "content", "status"]
                            }
                        }
                    },
                    "required": ["todos"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "question",
                "description": "向用户提出一个结构化问题并等待回答，用于任务中出现需要用户决策/澄清的岔路口（而不是把问题混在最终答案文字里）；也适用于用户这句话本身有明显歧义、可能对应几种差别很大的意图时——把想到的几种理解列成 options 让用户选，不要凭猜测直接选一种展开长篇回答。提供 options 时前端渲染成按钮组，否则渲染文本输入框",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "question": { "type": "string" },
                        "options": { "type": "array", "items": { "type": "string" }, "description": "可选的候选答案列表，不提供则用户自由输入" }
                    },
                    "required": ["question"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "skill",
                "description": "按名称加载一份项目技能说明（SKILL.md 正文），系统提示词里已经列出了当前工作区可用的技能名称和简介，需要用到某个技能的详细操作步骤时调用",
                "parameters": {
                    "type": "object",
                    "properties": { "name": { "type": "string" } },
                    "required": ["name"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "multi_edit",
                "description": "对同一个文件一次性做多处精确替换，按顺序依次应用（后一处 old_text 是在前面已经替换过的内容基础上匹配的），只有全部替换都成功才会生成一份 Diff——比连续多次调用 edit_file 更省工具调用次数，也避免中途失败留下部分修改。只在 Build 模式下可用",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" },
                        "edits": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "old_text": { "type": "string" },
                                    "new_text": { "type": "string" }
                                },
                                "required": ["old_text", "new_text"]
                            }
                        }
                    },
                    "required": ["path", "edits"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "git_status",
                "description": "查看工作区当前的 Git 状态（相当于 git status --porcelain），只读、不需要确认。工作区根目录不是 Git 仓库时会明确告知",
                "parameters": {
                    "type": "object",
                    "properties": { "path": { "type": "string", "description": "只看某个子目录/文件，不传则看整个仓库" } },
                    "required": []
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "git_diff",
                "description": "查看尚未暂存的改动内容（相当于 git diff），只读、不需要确认；改动范围大时建议先传 path 缩小到相关文件，避免一次性看到大量无关 diff",
                "parameters": {
                    "type": "object",
                    "properties": { "path": { "type": "string", "description": "只看某个子目录/文件的 diff，不传则看整个仓库" } },
                    "required": []
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "git_commit",
                "description": "把指定路径的改动 add 后提交一次 commit（相当于 git add -- <paths> && git commit -m <message>）；不确定具体动了哪些文件时先调用 git_status 确认。只在 Build 模式下可用，和 run_command 走同一套黑名单/权限规则/用户确认",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "message": { "type": "string" },
                        "paths": { "type": "array", "items": { "type": "string" }, "description": "要提交的文件/目录路径，不能为空——不支持一次性提交整个仓库的隐式写法，避免误提交不相关的文件" }
                    },
                    "required": ["message", "paths"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "run_command_background",
                "description": "在本地工作区后台启动一个不等待其退出的进程（比如开发服务器、长期挂着的 watch 进程），立即返回一个 job_id；用 read_background_output 查看它目前的输出，用 stop_background_process 结束它。只支持本地工作区（远程/Agent 目标不可用），只在 Build 模式下可用，和 run_command 走同一套黑名单/权限规则/用户确认。普通的一次性命令仍然应该用 run_command，不要为了图快而滥用这个",
                "parameters": {
                    "type": "object",
                    "properties": { "command": { "type": "string" } },
                    "required": ["command"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "read_background_output",
                "description": "查看 run_command_background 启动的某个后台进程目前的运行状态和已输出内容（累计输出，不是增量）。只在 Build 模式下可用",
                "parameters": {
                    "type": "object",
                    "properties": { "job_id": { "type": "string" } },
                    "required": ["job_id"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "stop_background_process",
                "description": "结束 run_command_background 启动的某个后台进程。只在 Build 模式下可用",
                "parameters": {
                    "type": "object",
                    "properties": { "job_id": { "type": "string" } },
                    "required": ["job_id"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "find_definition",
                "description": "按符号名（函数/类型/宏名）在工作区代码里查找定义位置——基于正则的轻量索引（不是真正的语言语义分析），第一次调用时会自动扫描整个工作区建索引，之后复用。多行签名、宏生成的定义、重载可能漏掉或者返回多个候选；没找到或者结果看起来不对时改用 search_files 按关键词搜索",
                "parameters": {
                    "type": "object",
                    "properties": { "symbol": { "type": "string" } },
                    "required": ["symbol"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "task",
                "description": "把一个具体的子任务委派给一个临时的、独立上下文的子代理去完成——子代理能用和你一样的工具（文件读写/搜索/命令等，取决于当前模式），完成后只把一段文字总结交还给你，你看不到它的中间探索过程。适合\"在这一大堆文件里找到某个具体实现/结论\"这类会消耗大量探索性工具调用、但你只需要一个结论的子任务，能避免这些过程细节占满你自己的上下文。子代理不能再往下委派子任务、也不能向用户提问，遇到需要用户决策的岔路口会自己做一个合理假设并在总结里说明，你需要在这个总结的基础上判断要不要追问用户",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "description": { "type": "string", "description": "3-6 个字的子任务简称，用于界面展示" },
                        "prompt": { "type": "string", "description": "交给子代理的完整任务说明，要包含足够的背景信息——子代理看不到你和用户之间的对话历史" }
                    },
                    "required": ["description", "prompt"]
                }
            }
        },
    ])
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    Pending,
    InProgress,
    Completed,
}

#[derive(Debug, Clone, serde::Serialize, Deserialize)]
pub struct TodoItem {
    pub id: String,
    pub content: String,
    pub status: TodoStatus,
}

#[derive(Debug, Clone)]
pub enum ToolCall {
    ReadFile {
        path: String,
    },
    ReadEvidence { id: uuid::Uuid, start_line: Option<usize>, end_line: Option<usize> },
    ListDirectory {
        path: String,
    },
    SearchFiles {
        pattern: String,
        path: String,
    },
    WebSearch {
        query: String,
    },
    WriteFile {
        path: String,
        content: String,
    },
    EditFile {
        path: String,
        old_text: String,
        new_text: String,
    },
    RunCommand {
        command: String,
    },
    Glob {
        pattern: String,
        path: String,
    },
    WebFetch {
        url: String,
    },
    TodoWrite {
        todos: Vec<TodoItem>,
    },
    Question {
        question: String,
        options: Vec<String>,
    },
    Skill {
        name: String,
    },
    MultiEdit {
        path: String,
        edits: Vec<MultiEditItem>,
    },
    GitStatus {
        path: Option<String>,
    },
    GitDiff {
        path: Option<String>,
    },
    GitCommit {
        message: String,
        paths: Vec<String>,
    },
    RunCommandBackground {
        command: String,
    },
    ReadBackgroundOutput {
        job_id: uuid::Uuid,
    },
    StopBackgroundProcess {
        job_id: uuid::Uuid,
    },
    FindDefinition {
        symbol: String,
    },
    Task {
        description: String,
        prompt: String,
    },
    /// MCP 工具调用不走这里的静态解析——工具名是运行时按已连接的服务器动态生成
    /// 的（`mcp__<server>__<tool>`），`CodingSession::send_message` 在调
    /// `parse_tool_call` 之前先检查这个前缀，命中就直接构造这个变体，见
    /// `coding/session.rs`。
    Mcp {
        server_id: uuid::Uuid,
        tool_name: String,
        arguments: serde_json::Value,
    },
}

#[derive(Deserialize)]
struct ReadFileArgs {
    path: String,
}
#[derive(Deserialize)]
struct ReadEvidenceArgs { evidence_id: uuid::Uuid, start_line: Option<usize>, end_line: Option<usize> }
#[derive(Deserialize)]
struct ListDirectoryArgs {
    path: String,
}
#[derive(Deserialize)]
struct SearchFilesArgs {
    pattern: String,
    path: String,
}
#[derive(Deserialize)]
struct WebSearchArgs {
    query: String,
}
#[derive(Deserialize)]
struct WriteFileArgs {
    path: String,
    content: String,
}
#[derive(Deserialize)]
struct EditFileArgs {
    path: String,
    old_text: String,
    new_text: String,
}
#[derive(Deserialize)]
struct RunCommandArgs {
    command: String,
}
#[derive(Deserialize)]
struct GlobArgs {
    pattern: String,
    path: String,
}
#[derive(Deserialize)]
struct WebFetchArgs {
    url: String,
}
#[derive(Deserialize)]
struct TodoWriteArgs {
    todos: Vec<TodoItem>,
}
#[derive(Deserialize)]
struct QuestionArgs {
    question: String,
    #[serde(default)]
    options: Vec<String>,
}
#[derive(Deserialize)]
struct SkillArgs {
    name: String,
}
#[derive(Debug, Clone, Deserialize)]
pub struct MultiEditItem {
    pub old_text: String,
    pub new_text: String,
}
#[derive(Deserialize)]
struct MultiEditArgs {
    path: String,
    edits: Vec<MultiEditItem>,
}
#[derive(Deserialize)]
struct GitStatusArgs {
    #[serde(default)]
    path: Option<String>,
}
#[derive(Deserialize)]
struct GitDiffArgs {
    #[serde(default)]
    path: Option<String>,
}
#[derive(Deserialize)]
struct GitCommitArgs {
    message: String,
    paths: Vec<String>,
}
#[derive(Deserialize)]
struct RunCommandBackgroundArgs {
    command: String,
}
#[derive(Deserialize)]
struct ReadBackgroundOutputArgs {
    job_id: uuid::Uuid,
}
#[derive(Deserialize)]
struct StopBackgroundProcessArgs {
    job_id: uuid::Uuid,
}
#[derive(Deserialize)]
struct FindDefinitionArgs {
    symbol: String,
}
#[derive(Deserialize)]
struct TaskArgs {
    description: String,
    prompt: String,
}

pub fn parse_tool_call(name: &str, arguments_json: &str) -> Result<ToolCall, AppError> {
    let bad_args = |e: serde_json::Error| {
        AppError::Internal(format!("invalid tool arguments for {name}: {e}"))
    };
    match name {
        "read_file" => {
            let a: ReadFileArgs = serde_json::from_str(arguments_json).map_err(bad_args)?;
            Ok(ToolCall::ReadFile { path: a.path })
        }
        "read_evidence" => {
            let a: ReadEvidenceArgs = serde_json::from_str(arguments_json).map_err(bad_args)?;
            Ok(ToolCall::ReadEvidence { id: a.evidence_id, start_line: a.start_line, end_line: a.end_line })
        }
        "list_directory" => {
            let a: ListDirectoryArgs = serde_json::from_str(arguments_json).map_err(bad_args)?;
            Ok(ToolCall::ListDirectory { path: a.path })
        }
        "search_files" => {
            let a: SearchFilesArgs = serde_json::from_str(arguments_json).map_err(bad_args)?;
            Ok(ToolCall::SearchFiles {
                pattern: a.pattern,
                path: a.path,
            })
        }
        "web_search" => {
            let a: WebSearchArgs = serde_json::from_str(arguments_json).map_err(bad_args)?;
            Ok(ToolCall::WebSearch { query: a.query })
        }
        "write_file" => {
            let a: WriteFileArgs = serde_json::from_str(arguments_json).map_err(bad_args)?;
            Ok(ToolCall::WriteFile {
                path: a.path,
                content: a.content,
            })
        }
        "edit_file" => {
            let a: EditFileArgs = serde_json::from_str(arguments_json).map_err(bad_args)?;
            Ok(ToolCall::EditFile {
                path: a.path,
                old_text: a.old_text,
                new_text: a.new_text,
            })
        }
        "run_command" => {
            let a: RunCommandArgs = serde_json::from_str(arguments_json).map_err(bad_args)?;
            Ok(ToolCall::RunCommand { command: a.command })
        }
        "glob" => {
            let a: GlobArgs = serde_json::from_str(arguments_json).map_err(bad_args)?;
            Ok(ToolCall::Glob {
                pattern: a.pattern,
                path: a.path,
            })
        }
        "webfetch" => {
            let a: WebFetchArgs = serde_json::from_str(arguments_json).map_err(bad_args)?;
            Ok(ToolCall::WebFetch { url: a.url })
        }
        "todo_write" => {
            let a: TodoWriteArgs = serde_json::from_str(arguments_json).map_err(bad_args)?;
            Ok(ToolCall::TodoWrite { todos: a.todos })
        }
        "question" => {
            let a: QuestionArgs = serde_json::from_str(arguments_json).map_err(bad_args)?;
            Ok(ToolCall::Question {
                question: a.question,
                options: a.options,
            })
        }
        "skill" => {
            let a: SkillArgs = serde_json::from_str(arguments_json).map_err(bad_args)?;
            Ok(ToolCall::Skill { name: a.name })
        }
        "multi_edit" => {
            let a: MultiEditArgs = serde_json::from_str(arguments_json).map_err(bad_args)?;
            Ok(ToolCall::MultiEdit { path: a.path, edits: a.edits })
        }
        "git_status" => {
            let a: GitStatusArgs = serde_json::from_str(arguments_json).map_err(bad_args)?;
            Ok(ToolCall::GitStatus { path: a.path })
        }
        "git_diff" => {
            let a: GitDiffArgs = serde_json::from_str(arguments_json).map_err(bad_args)?;
            Ok(ToolCall::GitDiff { path: a.path })
        }
        "git_commit" => {
            let a: GitCommitArgs = serde_json::from_str(arguments_json).map_err(bad_args)?;
            Ok(ToolCall::GitCommit { message: a.message, paths: a.paths })
        }
        "run_command_background" => {
            let a: RunCommandBackgroundArgs = serde_json::from_str(arguments_json).map_err(bad_args)?;
            Ok(ToolCall::RunCommandBackground { command: a.command })
        }
        "read_background_output" => {
            let a: ReadBackgroundOutputArgs = serde_json::from_str(arguments_json).map_err(bad_args)?;
            Ok(ToolCall::ReadBackgroundOutput { job_id: a.job_id })
        }
        "stop_background_process" => {
            let a: StopBackgroundProcessArgs = serde_json::from_str(arguments_json).map_err(bad_args)?;
            Ok(ToolCall::StopBackgroundProcess { job_id: a.job_id })
        }
        "find_definition" => {
            let a: FindDefinitionArgs = serde_json::from_str(arguments_json).map_err(bad_args)?;
            Ok(ToolCall::FindDefinition { symbol: a.symbol })
        }
        "task" => {
            let a: TaskArgs = serde_json::from_str(arguments_json).map_err(bad_args)?;
            Ok(ToolCall::Task { description: a.description, prompt: a.prompt })
        }
        other => Err(AppError::Internal(format!("unknown tool: {other}"))),
    }
}

/// 递归本地文件内容搜索（DESIGN.md `search_files` 工具，本地分支）。不引入 walkdir
/// 依赖——工具本身用途有限（供 AI 快速定位文件，不是给用户用的通用搜索），
/// 一个简单的递归 + 逐行 contains 匹配足够，同时主动跳过 `.git`/`node_modules`
/// 等大目录，避免一次调用扫描出几十万行结果拖垮工具循环。
pub fn search_files_local(
    root: &std::path::Path,
    pattern: &str,
    max_results: usize,
) -> Vec<String> {
    const SKIP_DIRS: &[&str] = &[".git", "node_modules", "target", "dist", "build", ".venv"];
    let mut results = Vec::new();
    let mut stack = vec![root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        if results.len() >= max_results {
            break;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
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
            let Ok(content) = std::fs::read_to_string(&path) else {
                continue;
            };
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
