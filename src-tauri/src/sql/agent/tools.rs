use serde::Deserialize;
use serde_json::json;

use crate::error::AppError;

pub use crate::coding::tools::{TodoItem, TodoStatus};

/// SQL Agent 的工具集——形状和 `coding::tools::tool_schema()` 完全一样（OpenAI
/// function-calling 的 `tools` 数组），只是把"文件/命令"换成"SQL 查询/表结构"。
/// `todo_write`/`question` 直接照抄 coding 那两个工具的定义和语义，连
/// [`TodoItem`]/[`TodoStatus`] 类型本身都是重新导出 `coding::tools` 里的，不是
/// 另建一份同名类型。
pub fn tool_schema() -> serde_json::Value {
    json!([
        {
            "type": "function",
            "function": {
                "name": "run_query",
                "description": "在当前连接的数据源上执行一条 SQL 语句并返回结果。只读查询（SELECT/EXPLAIN）会直接执行；\
                                 INSERT 也会直接执行；UPDATE/DELETE/DDL（CREATE/ALTER/DROP 等）会先弹窗让用户确认，\
                                 用户拒绝时这个工具会返回\"用户拒绝执行\"而不是报错。一次只能传一条语句。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "sql": { "type": "string", "description": "要执行的一条 SQL 语句，不要带结尾分号之外的多条语句" },
                        "row_limit": { "type": "integer", "description": "最多返回多少行，默认 50，避免结果集过大", "default": 50 }
                    },
                    "required": ["sql"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "describe_table",
                "description": "查看某张表/视图的列结构（列名、类型、是否可空、是否主键、注释）和索引信息",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "schema": { "type": "string", "description": "schema 名，不确定的话先用 list_objects 列出来" },
                        "name": { "type": "string", "description": "表名/视图名" }
                    },
                    "required": ["schema", "name"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "list_objects",
                "description": "列出当前数据库里的表/视图（可选按 schema 过滤），用于在写查询之前先确认有哪些表、分别在哪个 schema 下",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "schema": { "type": "string", "description": "只列出这个 schema 下的对象，不传则列出全部（结果会被截断到前一部分）" }
                    }
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
                "description": "向用户提出一个结构化问题并等待回答，用于任务中出现需要用户决策/澄清的岔路口（而不是把问题混在最终答案文字里）。提供 options 时前端渲染成按钮组，否则渲染文本输入框",
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
    ])
}

#[derive(Debug, Clone)]
pub enum ToolCall {
    RunQuery {
        sql: String,
        row_limit: usize,
    },
    DescribeTable {
        schema: String,
        name: String,
    },
    ListObjects {
        schema: Option<String>,
    },
    TodoWrite {
        todos: Vec<TodoItem>,
    },
    Question {
        question: String,
        options: Vec<String>,
    },
}

#[derive(Deserialize)]
struct RunQueryArgs {
    sql: String,
    #[serde(default)]
    row_limit: Option<usize>,
}
#[derive(Deserialize)]
struct DescribeTableArgs {
    schema: String,
    name: String,
}
#[derive(Deserialize)]
struct ListObjectsArgs {
    #[serde(default)]
    schema: Option<String>,
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

const DEFAULT_ROW_LIMIT: usize = 50;
const MAX_ROW_LIMIT: usize = 500;

pub fn parse_tool_call(name: &str, arguments_json: &str) -> Result<ToolCall, AppError> {
    let bad_args = |e: serde_json::Error| AppError::Internal(format!("invalid tool arguments for {name}: {e}"));
    match name {
        "run_query" => {
            let a: RunQueryArgs = serde_json::from_str(arguments_json).map_err(bad_args)?;
            Ok(ToolCall::RunQuery {
                sql: a.sql,
                row_limit: a.row_limit.unwrap_or(DEFAULT_ROW_LIMIT).clamp(1, MAX_ROW_LIMIT),
            })
        }
        "describe_table" => {
            let a: DescribeTableArgs = serde_json::from_str(arguments_json).map_err(bad_args)?;
            Ok(ToolCall::DescribeTable { schema: a.schema, name: a.name })
        }
        "list_objects" => {
            let a: ListObjectsArgs = serde_json::from_str(arguments_json).map_err(bad_args)?;
            Ok(ToolCall::ListObjects { schema: a.schema })
        }
        "todo_write" => {
            let a: TodoWriteArgs = serde_json::from_str(arguments_json).map_err(bad_args)?;
            Ok(ToolCall::TodoWrite { todos: a.todos })
        }
        "question" => {
            let a: QuestionArgs = serde_json::from_str(arguments_json).map_err(bad_args)?;
            Ok(ToolCall::Question { question: a.question, options: a.options })
        }
        other => Err(AppError::Internal(format!("unknown tool: {other}"))),
    }
}
