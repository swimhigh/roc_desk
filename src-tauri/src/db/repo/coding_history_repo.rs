use chrono::Utc;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::db::DbPool;
use crate::error::AppError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodingHistoryInput {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub title: String,
    pub provider_id: Uuid,
    pub provider_label: String,
    pub model: String,
    pub mode: String,
    pub timeline: serde_json::Value,
    pub changes: serde_json::Value,
    /// 发给 AI 的真实对话上下文（`CodingSession` 内部的 `messages`）——前端不知道
    /// 也不需要填这个字段（`#[serde(default)]`：前端传来的 JSON 里没有这个键也能
    /// 反序列化成功），`commands::coding::coding_history_save` 在落库前会用当前
    /// 存活会话的最新 `messages` 覆盖它。`coding_history_resume` 靠这份数据把
    /// 历史会话真正续上，而不只是回放 `changes`/`timeline`。
    #[serde(default)]
    pub messages: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct CodingHistorySummary {
    pub id: Uuid,
    pub title: String,
    /// 摘要里也带上 provider id（不只是 `provider_label`）——前端
    /// `restoreOrStart` 需要在"只查本地缓存列表、还没去远程工作区对账"的阶段就
    /// 知道最近一条历史用的是哪个 Provider，好用它直接起会话；否则就得先拉一遍
    /// 完整详情（`coding_history_get`，会跨网络读一份可能上兆的快照）才能知道，
    /// 那正是"打开 AI 工具要等十几秒"的一段耗时。
    pub provider_id: Uuid,
    pub provider_label: String,
    pub model: String,
    pub mode: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CodingHistoryDetail {
    #[serde(flatten)]
    pub summary: CodingHistorySummary,
    pub workspace_id: Uuid,
    pub timeline: serde_json::Value,
    pub changes: serde_json::Value,
    /// 不通过 IPC 序列化给前端（前端没有必要、也不应该直接看到原始 LLM 消息
    /// 上下文）——只在后端内部 `coding_history_resume` 里读取用来重建会话。
    #[serde(skip)]
    pub messages: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceHistorySnapshot {
    pub input: CodingHistoryInput,
    pub created_at: String,
    pub updated_at: String,
}

pub struct CodingHistoryRepo {
    pool: DbPool,
}

impl CodingHistoryRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub fn save(&self, input: &CodingHistoryInput) -> Result<(), AppError> {
        let conn = self.pool.get()?;
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO coding_history (id, workspace_id, title, provider_id, provider_label, model, mode, timeline_json, changes_json, messages_json, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11)
             ON CONFLICT(id) DO UPDATE SET title=excluded.title, provider_id=excluded.provider_id,
             provider_label=excluded.provider_label, model=excluded.model, mode=excluded.mode,
             timeline_json=excluded.timeline_json, changes_json=excluded.changes_json,
             messages_json=excluded.messages_json, updated_at=excluded.updated_at",
            params![input.id.to_string(), input.workspace_id.to_string(), input.title, input.provider_id.to_string(),
                input.provider_label, input.model, input.mode, input.timeline.to_string(), input.changes.to_string(),
                input.messages.to_string(), now],
        )?;
        Ok(())
    }

    pub fn list(&self, workspace_id: Uuid) -> Result<Vec<CodingHistorySummary>, AppError> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare("SELECT id, title, provider_id, provider_label, model, mode, created_at, updated_at FROM coding_history WHERE workspace_id=?1 ORDER BY updated_at DESC")?;
        let rows = stmt
            .query_map([workspace_id.to_string()], |r| {
                Ok(CodingHistorySummary {
                    id: Uuid::parse_str(&r.get::<_, String>(0)?).unwrap_or_default(),
                    title: r.get(1)?,
                    provider_id: Uuid::parse_str(&r.get::<_, String>(2)?).unwrap_or_default(),
                    provider_label: r.get(3)?,
                    model: r.get(4)?,
                    mode: r.get(5)?,
                    created_at: r.get(6)?,
                    updated_at: r.get(7)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn get(&self, id: Uuid) -> Result<Option<CodingHistoryDetail>, AppError> {
        let conn = self.pool.get()?;
        conn.query_row("SELECT workspace_id, title, provider_id, provider_label, model, mode, timeline_json, changes_json, created_at, updated_at, messages_json FROM coding_history WHERE id=?1", [id.to_string()], |r| {
            let parse_uuid = |s: String| Uuid::parse_str(&s).unwrap_or_default();
            let timeline: String = r.get(6)?;
            let changes: String = r.get(7)?;
            let messages: String = r.get(10)?;
            Ok(CodingHistoryDetail {
                summary: CodingHistorySummary { id, title: r.get(1)?, provider_id: parse_uuid(r.get(2)?), provider_label: r.get(3)?, model: r.get(4)?, mode: r.get(5)?, created_at: r.get(8)?, updated_at: r.get(9)? },
                workspace_id: parse_uuid(r.get(0)?),
                timeline: serde_json::from_str(&timeline).unwrap_or_default(), changes: serde_json::from_str(&changes).unwrap_or_default(),
                messages: serde_json::from_str(&messages).unwrap_or_default(),
            })
        }).optional().map_err(AppError::from)
    }

    pub fn import_snapshot(&self, snapshot: &WorkspaceHistorySnapshot) -> Result<(), AppError> {
        let input = &snapshot.input;
        self.pool.get()?.execute(
            "INSERT INTO coding_history (id, workspace_id, title, provider_id, provider_label, model, mode, timeline_json, changes_json, messages_json, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT(id) DO UPDATE SET title=excluded.title, provider_id=excluded.provider_id,
             provider_label=excluded.provider_label, model=excluded.model, mode=excluded.mode,
             timeline_json=excluded.timeline_json, changes_json=excluded.changes_json,
             messages_json=excluded.messages_json,
             updated_at=excluded.updated_at WHERE excluded.updated_at > coding_history.updated_at",
            params![input.id.to_string(), input.workspace_id.to_string(), input.title, input.provider_id.to_string(),
                input.provider_label, input.model, input.mode, input.timeline.to_string(), input.changes.to_string(),
                input.messages.to_string(), snapshot.created_at, snapshot.updated_at],
        )?;
        Ok(())
    }

    pub fn rename(&self, id: Uuid, title: &str) -> Result<(), AppError> {
        self.pool.get()?.execute(
            "UPDATE coding_history SET title=?2, updated_at=?3 WHERE id=?1",
            params![id.to_string(), title, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    pub fn delete(&self, id: Uuid) -> Result<(), AppError> {
        self.pool
            .get()?
            .execute("DELETE FROM coding_history WHERE id=?1", [id.to_string()])?;
        Ok(())
    }
}
