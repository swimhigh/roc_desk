use rusqlite::{params, OptionalExtension};
use uuid::Uuid;

use crate::ai::providers::AiProvider;
use crate::db::DbPool;
use crate::error::AppError;

pub struct AiProvidersRepo {
    pool: DbPool,
}

impl AiProvidersRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub fn create(&self, provider: &AiProvider) -> Result<(), AppError> {
        let conn = self.pool.get()?;
        conn.execute(
            "INSERT INTO ai_providers (id, name, api_base, api_key_ref, model, is_local, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                provider.id.to_string(),
                provider.name,
                provider.api_base,
                provider.api_key_ref,
                provider.model,
                provider.is_local as i64,
                provider.created_at,
            ],
        )?;
        Ok(())
    }

    pub fn update(&self, provider: &AiProvider) -> Result<(), AppError> {
        let conn = self.pool.get()?;
        conn.execute(
            "UPDATE ai_providers SET name = ?2, api_base = ?3, api_key_ref = ?4, model = ?5, is_local = ?6
             WHERE id = ?1",
            params![
                provider.id.to_string(),
                provider.name,
                provider.api_base,
                provider.api_key_ref,
                provider.model,
                provider.is_local as i64,
            ],
        )?;
        Ok(())
    }

    pub fn delete(&self, id: Uuid) -> Result<(), AppError> {
        let conn = self.pool.get()?;
        conn.execute("DELETE FROM ai_providers WHERE id = ?1", params![id.to_string()])?;
        Ok(())
    }

    pub fn get(&self, id: Uuid) -> Result<Option<AiProvider>, AppError> {
        let conn = self.pool.get()?;
        conn.query_row(
            "SELECT id, name, api_base, api_key_ref, model, is_local, created_at
             FROM ai_providers WHERE id = ?1",
            params![id.to_string()],
            Self::map_row,
        )
        .optional()
        .map_err(AppError::from)
    }

    pub fn list(&self) -> Result<Vec<AiProvider>, AppError> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT id, name, api_base, api_key_ref, model, is_local, created_at
             FROM ai_providers ORDER BY created_at",
        )?;
        let rows = stmt.query_map([], Self::map_row)?.collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    fn map_row(row: &rusqlite::Row) -> rusqlite::Result<AiProvider> {
        let id: String = row.get(0)?;
        Ok(AiProvider {
            id: Uuid::parse_str(&id).unwrap_or_else(|_| Uuid::nil()),
            name: row.get(1)?,
            api_base: row.get(2)?,
            api_key_ref: row.get(3)?,
            model: row.get(4)?,
            is_local: row.get::<_, i64>(5)? != 0,
            created_at: row.get(6)?,
        })
    }
}
