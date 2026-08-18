use chrono::Utc;
use rusqlite::{params, OptionalExtension};

use crate::db::DbPool;
use crate::error::AppError;

pub enum KnownHostStatus {
    Match,
    Mismatch(String), // 旧指纹
    Unknown,
}

pub struct KnownHostsRepo {
    pool: DbPool,
}

impl KnownHostsRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    /// TOFU / 指纹比对（DESIGN.md §3.2.1）。
    pub fn lookup(&self, host: &str, port: u16, fingerprint: &str) -> Result<KnownHostStatus, AppError> {
        let conn = self.pool.get()?;
        let existing: Option<String> = conn
            .query_row(
                "SELECT fingerprint FROM known_hosts WHERE host = ?1 AND port = ?2",
                params![host, port as i64],
                |row| row.get(0),
            )
            .optional()?;

        Ok(match existing {
            None => KnownHostStatus::Unknown,
            Some(old) if old == fingerprint => KnownHostStatus::Match,
            Some(old) => KnownHostStatus::Mismatch(old),
        })
    }

    pub fn save(&self, host: &str, port: u16, fingerprint: &str) -> Result<(), AppError> {
        let conn = self.pool.get()?;
        conn.execute(
            "INSERT INTO known_hosts (host, port, fingerprint, trusted_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(host, port) DO UPDATE SET fingerprint = excluded.fingerprint, trusted_at = excluded.trusted_at",
            params![host, port as i64, fingerprint, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }
}
