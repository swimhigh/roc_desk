use chrono::Utc;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::db::DbPool;
use crate::error::AppError;

/// 2026-10 用户明确要求："AI 工具的会话历史/消息记录只存在工作区自己的
/// `.rock_desk` 子目录里，不要存在 roc_desk.exe 所在目录的全局库"——此前的
/// 设计是把 `timeline_json`/`changes_json`/`messages_json` 这几份可能长到
/// 几十上百 MB 的内容完整镜像进本地全局 `coding_history` 表（最初是为了让
/// "列历史"能脱离网络快速展示），实际后果是全局 `roc_desk.db` 被撑到
/// 598MB（哪怕只有十几个工作区）——压缩（gzip）能把单份文件压小，但治标
/// 不治本：内容本来就不该有两份。
///
/// 现在这张表只保留"摘要"——标题/Provider/模型/模式/时间戳，用来让"打开
/// 历史列表"不用每次都读一遍工作区目录（尤其是远程工作区，列目录本身要走
/// 网络）。真正的内容（`timeline`/`changes`/`messages`）只有一份，就在
/// `WorkspaceHistorySnapshot` 写出的那个工作区目录文件里，点开某一条历史
/// 记录时才去读（本地工作区是本地磁盘读，远程工作区走 SFTP/Agent 协议，
/// 见 `commands::coding::load_history_snapshot`）——用户认可"点开某条历史
/// 可能要多等一下"这个代价换"本地不再无限累积这些内容"。
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
    /// 存活会话的最新 `messages` 覆盖它。这几份内容只写进工作区镜像文件，不进
    /// 这张表（见上面的模块文档）。
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
    /// 完整详情（会跨网络读一份可能上兆的快照）才能知道，那正是"打开 AI 工具
    /// 要等十几秒"的一段耗时。
    pub provider_id: Uuid,
    pub provider_label: String,
    pub model: String,
    pub mode: String,
    pub created_at: String,
    pub updated_at: String,
}

/// 根据历史记录 id 反查"它属于哪个工作区"——点开一条历史时，只有知道
/// workspace_id 才能去对应工作区目录下读 `.rock_desk/sessions/{id}.json`
/// 这份真正的内容。`summary` 一并带出，内容不可达（工作区断连）时前端至少
/// 还能显示标题/Provider 等摘要信息，而不是完全空白。
#[derive(Debug, Clone, Serialize)]
pub struct CodingHistoryLocation {
    pub workspace_id: Uuid,
    pub summary: CodingHistorySummary,
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

    /// 只落摘要字段——`timeline_json`/`changes_json`/`messages_json` 三列是
    /// 这张表里的历史包袱（建表时就是 `NOT NULL`，拿掉列本身要动迁移，犯不上
    /// 为了三个永远写空字符串的列专门改 schema），统一写 `''`/`'[]'` 占位。
    pub fn save(&self, input: &CodingHistoryInput) -> Result<(), AppError> {
        let conn = self.pool.get()?;
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO coding_history (id, workspace_id, title, provider_id, provider_label, model, mode, timeline_json, changes_json, messages_json, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, '', '', '[]', ?8, ?8)
             ON CONFLICT(id) DO UPDATE SET title=excluded.title, provider_id=excluded.provider_id,
             provider_label=excluded.provider_label, model=excluded.model, mode=excluded.mode,
             updated_at=excluded.updated_at",
            params![input.id.to_string(), input.workspace_id.to_string(), input.title, input.provider_id.to_string(),
                input.provider_label, input.model, input.mode, now],
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

    /// 轻量查找——只读摘要列，不碰（已经常年是空字符串的）内容列。调用方
    /// 用返回的 `workspace_id` 去对应工作区目录读真正的内容文件。
    pub fn get_location(&self, id: Uuid) -> Result<Option<CodingHistoryLocation>, AppError> {
        let conn = self.pool.get()?;
        conn.query_row(
            "SELECT workspace_id, title, provider_id, provider_label, model, mode, created_at, updated_at FROM coding_history WHERE id=?1",
            [id.to_string()],
            |r| {
                let parse_uuid = |s: String| Uuid::parse_str(&s).unwrap_or_default();
                Ok(CodingHistoryLocation {
                    workspace_id: parse_uuid(r.get(0)?),
                    summary: CodingHistorySummary {
                        id,
                        title: r.get(1)?,
                        provider_id: parse_uuid(r.get(2)?),
                        provider_label: r.get(3)?,
                        model: r.get(4)?,
                        mode: r.get(5)?,
                        created_at: r.get(6)?,
                        updated_at: r.get(7)?,
                    },
                })
            },
        )
        .optional()
        .map_err(AppError::from)
    }

    /// 用工作区目录里读到的快照对账本地摘要缓存——只更新标题/Provider/模型/
    /// 模式/时间戳这些摘要字段，快照里的 `timeline`/`changes`/`messages`
    /// 用完即弃，不写进这张表（见模块文档）。`updated_at` 更旧的快照不覆盖
    /// 本地已有的摘要，避免网络抖动/并发写导致"列表显示的标题又被旧内容
    /// 冲回去"。
    pub fn import_snapshot(&self, snapshot: &WorkspaceHistorySnapshot) -> Result<(), AppError> {
        let input = &snapshot.input;
        self.pool.get()?.execute(
            "INSERT INTO coding_history (id, workspace_id, title, provider_id, provider_label, model, mode, timeline_json, changes_json, messages_json, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, '', '', '[]', ?8, ?9)
             ON CONFLICT(id) DO UPDATE SET title=excluded.title, provider_id=excluded.provider_id,
             provider_label=excluded.provider_label, model=excluded.model, mode=excluded.mode,
             updated_at=excluded.updated_at WHERE excluded.updated_at > coding_history.updated_at",
            params![input.id.to_string(), input.workspace_id.to_string(), input.title, input.provider_id.to_string(),
                input.provider_label, input.model, input.mode, snapshot.created_at, snapshot.updated_at],
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
