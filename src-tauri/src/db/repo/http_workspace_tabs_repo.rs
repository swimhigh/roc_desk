use rusqlite::{params, OptionalExtension};

use crate::db::DbPool;
use crate::error::AppError;
use crate::http_desk::model::HttpWorkspaceTab;

/// 注意：这个仓库连接的是 `workspaces_pool`（工作区独立数据库文件），不是主库
/// `pool`——`http_workspace_tabs` 的迁移挂在 `db::migrate::WORKSPACES_MIGRATIONS`
/// 下，FK 引用的 `workspaces(id)` 必须和它同一个数据库文件才能生效
/// （docs/HTTP_DESKTOP_PLAN.md §5）。
pub struct HttpWorkspaceTabsRepo {
    pool: DbPool,
}

impl HttpWorkspaceTabsRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub fn create(&self, tab: &HttpWorkspaceTab) -> Result<(), AppError> {
        let conn = self.pool.get()?;
        conn.execute(
            "INSERT INTO http_workspace_tabs
                (id, workspace_id, collection_slug, request_id, title, sort_order, updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![
                tab.id,
                tab.workspace_id,
                tab.collection_slug,
                tab.request_id,
                tab.title,
                tab.sort_order,
                tab.updated_at,
            ],
        )?;
        Ok(())
    }

    pub fn update_meta(
        &self,
        id: &str,
        title: &str,
        sort_order: i64,
        updated_at: &str,
    ) -> Result<(), AppError> {
        let conn = self.pool.get()?;
        conn.execute(
            "UPDATE http_workspace_tabs SET title=?2, sort_order=?3, updated_at=?4 WHERE id=?1",
            params![id, title, sort_order, updated_at],
        )?;
        Ok(())
    }

    pub fn delete(&self, id: &str) -> Result<(), AppError> {
        let conn = self.pool.get()?;
        conn.execute("DELETE FROM http_workspace_tabs WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn delete_by_request(&self, workspace_id: &str, request_id: &str) -> Result<(), AppError> {
        let conn = self.pool.get()?;
        conn.execute(
            "DELETE FROM http_workspace_tabs WHERE workspace_id = ?1 AND request_id = ?2",
            params![workspace_id, request_id],
        )?;
        Ok(())
    }

    pub fn get(&self, id: &str) -> Result<Option<HttpWorkspaceTab>, AppError> {
        let conn = self.pool.get()?;
        conn.query_row(
            "SELECT id, workspace_id, collection_slug, request_id, title, sort_order, updated_at
             FROM http_workspace_tabs WHERE id = ?1",
            params![id],
            Self::map_row,
        )
        .optional()
        .map_err(AppError::from)
    }

    pub fn list_for_workspace(&self, workspace_id: &str) -> Result<Vec<HttpWorkspaceTab>, AppError> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT id, workspace_id, collection_slug, request_id, title, sort_order, updated_at
             FROM http_workspace_tabs WHERE workspace_id = ?1 ORDER BY sort_order",
        )?;
        let rows = stmt
            .query_map(params![workspace_id], Self::map_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    fn map_row(row: &rusqlite::Row) -> rusqlite::Result<HttpWorkspaceTab> {
        Ok(HttpWorkspaceTab {
            id: row.get(0)?,
            workspace_id: row.get(1)?,
            collection_slug: row.get(2)?,
            request_id: row.get(3)?,
            title: row.get(4)?,
            sort_order: row.get(5)?,
            updated_at: row.get(6)?,
        })
    }
}
