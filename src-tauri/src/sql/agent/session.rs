use serde_json::json;
use sqlparser::ast::Statement;
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

use super::tools::{self, TodoItem, ToolCall};
use crate::agent_llm;
use crate::ai::AiProviderManager;
use crate::coding::{CommandConfirmRegistry, QuestionRegistry};
use crate::coding::ChatAttachment;
use crate::db::repo::sql_query_history_repo::SqlQueryHistoryRepo;
use crate::error::AppError;
use crate::sql::adapter::new_backend_handle_slot;
use crate::sql::model::{DbKind, ExecuteResult, ObjectDefinition, ObjectKind, ObjectRef};
use crate::sql::policy::{self, ExecutionKind};
use crate::sql::service::{SqlDataSourceService, SqlSessionManager};

/// SQL 对话通常比"分析整个项目"这类编程任务收敛快得多——一般是"看几眼表结构、
/// 跑一两条查询、给结论"，给一个比较小的预算即可；真的用完了不丢上下文，用户发
/// "继续"即可接着跑（`coding::session` 的工具循环 2026-09 应用户要求已经改成没有
/// 硬编码轮次上限的 `loop`，这里的预算/强制收尾机制是两边各自独立的实现，不联动）。
const MAX_TOOL_ITERATIONS: usize = 20;
const FORCE_CONCLUDE_LAST_N: usize = 3;
/// 触发裁剪的消息条数上限——没有做 `coding::session::limit_context` 那套按
/// token 估算 + LLM 摘要的精细策略（v1 范围裁剪，见模块文档）：SQL 对话通常
/// 短得多，且每条工具结果已经被 `agent_llm::cap_tool_result` 限制了大小，
/// 真正会撞上这个上限的场景应该很少；触发时直接整轮丢弃、不摘要。
const MAX_CONTEXT_MESSAGES: usize = 60;
const DEFAULT_SQL_CONTEXT_BUDGET: usize = 48_000;

/// SQL Desktop 的多轮 Agent 会话——和 `coding::CodingSession` 是同一种"多轮
/// 工具调用"架构（共用 `agent_llm` 里协议无关的那部分：请求构建/重试/
/// usage 抽取），但工具集换成了 `run_query`/`describe_table`/`list_objects`，
/// 也没有搬 coding 那边"文件 Diff/Git/MCP/Skills/远程主机目标"这些和 SQL 场景
/// 无关的机制——这些概念在这里没有对应物，硬套只会增加一层没有收益的抽象。
/// 写操作的确认门禁复用已有的 `sql::policy::classify`（和手动 SQL 编辑器
/// 完全一样的 Normal/Confirm/Transaction 分类），但简化了 `Transaction` 类
/// 语句（UPDATE/DELETE）的处理：手动编辑器会先在独立事务里跑出真实受影响
/// 行数、等用户二次确认提交/回滚；这里为了让一次工具调用能在一轮里给模型返回
/// 一个确定的结果，统一简化成"确认后直接执行并自动提交"，不暴露"先跑看看再
/// 决定提交/回滚"这一步——这是 v1 的一个刻意简化，不是被遗漏的功能。
pub struct SqlAgentSession {
    pub id: Uuid,
    pub data_source_id: Uuid,
    pub provider_id: Uuid,
    pub todos: Vec<TodoItem>,
    messages: Vec<serde_json::Value>,
}

impl SqlAgentSession {
    pub fn new(data_source_id: Uuid, provider_id: Uuid, data_source_label: &str, db_kind: DbKind) -> Self {
        let system_prompt = format!(
            "你是一个 SQL 助手，当前连接的数据源是「{data_source_label}」（{db_kind:?} 数据库）。\n\
             可以用 list_objects 列出有哪些表、describe_table 查看表结构，再用 run_query 执行 SQL 查看数据。\n\
             只读查询（SELECT/EXPLAIN）和 INSERT 会直接执行；UPDATE/DELETE/建表改表删表这类语句会先弹窗\n\
             让用户确认，用户拒绝时如实告诉用户，不要重复尝试同一条语句、也不要擅自改写成别的语句绕过确认。\n\
             一次 run_query 只能传一条语句。回答尽量简洁，用中文，涉及具体数据结论时给出你依据的 SQL。"
        );
        Self {
            id: Uuid::new_v4(),
            data_source_id,
            provider_id,
            todos: Vec::new(),
            messages: vec![json!({ "role": "system", "content": system_prompt })],
        }
    }

    pub fn messages_snapshot(&self) -> Vec<serde_json::Value> {
        self.messages.clone()
    }

