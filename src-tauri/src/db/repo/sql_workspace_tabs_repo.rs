use rusqlite::{params, OptionalExtension};
use uuid::Uuid;

use crate::db::DbPool;
use crate::error::AppError;
use crate::sql::model::WorkspaceTab;

pub struct SqlWorkspaceTabsRepo {
    pool: DbPool,
}

impl SqlWorkspaceTabsRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub fn create(&self, tab: &WorkspaceTab) -> Result<(), AppError> {
        let conn = self.pool.get()?;
        conn.execute(
            "INSERT INTO sql_workspace_tabs
                (id, data_source_id, title, file_path, result_view_mode, cursor_json, sort_order, updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                tab.id.to_string(),
                tab.data_source_id.to_string(),
                tab.title,
                tab.file_path,
                tab.result_view_mode,
                tab.cursor_json,
                tab.sort_order,
                tab.updated_at,
            ],
        )?;
        Ok(())
    }

    pub fn update_meta(
        &self,
        id: Uuid,
        title: &str,
        result_view_mode: &str,
        cursor_json: Option<&str>,
        sort_order: i64,
        updated_at: &str,
    ) -> Result<(), AppError> {
        let conn = self.pool.get()?;
        conn.execute(
            "UPDATE sql_workspace_tabs SET title=?2, result_view_mode=?3, cursor_json=?4,
                sort_order=?5, updated_at=?6 WHERE id=?1",
            params![id.to_string(), title, result_view_mode, cursor_json, sort_order, updated_at],
        )?;
        Ok(())
    }

    pub fn delete(&self, id: Uuid) -> Result<(), AppError> {
        let conn = self.pool.get()?;
        conn.execute(
            "DELETE FROM sql_workspace_tabs WHERE id = ?1",
            params![id.to_string()],
        )?;
        Ok(())
    }

    pub fn get(&self, id: Uuid) -> Result<Option<WorkspaceTab>, AppError> {
        let conn = self.pool.get()?;
        conn.query_row(
            "SELECT id, data_source_id, title, file_path, result_view_mode, cursor_json, sort_order, updated_at
             FROM sql_workspace_tabs WHERE id = ?1",
            params![id.to_string()],
            Self::map_row,
        )
        .optional()
        .map_err(AppError::from)
    }

    pub fn list_for_data_source(&self, data_source_id: Uuid) -> Result<Vec<WorkspaceTab>, AppError> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT id, data_source_id, title, file_path, result_view_mode, cursor_json, sort_order, updated_at
             FROM sql_workspace_tabs WHERE data_source_id = ?1 ORDER BY sort_order",
        )?;
        let rows = stmt
            .query_map(params![data_source_id.to_string()], Self::map_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    fn map_row(row: &rusqlite::Row) -> rusqlite::Result<WorkspaceTab> {
        let id: String = row.get(0)?;
        let ds: String = row.get(1)?;
        Ok(WorkspaceTab {
            id: Uuid::parse_str(&id).unwrap_or_else(|_| Uuid::nil()),
            data_source_id: Uuid::parse_str(&ds).unwrap_or_else(|_| Uuid::nil()),
            title: row.get(2)?,
            file_path: row.get(3)?,
            result_view_mode: row.get(4)?,
            cursor_json: row.get(5)?,
            sort_order: row.get(6)?,
            updated_at: row.get(7)?,
        })
    }
}
