use chrono::Utc;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::db::DbPool;
use crate::error::AppError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserHistoryEntry {
    pub id: Uuid,
    pub url: String,
    pub title: Option<String>,
    pub visited_at: String,
}

pub struct BrowserHistoryRepo {
    pool: DbPool,
}

impl BrowserHistoryRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub fn add(&self, url: &str, title: Option<&str>) -> Result<BrowserHistoryEntry, AppError> {
        let entry = BrowserHistoryEntry {
            id: Uuid::new_v4(),
            url: url.to_string(),
            title: title.map(str::to_string),
            visited_at: Utc::now().to_rfc3339(),
        };
        let conn = self.pool.get()?;
        conn.execute(
            "INSERT INTO browser_history (id, url, title, visited_at) VALUES (?1, ?2, ?3, ?4)",
            params![entry.id.to_string(), entry.url, entry.title, entry.visited_at],
        )?;
        Ok(entry)
    }

    pub fn list_recent(&self, limit: usize) -> Result<Vec<BrowserHistoryEntry>, AppError> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT id, url, title, visited_at FROM browser_history ORDER BY visited_at DESC LIMIT ?1",
        )?;
        let rows = stmt
            .query_map(params![limit as i64], Self::map_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn remove(&self, id: Uuid) -> Result<(), AppError> {
        let conn = self.pool.get()?;
        conn.execute("DELETE FROM browser_history WHERE id = ?1", params![id.to_string()])?;
        Ok(())
    }

    pub fn clear(&self) -> Result<(), AppError> {
        let conn = self.pool.get()?;
        conn.execute("DELETE FROM browser_history", [])?;
        Ok(())
    }

    fn map_row(row: &rusqlite::Row) -> rusqlite::Result<BrowserHistoryEntry> {
        let id: String = row.get(0)?;
        Ok(BrowserHistoryEntry {
            id: Uuid::parse_str(&id).unwrap_or_else(|_| Uuid::nil()),
            url: row.get(1)?,
            title: row.get(2)?,
            visited_at: row.get(3)?,
        })
    }
}
