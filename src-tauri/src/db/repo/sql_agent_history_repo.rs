use chrono::Utc;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::db::DbPool;
use crate::error::AppError;

/// SQL Agent 会话持久化——结构上是 `coding_history` 的简化版（少了
/// `mode`/`changes_json`：SQL Agent 没有 Plan/Build 模式切换，写操作的
/// 确认门禁按每条语句单独判断，见 `sql::agent::session`；也没有文件改动，
/// 不需要 `ChangeStore` 快照）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SqlAgentHistoryInput {
    pub id: Uuid,
    pub data_source_id: Uuid,
    pub title: String,
    pub provider_id: Uuid,
    pub provider_label: String,
    pub model: String,
    pub timeline: serde_json::Value,
    /// 发给 AI 的真实对话上下文——前端不需要填，`sql_agent_history_save` 落库前
    /// 用当前存活会话的最新 messages 覆盖。
    #[serde(default)]
    pub messages: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct SqlAgentHistorySummary {
    pub id: Uuid,
    pub title: String,
    pub provider_id: Uuid,
    pub provider_label: String,
    pub model: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SqlAgentHistoryDetail {
    #[serde(flatten)]
    pub summary: SqlAgentHistorySummary,
    pub data_source_id: Uuid,
    pub timeline: serde_json::Value,
    /// 不通过 IPC 序列化给前端，只在后端 `sql_agent_history_resume` 内部用来
    /// 重建会话上下文。
    #[serde(skip)]
    pub messages: serde_json::Value,
}

pub struct SqlAgentHistoryRepo {
    pool: DbPool,
}

impl SqlAgentHistoryRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub fn save(&self, input: &SqlAgentHistoryInput) -> Result<(), AppError> {
        let conn = self.pool.get()?;
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO sql_agent_history (id, data_source_id, title, provider_id, provider_label, model, timeline_json, messages_json, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)
             ON CONFLICT(id) DO UPDATE SET title=excluded.title, provider_id=excluded.provider_id,
             provider_label=excluded.provider_label, model=excluded.model,
             timeline_json=excluded.timeline_json, messages_json=excluded.messages_json,
             updated_at=excluded.updated_at",
            params![
                input.id.to_string(),
                input.data_source_id.to_string(),
                input.title,
                input.provider_id.to_string(),
                input.provider_label,
                input.model,
                input.timeline.to_string(),
                input.messages.to_string(),
                now,
            ],
        )?;
        Ok(())
    }

    pub fn list(&self, data_source_id: Uuid) -> Result<Vec<SqlAgentHistorySummary>, AppError> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT id, title, provider_id, provider_label, model, created_at, updated_at
             FROM sql_agent_history WHERE data_source_id=?1 ORDER BY updated_at DESC",
        )?;
        let rows = stmt
            .query_map([data_source_id.to_string()], |r| {
                Ok(SqlAgentHistorySummary {
                    id: Uuid::parse_str(&r.get::<_, String>(0)?).unwrap_or_default(),
                    title: r.get(1)?,
                    provider_id: Uuid::parse_str(&r.get::<_, String>(2)?).unwrap_or_default(),
                    provider_label: r.get(3)?,
                    model: r.get(4)?,
                    created_at: r.get(5)?,
                    updated_at: r.get(6)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn get(&self, id: Uuid) -> Result<Option<SqlAgentHistoryDetail>, AppError> {
        let conn = self.pool.get()?;
        conn.query_row(
            "SELECT data_source_id, title, provider_id, provider_label, model, timeline_json, created_at, updated_at, messages_json
             FROM sql_agent_history WHERE id=?1",
            [id.to_string()],
            |r| {
                let timeline: String = r.get(5)?;
                let messages: String = r.get(8)?;
                Ok(SqlAgentHistoryDetail {
                    summary: SqlAgentHistorySummary {
                        id,
                        title: r.get(1)?,
                        provider_id: Uuid::parse_str(&r.get::<_, String>(2)?).unwrap_or_default(),
                        provider_label: r.get(3)?,
                        model: r.get(4)?,
                        created_at: r.get(6)?,
                        updated_at: r.get(7)?,
                    },
                    data_source_id: Uuid::parse_str(&r.get::<_, String>(0)?).unwrap_or_default(),
                    timeline: serde_json::from_str(&timeline).unwrap_or_default(),
                    messages: serde_json::from_str(&messages).unwrap_or_default(),
                })
            },
        )
        .optional()
        .map_err(AppError::from)
    }

    pub fn rename(&self, id: Uuid, title: &str) -> Result<(), AppError> {
        self.pool.get()?.execute(
            "UPDATE sql_agent_history SET title=?2, updated_at=?3 WHERE id=?1",
            params![id.to_string(), title, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    pub fn delete(&self, id: Uuid) -> Result<(), AppError> {
        self.pool
            .get()?
            .execute("DELETE FROM sql_agent_history WHERE id=?1", [id.to_string()])?;
        Ok(())
    }
}
