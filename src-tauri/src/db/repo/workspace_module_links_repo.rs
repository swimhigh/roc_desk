use chrono::Utc;
use rusqlite::params;
use uuid::Uuid;

use crate::db::DbPool;
use crate::error::AppError;
use crate::workspace::profile::{WorkspaceKind, WorkspaceProfile};

/// 工作模块（"workspace"/"http"）各自维护一份"这个工作区在不在我的首页/选择页
/// 列表里"的关联表——2026-09 用户反馈：HTTP 桌面不应该默认展示所有工作区目录，
/// 需要显式"添加"（选择已有的或新建）；反过来从 HTTP 桌面新建的工作区目录也不该
/// 自动出现在"工作区"卡片。`workspaces` 表本身保持模块无关，这张表只管"哪个
/// 模块看得到它"。
pub struct WorkspaceModuleLinksRepo {
    pool: DbPool,
}

impl WorkspaceModuleLinksRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub fn add(&self, workspace_id: Uuid, module: &str) -> Result<(), AppError> {
        let conn = self.pool.get()?;
        conn.execute(
            "INSERT OR IGNORE INTO workspace_module_links (workspace_id, module, added_at) VALUES (?1, ?2, ?3)",
            params![workspace_id.to_string(), module, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    pub fn remove(&self, workspace_id: Uuid, module: &str) -> Result<(), AppError> {
        let conn = self.pool.get()?;
        conn.execute(
            "DELETE FROM workspace_module_links WHERE workspace_id = ?1 AND module = ?2",
            params![workspace_id.to_string(), module],
        )?;
        Ok(())
    }

    pub fn list_for_module(&self, module: &str, limit: usize) -> Result<Vec<WorkspaceProfile>, AppError> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT w.id, w.kind, w.root_path, w.connection_id, w.display_name, w.last_opened_at,
                    w.last_sftp_local_path, w.last_sftp_remote_path
             FROM workspaces w
             JOIN workspace_module_links l ON l.workspace_id = w.id
             WHERE l.module = ?1
             ORDER BY l.added_at DESC
             LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![module, limit as i64], Self::map_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    fn map_row(row: &rusqlite::Row) -> rusqlite::Result<WorkspaceProfile> {
        let id: String = row.get(0)?;
        let kind: String = row.get(1)?;
        let connection_id: Option<String> = row.get(3)?;
        Ok(WorkspaceProfile {
            id: Uuid::parse_str(&id).unwrap_or_else(|_| Uuid::nil()),
            kind: WorkspaceKind::from_str(&kind),
            root_path: row.get(2)?,
            connection_id: connection_id.and_then(|s| Uuid::parse_str(&s).ok()),
            display_name: row.get(4)?,
            last_opened_at: row.get(5)?,
            last_sftp_local_path: row.get(6)?,
            last_sftp_remote_path: row.get(7)?,
        })
    }
}
