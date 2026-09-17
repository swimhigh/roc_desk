use std::sync::Arc;

use uuid::Uuid;

use crate::ai::chat::AiChatClient;
use crate::ai::providers::{AiProvider, AiProviderManager};
use crate::error::AppError;

/// SQL 桌面 AI 面板（docs/SQL_DESKTOP_PLAN.md §8）。
///
/// 范围说明：这一版只做"单次请求 -> 改写建议"，不是像"AI 编程助手"那样的
/// 多轮工具调用 agent（`sql_run_query`/`sql_describe_object` 之类可供 AI
/// 自主调用的工具尚未接入，仍是后续工作）。生成的 SQL 由调用方
/// （`commands::sql`）通过 `ChangeStore::stage` 转成一条 Pending 的文件改动，
/// 走和"人手改 SQL"完全相同的 Diff/Accept/Reject/Undo 流程（方案 §4.4、§8），
/// 不会绕过确认直接覆盖编辑器内容。
pub struct SqlAiAssistant {
    chat_client: Arc<AiChatClient>,
    provider_manager: Arc<AiProviderManager>,
}

const SYSTEM_PROMPT: &str = "你是嵌入在 roc_desk SQL 桌面模块里的 SQL 助手。\
只输出可以直接替换编辑器内容的完整 SQL 语句本身，禁止使用 Markdown 代码块围栏，\
禁止输出任何解释性文字、前后缀说明——你的整个回复都会被原样写回 SQL 编辑器。";

fn strip_markdown_fence(text: &str) -> String {
    let trimmed = text.trim();
    let Some(rest) = trimmed.strip_prefix("```") else {
        return trimmed.to_string();
    };
    let rest = rest.strip_prefix("sql").unwrap_or(rest);
    let rest = rest.trim_start_matches('\n');
    match rest.rfind("```") {
        Some(end) => rest[..end].trim().to_string(),
        None => rest.trim().to_string(),
    }
}

impl SqlAiAssistant {
    pub fn new(chat_client: Arc<AiChatClient>, provider_manager: Arc<AiProviderManager>) -> Self {
        Self {
            chat_client,
            provider_manager,
        }
    }

    async fn resolve_provider(&self, provider_id: Uuid) -> Result<(AiProvider, Option<String>), AppError> {
        let provider = self
            .provider_manager
            .get(provider_id)?
            .ok_or_else(|| AppError::NotFound(format!("AI provider not found: {provider_id}")))?;
        let api_key = self.provider_manager.resolve_api_key(&provider).await?;
        Ok((provider, api_key))
    }

    async fn ask(&self, provider_id: Uuid, user_text: &str) -> Result<String, AppError> {
        let (provider, api_key) = self.resolve_provider(provider_id).await?;
        let raw = self
            .chat_client
            .complete_once(&provider, api_key.as_deref(), SYSTEM_PROMPT, user_text)
            .await?;
        Ok(strip_markdown_fence(&raw))
    }

    /// 根据自然语言描述和可选的表结构上下文生成 SQL（方案 §8 第 1 项）。
    pub async fn generate_sql(
        &self,
        provider_id: Uuid,
        instruction: &str,
        schema_context: &str,
        current_sql: &str,
    ) -> Result<String, AppError> {
        let user = format!(
            "当前 SQL 编辑器内容（可能为空）：\n```sql\n{current_sql}\n```\n\n\
             可用表结构（DDL，仅供参考，不代表要全部用到）：\n{schema_context}\n\n\
             用户需求：{instruction}"
        );
        self.ask(provider_id, &user).await
    }

    /// 解释语义、索引使用和潜在风险（方案 §8 第 2 项）——返回的是给人看的说明
    /// 文字，不是替换编辑器内容的 SQL，调用方不应该把结果传给 `ChangeStore`。
    pub async fn explain_sql(&self, provider_id: Uuid, sql: &str, schema_context: &str) -> Result<String, AppError> {
        let (provider, api_key) = self.resolve_provider(provider_id).await?;
        let system = "你是嵌入在 roc_desk SQL 桌面模块里的 SQL 助手，现在的任务是解释一条 SQL 语句：\
说明它的语义、可能用到的索引、潜在的性能或数据风险，用简体中文分点作答。";
        let user = format!("表结构（DDL，供参考）：\n{schema_context}\n\nSQL：\n```sql\n{sql}\n```");
        self.chat_client
            .complete_once(&provider, api_key.as_deref(), system, &user)
            .await
    }

    /// 结合执行计划提出优化建议、生成可对比的改写版本（方案 §8 第 3 项）。
    pub async fn optimize_sql(
        &self,
        provider_id: Uuid,
        sql: &str,
        explain_output: Option<&str>,
        schema_context: &str,
    ) -> Result<String, AppError> {
        let plan_section = match explain_output {
            Some(plan) => format!("\n\n执行计划：\n{plan}"),
            None => String::new(),
        };
        let user = format!(
            "表结构（DDL，供参考）：\n{schema_context}\n\n需要优化的 SQL：\n```sql\n{sql}\n```{plan_section}\n\n\
             请给出优化后的等价 SQL。"
        );
        self.ask(provider_id, &user).await
    }

    /// 把数据库报错和原 SQL 一起发给模型，要求给出修复后的版本（方案 §8 第 4 项）。
    pub async fn fix_error(
        &self,
        provider_id: Uuid,
        sql: &str,
        error_message: &str,
        schema_context: &str,
    ) -> Result<String, AppError> {
        let user = format!(
            "表结构（DDL，供参考）：\n{schema_context}\n\n执行失败的 SQL：\n```sql\n{sql}\n```\n\n\
             数据库返回的错误：\n{error_message}\n\n请给出修复后的 SQL。"
        );
        self.ask(provider_id, &user).await
    }
}
