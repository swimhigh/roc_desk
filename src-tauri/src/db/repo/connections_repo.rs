use chrono::Utc;
use rusqlite::{params, OptionalExtension};
use uuid::Uuid;

use crate::connection::profile::{AuthMethod, ConnectionProfile};
use crate::db::DbPool;
use crate::error::AppError;

pub struct ConnectionsRepo {
    pool: DbPool,
}

impl ConnectionsRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub fn create(&self, profile: &ConnectionProfile) -> Result<(), AppError> {
        let conn = self.pool.get()?;
        conn.execute(
            "INSERT INTO connections
                (id, name, host, port, username, auth_method, credential_ref, group_id, tags, jump_host_id, last_connected_at, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                profile.id.to_string(),
                profile.name,
                profile.host,
                profile.port,
                profile.username,
                profile.auth_method.as_str(),
                profile.credential_ref,
                profile.group_id.map(|g| g.to_string()),
                serde_json::to_string(&profile.tags).unwrap_or_default(),
                profile.jump_host_id.map(|j| j.to_string()),
                profile.last_connected_at,
                profile.created_at,
            ],
        )?;
        Ok(())
    }

    pub fn update(&self, profile: &ConnectionProfile) -> Result<(), AppError> {
        let conn = self.pool.get()?;
        conn.execute(
            "UPDATE connections SET
                name = ?2, host = ?3, port = ?4, username = ?5, auth_method = ?6,
                credential_ref = ?7, group_id = ?8, tags = ?9, jump_host_id = ?10
             WHERE id = ?1",
            params![
                profile.id.to_string(),
                profile.name,
                profile.host,
                profile.port,
                profile.username,
                profile.auth_method.as_str(),
                profile.credential_ref,
                profile.group_id.map(|g| g.to_string()),
                serde_json::to_string(&profile.tags).unwrap_or_default(),
                profile.jump_host_id.map(|j| j.to_string()),
            ],
        )?;
        Ok(())
    }

    pub fn touch_last_connected(&self, id: Uuid) -> Result<(), AppError> {
        let conn = self.pool.get()?;
        conn.execute(
            "UPDATE connections SET last_connected_at = ?1 WHERE id = ?2",
            params![Utc::now().to_rfc3339(), id.to_string()],
        )?;
        Ok(())
    }

    pub fn get(&self, id: Uuid) -> Result<Option<ConnectionProfile>, AppError> {
        let conn = self.pool.get()?;
        let result = conn
            .query_row(
                "SELECT id, name, host, port, username, auth_method, credential_ref,
                        group_id, tags, jump_host_id, last_connected_at, created_at
                 FROM connections WHERE id = ?1",
                params![id.to_string()],
                Self::map_row,
            )
            .optional()?;
        Ok(result)
    }

    pub fn list(&self, group_id: Option<Uuid>) -> Result<Vec<ConnectionProfile>, AppError> {
        let conn = self.pool.get()?;
        let mut stmt = if group_id.is_some() {
            conn.prepare(
                "SELECT id, name, host, port, username, auth_method, credential_ref,
                        group_id, tags, jump_host_id, last_connected_at, created_at
                 FROM connections WHERE group_id = ?1 ORDER BY name",
            )?
        } else {
            conn.prepare(
                "SELECT id, name, host, port, username, auth_method, credential_ref,
                        group_id, tags, jump_host_id, last_connected_at, created_at
                 FROM connections ORDER BY name",
            )?
        };

        let rows = if let Some(gid) = group_id {
            stmt.query_map(params![gid.to_string()], Self::map_row)?
                .collect::<Result<Vec<_>, _>>()?
        } else {
            stmt.query_map([], Self::map_row)?.collect::<Result<Vec<_>, _>>()?
        };
        Ok(rows)
    }

    pub fn delete(&self, id: Uuid) -> Result<(), AppError> {
        let conn = self.pool.get()?;
        conn.execute("DELETE FROM connections WHERE id = ?1", params![id.to_string()])?;
        Ok(())
    }

    fn map_row(row: &rusqlite::Row) -> rusqlite::Result<ConnectionProfile> {
        let id: String = row.get(0)?;
        let auth_method: String = row.get(5)?;
        let group_id: Option<String> = row.get(7)?;
        let tags_json: String = row.get(8)?;
        let jump_host_id: Option<String> = row.get(9)?;

        Ok(ConnectionProfile {
            id: Uuid::parse_str(&id).unwrap_or_else(|_| Uuid::nil()),
            name: row.get(1)?,
            host: row.get(2)?,
            port: row.get::<_, i64>(3)? as u16,
            username: row.get(4)?,
            auth_method: AuthMethod::from_str(&auth_method),
            credential_ref: row.get(6)?,
            group_id: group_id.and_then(|s| Uuid::parse_str(&s).ok()),
            tags: serde_json::from_str(&tags_json).unwrap_or_default(),
            jump_host_id: jump_host_id.and_then(|s| Uuid::parse_str(&s).ok()),
            last_connected_at: row.get(10)?,
            created_at: row.get(11)?,
        })
    }
}
