use chrono::Utc;
use rusqlite::{params, OptionalExtension};
use uuid::Uuid;

use crate::db::DbPool;
use crate::error::AppError;
use crate::workspace::profile::{WorkspaceKind, WorkspaceProfile};

pub struct WorkspaceRepo {
    pool: DbPool,
}

impl WorkspaceRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub fn upsert(&self, profile: &WorkspaceProfile) -> Result<(), AppError> {
        let conn = self.pool.get()?;
        // 故意不在 ON CONFLICT 的 UPDATE SET 里包含 last_sftp_local_path/
        // last_sftp_remote_path——`upsert` 是"打开工作区"时调的（`open_local`/
        // `open_remote`），新建的 `WorkspaceProfile` 字面量里这两个字段总是 None，
        // 如果也跟着更新会把已经记住的目录每次打开工作区都冲掉。这两个字段只应该由
        // `update_last_sftp_paths` 改，`upsert` 对已存在的行完全不碰它们。
        conn.execute(
            "INSERT INTO workspaces (id, kind, root_path, connection_id, display_name, last_opened_at, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET
                root_path = excluded.root_path,
                display_name = excluded.display_name,
                last_opened_at = excluded.last_opened_at",
            params![
                profile.id.to_string(),
                profile.kind.as_str(),
                profile.root_path,
                profile.connection_id.map(|id| id.to_string()),
                profile.display_name,
                profile.last_opened_at,
                Utc::now().to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    pub fn touch_last_opened(&self, id: Uuid) -> Result<(), AppError> {
        let conn = self.pool.get()?;
        conn.execute(
            "UPDATE workspaces SET last_opened_at = ?1 WHERE id = ?2",
            params![Utc::now().to_rfc3339(), id.to_string()],
        )?;
        Ok(())
    }

    /// SFTP/Agent 双栏浏览器每次导航都会调一次（见 `commands::workspace::
    /// workspace_update_last_sftp_paths`）——两个参数都是"当前值"，不是增量，
    /// 一次性整体覆盖，调用方负责传完整的一对。
    pub fn update_last_sftp_paths(&self, id: Uuid, local_path: &str, remote_path: &str) -> Result<(), AppError> {
        let conn = self.pool.get()?;
        conn.execute(
            "UPDATE workspaces SET last_sftp_local_path = ?1, last_sftp_remote_path = ?2 WHERE id = ?3",
            params![local_path, remote_path, id.to_string()],
        )?;
        Ok(())
    }

    pub fn find_by_local_path(&self, root_path: &str) -> Result<Option<WorkspaceProfile>, AppError> {
        let conn = self.pool.get()?;
        let result = conn
            .query_row(
                "SELECT id, kind, root_path, connection_id, display_name, last_opened_at, last_sftp_local_path, last_sftp_remote_path
                 FROM workspaces WHERE kind = 'local' AND root_path = ?1",
                params![root_path],
                Self::map_row,
            )
            .optional()?;
        Ok(result)
    }

    /// 和 `find_by_local_path` 对称——远程工作区判"是否已经打开过"要看连接档案 +
    /// 远程目录这两个字段的组合，不能只看 `root_path`（同一个远程路径字符串在
    /// 不同主机上完全是两回事）。之前 `open_remote` 里漏了这一步，导致每次重新打开
    /// 同一个远程工作区都新建一条记录，"最近工作区"列表里同一个目录会不断堆积
    /// 重复项（真实 bug，2026-08-18 用户报告同一目录出现 4 条记录）。
    pub fn find_by_remote(&self, connection_id: Uuid, root_path: &str) -> Result<Option<WorkspaceProfile>, AppError> {
        let conn = self.pool.get()?;
        let result = conn
            .query_row(
                "SELECT id, kind, root_path, connection_id, display_name, last_opened_at, last_sftp_local_path, last_sftp_remote_path
                 FROM workspaces WHERE kind = 'remote' AND connection_id = ?1 AND root_path = ?2",
                params![connection_id.to_string(), root_path],
                Self::map_row,
            )
            .optional()?;
        Ok(result)
    }

    pub fn find_by_id(&self, id: Uuid) -> Result<Option<WorkspaceProfile>, AppError> {
        let conn = self.pool.get()?;
        let result = conn
            .query_row(
                "SELECT id, kind, root_path, connection_id, display_name, last_opened_at, last_sftp_local_path, last_sftp_remote_path
                 FROM workspaces WHERE id = ?1",
                params![id.to_string()],
                Self::map_row,
            )
            .optional()?;
        Ok(result)
    }

    pub fn list_recent(&self, limit: usize) -> Result<Vec<WorkspaceProfile>, AppError> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT id, kind, root_path, connection_id, display_name, last_opened_at, last_sftp_local_path, last_sftp_remote_path
             FROM workspaces
             ORDER BY last_opened_at DESC
             LIMIT ?1",
        )?;
        let rows = stmt
            .query_map(params![limit as i64], Self::map_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn remove(&self, id: Uuid) -> Result<(), AppError> {
        let conn = self.pool.get()?;
        conn.execute("DELETE FROM workspaces WHERE id = ?1", params![id.to_string()])?;
        Ok(())
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
