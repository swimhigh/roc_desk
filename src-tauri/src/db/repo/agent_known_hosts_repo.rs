use chrono::Utc;
use rusqlite::{params, OptionalExtension};
use uuid::Uuid;

use crate::db::DbPool;
use crate::error::AppError;

pub enum AgentKnownHostStatus {
    Match,
    Mismatch(String), // 旧指纹
    Unknown,
}

/// Agent TLS 证书指纹 TOFU（AGENT_DESIGN.md §3.1），和 `KnownHostsRepo` 是同一种
/// 模式，按 `connection_id` 而不是 host/port 做主键——见迁移文件 0013 的注释。
pub struct AgentKnownHostsRepo {
    pool: DbPool,
}

impl AgentKnownHostsRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub fn lookup(&self, connection_id: Uuid, fingerprint: &str) -> Result<AgentKnownHostStatus, AppError> {
        let conn = self.pool.get()?;
        let existing: Option<String> = conn
            .query_row(
                "SELECT fingerprint FROM agent_known_hosts WHERE connection_id = ?1",
                params![connection_id.to_string()],
                |row| row.get(0),
            )
            .optional()?;

        Ok(match existing {
            None => AgentKnownHostStatus::Unknown,
            Some(old) if old == fingerprint => AgentKnownHostStatus::Match,
            Some(old) => AgentKnownHostStatus::Mismatch(old),
        })
    }

    pub fn save(&self, connection_id: Uuid, fingerprint: &str) -> Result<(), AppError> {
        let conn = self.pool.get()?;
        conn.execute(
            "INSERT INTO agent_known_hosts (connection_id, fingerprint, trusted_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(connection_id) DO UPDATE SET fingerprint = excluded.fingerprint, trusted_at = excluded.trusted_at",
            params![connection_id.to_string(), fingerprint, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }
}