    pub fn restore_messages(&mut self, messages: Vec<serde_json::Value>) {
        if !messages.is_empty() {
            self.messages = messages;
        }
    }

    fn limit_context(&mut self, budget: usize) {
        while self.messages.len() > MAX_CONTEXT_MESSAGES
            || agent_llm::estimate_value_tokens(&serde_json::Value::Array(self.messages.clone())) > budget
        {
            let Some(end) = self
                .messages
                .iter()
                .skip(2)
                .position(|m| m["role"] == "user")
                .map(|p| p + 2)
            else {
                break;
            };
            self.messages.drain(1..end);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn send_message(
        &mut self,
        user_text: &str,
        attachments: &[ChatAttachment],
        providers: &AiProviderManager,
        ds_service: &SqlDataSourceService,
        sql_sessions: &SqlSessionManager,
        query_history: &SqlQueryHistoryRepo,
        confirms: &CommandConfirmRegistry,
        question_confirms: &QuestionRegistry,
        app_handle: &AppHandle,
        cancel_token: &tokio_util::sync::CancellationToken,
    ) -> Result<String, AppError> {
        let provider = providers
            .get(self.provider_id)?
            .ok_or_else(|| AppError::NotFound(format!("ai provider not found: {}", self.provider_id)))?;
        let api_key = providers.resolve_api_key(&provider).await?;
        let client = reqwest::Client::new();

        // 附件（PDF/文本文件）太大、直接塞全文会让请求超出上下文预算时，
        // `build_user_message_content` 会自动分窗口提取相关内容代替原文——
        // 和 `coding::session` 共用同一份实现（同一个预算算法
        // `agent_llm::context_budget`，不是这里下面 `budget`/
        // `DEFAULT_SQL_CONTEXT_BUDGET` 那个专门给 `limit_context` 裁剪历史
        // 消息用的、默认值不同的预算——两个预算数字服务不同的目的，不需要
        // 对齐）。
        let content =
            crate::coding::session::build_user_message_content(user_text, attachments, &client, &provider, &api_key, app_handle, self.id)
                .await;
        self.messages.push(json!({ "role": "user", "content": content }));

        let budget = provider
            .context_window_tokens
            .map(|n| n as usize / 10 * 8)
            .unwrap_or(DEFAULT_SQL_CONTEXT_BUDGET);
        self.limit_context(budget);
        let mut turn_usage = agent_llm::TurnUsage::default();
        let session_id = self.id;
        let tools = tools::tool_schema();

        for i in 0..MAX_TOOL_ITERATIONS {
            if cancel_token.is_cancelled() {
                return Err(AppError::Internal("已停止：用户取消了当前对话轮次".to_string()));
            }

            let force_conclude_start = MAX_TOOL_ITERATIONS.saturating_sub(FORCE_CONCLUDE_LAST_N);
            let force_conclude = i >= force_conclude_start;
            if i == force_conclude_start {
                self.messages.push(json!({
                    "role": "system",
                    "content": "你已经调用了很多次工具，收集到的信息应该已经足够。接下来不再提供任何工具，\
                                 请直接基于目前已经了解到的内容给出结论/总结，不要说\"我需要再看看\"这类话。"
                }));
            }

            // Tool output can grow the current round after the initial history trim.
            // Re-apply the message-count/token guard before every provider request.
            self.limit_context(budget);

            let round = match agent_llm::call_llm_once(
                &client,
                &provider,
                &api_key,
                &self.messages,
                (!force_conclude).then_some(&tools),
                app_handle,
                session_id,
                "sqlagent",
                cancel_token,
            )
            .await
            {
                Ok(round) => round,
                Err(agent_llm::LlmCallError::Cancelled) => {
                    return Err(AppError::Internal("已停止：用户取消了当前对话轮次".to_string()));
                }
                Err(agent_llm::LlmCallError::RequestFailed(detail)) => {
                    self.messages.push(agent_llm::request_failed_message(&detail));
                    return Err(AppError::Connection(detail));
                }
                Err(agent_llm::LlmCallError::ParseFailed(e)) => {
                    self.messages.push(agent_llm::parse_failed_message(&e));
                    return Err(AppError::Internal(format!("解析响应失败: {e}")));
                }
            };
            turn_usage.add(round.usage);
            let message = round.message;
            let tool_calls = round.tool_calls;

            if tool_calls.is_empty() {
                let text = message["content"].as_str().unwrap_or("").to_string();
                self.messages.push(json!({ "role": "assistant", "content": text }));
                turn_usage.emit_summary(app_handle, session_id, "sqlagent");
                return Ok(text);
            }

            if let Some(note) = message["content"].as_str() {
                if !note.trim().is_empty() {
                    let _ = app_handle.emit(
                        "sqlagent:assistant-note",
                        json!({ "sessionId": self.id, "text": note, "kind": "model" }),
                    );
                }
            }

            self.messages.push(message);
            for call in &tool_calls {
                let call_id = call["id"].as_str().unwrap_or_default().to_string();
                let fn_name = call["function"]["name"].as_str().unwrap_or_default().to_string();
                let fn_args = call["function"]["arguments"].as_str().unwrap_or("{}");

                let _ = app_handle.emit(
                    "sqlagent:tool-call-start",
                    json!({ "sessionId": self.id, "tool": fn_name, "detail": tool_call_detail(&fn_name, fn_args) }),
                );

                let result_text = agent_llm::cap_tool_result(match tools::parse_tool_call(&fn_name, fn_args) {
                    Ok(call) => self
                        .execute_tool(call, ds_service, sql_sessions, query_history, confirms, question_confirms, app_handle)
                        .await
                        .unwrap_or_else(|e| format!("工具执行出错：{e}")),
                    Err(e) => format!("工具调用参数解析失败：{e}"),
                });

                let _ = app_handle.emit(
                    "sqlagent:tool-call-end",
                    json!({ "sessionId": self.id, "tool": fn_name, "output": result_text }),
                );

                self.messages.push(json!({
                    "role": "tool",
                    "tool_call_id": call_id,
                    "content": result_text,
                }));
            }
        }

        turn_usage.emit_summary(app_handle, session_id, "sqlagent");
        Err(AppError::Internal(format!(
            "这一轮已经调用了 {MAX_TOOL_ITERATIONS} 次工具还没给出最终结论，先停下来避免无限跑下去。\
             之前的进度都还在，直接发\"继续\"就会接着刚才的内容往下做。"
        )))
    }

    #[allow(clippy::too_many_arguments)]
    async fn execute_tool(
        &mut self,
        call: ToolCall,
        ds_service: &SqlDataSourceService,
        sql_sessions: &SqlSessionManager,
        query_history: &SqlQueryHistoryRepo,
        confirms: &CommandConfirmRegistry,
        question_confirms: &QuestionRegistry,
        app_handle: &AppHandle,
    ) -> Result<String, AppError> {
        match call {
            ToolCall::RunQuery { sql, row_limit } => {
                self.run_query(&sql, row_limit, ds_service, sql_sessions, query_history, confirms, app_handle).await
            }
            ToolCall::DescribeTable { schema, name } => self.describe_table(&schema, &name, sql_sessions).await,
            ToolCall::ListObjects { schema } => self.list_objects(schema.as_deref(), sql_sessions).await,
            ToolCall::TodoWrite { todos } => {
                self.todos = todos;
                let _ = app_handle.emit("sqlagent:todo-update", json!({ "sessionId": self.id, "todos": &self.todos }));
                Ok(format!("任务清单已更新，共 {} 项", self.todos.len()))
            }
            ToolCall::Question { question, options } => self.ask_user(&question, options, question_confirms, app_handle).await,
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_query(
        &self,
        sql: &str,
        row_limit: usize,
        ds_service: &SqlDataSourceService,
        sql_sessions: &SqlSessionManager,
        query_history: &SqlQueryHistoryRepo,
        confirms: &CommandConfirmRegistry,
        app_handle: &AppHandle,
    ) -> Result<String, AppError> {
        let profile = ds_service
            .get(self.data_source_id)?
            .ok_or_else(|| AppError::NotFound(format!("data source not found: {}", self.data_source_id)))?;
        let (statement, kind) = policy::parse_and_classify(sql, profile.db_kind)?;

        if profile.readonly && !matches!(statement, Statement::Query(_) | Statement::Explain { .. }) {
            return Ok("该数据源已开启只读模式，拒绝执行写操作，请改用只读查询。".to_string());
        }

        if matches!(kind, ExecutionKind::Confirm | ExecutionKind::Transaction) {
            let (request_id, rx) = confirms.register(self.id).await;
            let _ = app_handle.emit(
                "sqlagent:confirm-request",
                json!({ "sessionId": self.id, "requestId": request_id, "sql": sql }),
            );
            let allowed = rx.await.unwrap_or(false);
            if !allowed {
                return Ok("用户拒绝执行该语句，没有对数据库做任何修改。".to_string());
            }
        }

        let session = sql_sessions.get_or_open(self.data_source_id).await?;
        let result = session.execute_sql(sql, row_limit, new_backend_handle_slot()).await;
        let now = chrono::Utc::now().to_rfc3339();
        match &result {
            Ok(r) => {
                let _ = query_history.record(
                    self.data_source_id,
                    sql,
                    "finished",
                    Some(r.duration_ms as i64),
                    r.rows_affected.map(|n| n as i64).or(Some(r.rows.len() as i64)),
                    None,
                    &now,
                );
            }
            Err(e) => {
                let _ = query_history.record(self.data_source_id, sql, "error", None, None, Some(&e.to_string()), &now);
            }
        }
        Ok(format_result_for_agent(&result?))
    }

    async fn describe_table(&self, schema: &str, name: &str, sql_sessions: &SqlSessionManager) -> Result<String, AppError> {
        let session = sql_sessions.get_or_open(self.data_source_id).await?;
        let object = ObjectRef { schema: schema.to_string(), name: name.to_string(), kind: ObjectKind::Table };
        let def = session.describe_object(&object).await?;
        Ok(format_definition_for_agent(&def))
    }

    async fn list_objects(&self, schema: Option<&str>, sql_sessions: &SqlSessionManager) -> Result<String, AppError> {
        let session = sql_sessions.get_or_open(self.data_source_id).await?;
        let mut objects = session.list_objects().await?;
        if let Some(schema) = schema {
            objects.retain(|o| o.schema.eq_ignore_ascii_case(schema));
        }
        if objects.is_empty() {
            return Ok("没有找到匹配的对象".to_string());
        }
        Ok(objects
            .iter()
            .take(300)
            .map(|o| format!("{}.{} ({:?})", o.schema, o.name, o.kind))
            .collect::<Vec<_>>()
            .join("\n"))
    }

    async fn ask_user(
        &self,
        question: &str,
        options: Vec<String>,
        question_confirms: &QuestionRegistry,
        app_handle: &AppHandle,
    ) -> Result<String, AppError> {
        let (request_id, rx) = question_confirms.register().await;
        let _ = app_handle.emit(
            "sqlagent:question-request",
            json!({ "sessionId": self.id, "requestId": request_id, "question": question, "options": options }),
        );
        Ok(rx.await.unwrap_or_default())
    }
}

fn format_result_for_agent(result: &ExecuteResult) -> String {
    if result.columns.is_empty() {
        return format!("执行成功，受影响 {} 行，耗时 {}ms", result.rows_affected.unwrap_or(0), result.duration_ms);
    }
    let mut out = format!(
        "耗时 {}ms，返回 {} 行{}\n",
        result.duration_ms,
        result.rows.len(),
        if result.truncated { "（已截断，更多数据请缩小范围/加 LIMIT 重新查询）" } else { "" }
    );
    out.push_str(&result.columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>().join(" | "));
    out.push('\n');
    for row in &result.rows {
        out.push_str(
            &row.iter()
                .map(|c| if c.is_null { "NULL".to_string() } else { c.text.clone() })
                .collect::<Vec<_>>()
                .join(" | "),
        );
        out.push('\n');
    }
    out
}

fn format_definition_for_agent(def: &ObjectDefinition) -> String {
    let mut out = format!("{}.{}", def.object.schema, def.object.name);
    if let Some(c) = &def.comment {
        out.push_str(&format!("  -- {c}"));
    }
    out.push('\n');
    for col in &def.columns {
        out.push_str(&format!(
            "- {} {}{}{}{}\n",
            col.name,
            col.data_type,
            if col.nullable { "" } else { " NOT NULL" },
            if col.is_primary_key { " PRIMARY KEY" } else { "" },
            col.comment.as_deref().map(|c| format!("  -- {c}")).unwrap_or_default()
        ));
    }
    for idx in &def.indexes {
        out.push_str(&format!("INDEX {}: {}\n", idx.name, idx.definition));
    }
    out
}

/// 和 `coding::session::tool_call_detail` 同样的用途——只显示工具名之前完全
/// 看不出模型在反复对同一张表/同一条查询调用，还是在正常探索不同的表。
fn tool_call_detail(fn_name: &str, fn_args: &str) -> Option<String> {
    let args: serde_json::Value = serde_json::from_str(fn_args).ok()?;
    match fn_name {
        "run_query" => args["sql"].as_str().map(|s| s.chars().take(80).collect()),
        "describe_table" => Some(format!("{}.{}", args["schema"].as_str().unwrap_or(""), args["name"].as_str().unwrap_or(""))),
        "list_objects" => args["schema"].as_str().map(|s| s.to_string()),
        _ => None,
    }
}
