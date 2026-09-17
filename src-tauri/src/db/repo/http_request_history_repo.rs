use rusqlite::{params, OptionalExtension};

use crate::db::DbPool;
use crate::error::AppError;
use crate::http_desk::model::{HttpRequestHistoryDetail, HttpRequestHistoryEntry};

/// 和 `HttpWorkspaceTabsRepo` 一样连接 `workspaces_pool`（不是主库），理由同上
/// （FK 引用 `workspaces(id)`，见 docs/HTTP_DESKTOP_PLAN.md §5）。
pub struct HttpRequestHistoryRepo {
    pool: DbPool,
}

/// `create` 的入参——快照文本（请求/响应完整 JSON）只在写入时经过这里，读取列表
/// 时不带出来，见 `HttpRequestHistoryDetail` 的注释。
pub struct NewHistoryEntry<'a> {
    pub id: String,
    pub workspace_id: String,
    pub collection_slug: String,
    pub request_id: Option<String>,
    pub environment_id: Option<String>,
    pub method: String,
    pub url: String,
    pub status_code: Option<i64>,
    pub duration_ms: Option<i64>,
    pub response_size_bytes: Option<i64>,
    pub error_message: Option<String>,
    pub request_snapshot: &'a str,
    pub response_snapshot: Option<&'a str>,
    pub created_at: String,
}

impl HttpRequestHistoryRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub fn create(&self, entry: &NewHistoryEntry) -> Result<(), AppError> {
        let conn = self.pool.get()?;
        conn.execute(
            "INSERT INTO http_request_history
                (id, workspace_id, collection_slug, request_id, environment_id, method, url,
                 status_code, duration_ms, response_size_bytes, error_message,
                 request_snapshot, response_snapshot, created_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
            params![
                entry.id,
                entry.workspace_id,
                entry.collection_slug,
                entry.request_id,
                entry.environment_id,
                entry.method,
                entry.url,
                entry.status_code,
                entry.duration_ms,
                entry.response_size_bytes,
                entry.error_message,
                entry.request_snapshot,
                entry.response_snapshot,
                entry.created_at,
            ],
        )?;
        Ok(())
    }

    pub fn list_for_workspace(
        &self,
        workspace_id: &str,
        limit: i64,
    ) -> Result<Vec<HttpRequestHistoryEntry>, AppError> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT id, workspace_id, collection_slug, request_id, environment_id, method, url,
                    status_code, duration_ms, response_size_bytes, error_message, created_at
             FROM http_request_history WHERE workspace_id = ?1
             ORDER BY created_at DESC LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![workspace_id, limit], Self::map_summary_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn get_detail(&self, id: &str) -> Result<Option<HttpRequestHistoryDetail>, AppError> {
        let conn = self.pool.get()?;
        conn.query_row(
            "SELECT id, request_snapshot, response_snapshot FROM http_request_history WHERE id = ?1",
            params![id],
            |row| {
                Ok(HttpRequestHistoryDetail {
                    id: row.get(0)?,
                    request_snapshot: row.get(1)?,
                    response_snapshot: row.get(2)?,
                })
            },
        )
        .optional()
        .map_err(AppError::from)
    }

    pub fn delete(&self, id: &str) -> Result<(), AppError> {
        let conn = self.pool.get()?;
        conn.execute("DELETE FROM http_request_history WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn clear_for_workspace(&self, workspace_id: &str) -> Result<(), AppError> {
        let conn = self.pool.get()?;
        conn.execute(
            "DELETE FROM http_request_history WHERE workspace_id = ?1",
            params![workspace_id],
        )?;
        Ok(())
    }

    fn map_summary_row(row: &rusqlite::Row) -> rusqlite::Result<HttpRequestHistoryEntry> {
        Ok(HttpRequestHistoryEntry {
            id: row.get(0)?,
            workspace_id: row.get(1)?,
            collection_slug: row.get(2)?,
            request_id: row.get(3)?,
            environment_id: row.get(4)?,
            method: row.get(5)?,
            url: row.get(6)?,
            status_code: row.get(7)?,
            duration_ms: row.get(8)?,
            response_size_bytes: row.get(9)?,
            error_message: row.get(10)?,
            created_at: row.get(11)?,
        })
    }
}
