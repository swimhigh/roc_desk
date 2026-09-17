use rusqlite::params;
use uuid::Uuid;

use crate::db::DbPool;
use crate::error::AppError;
use crate::sql::model::QueryHistoryEntry;

pub struct SqlQueryHistoryRepo {
    pool: DbPool,
}

impl SqlQueryHistoryRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record(
        &self,
        data_source_id: Uuid,
        sql_text: &str,
        status: &str,
        duration_ms: Option<i64>,
        row_count: Option<i64>,
        error_message: Option<&str>,
        created_at: &str,
    ) -> Result<(), AppError> {
        let conn = self.pool.get()?;
        conn.execute(
            "INSERT INTO sql_query_history
                (id, data_source_id, sql_text, status, duration_ms, row_count, error_message, created_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                Uuid::new_v4().to_string(),
                data_source_id.to_string(),
                sql_text,
                status,
                duration_ms,
                row_count,
                error_message,
                created_at,
            ],
        )?;
        Ok(())
    }

    pub fn list(&self, data_source_id: Uuid, limit: i64) -> Result<Vec<QueryHistoryEntry>, AppError> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT id, data_source_id, sql_text, status, duration_ms, row_count, error_message, created_at
             FROM sql_query_history WHERE data_source_id = ?1 ORDER BY created_at DESC LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![data_source_id.to_string(), limit], |row| {
                let id: String = row.get(0)?;
                let ds: String = row.get(1)?;
                Ok(QueryHistoryEntry {
                    id: Uuid::parse_str(&id).unwrap_or_else(|_| Uuid::nil()),
                    data_source_id: Uuid::parse_str(&ds).unwrap_or_else(|_| Uuid::nil()),
                    title: None,
                    sql_text: row.get(2)?,
                    status: row.get(3)?,
                    duration_ms: row.get(4)?,
                    row_count: row.get(5)?,
                    error_message: row.get(6)?,
                    created_at: row.get(7)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn delete(&self, id: Uuid) -> Result<(), AppError> {
        let conn = self.pool.get()?;
        conn.execute(
            "DELETE FROM sql_query_history WHERE id = ?1",
            params![id.to_string()],
        )?;
        Ok(())
    }
}
