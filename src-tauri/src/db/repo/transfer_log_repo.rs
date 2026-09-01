use chrono::Utc;
use rusqlite::params;
use serde::Serialize;
use uuid::Uuid;

use crate::db::DbPool;
use crate::error::AppError;

/// 记一条传输日志用的输入——`record` 在传输命令收尾时调用一次，成功/取消/失败
/// 三种结局都要记（和 `AuditLogRepo::record` 同一个"全记录，不只记失败"的原则，
/// 用户要的是"可查询追溯"，只留失败记录看不出正常传输发生过什么）。
pub struct TransferLogInput<'a> {
    pub protocol: &'a str,
    pub direction: &'a str,
    pub profile_id: Option<Uuid>,
    pub profile_name: &'a str,
    pub local_path: &'a str,
    pub remote_path: &'a str,
    pub is_dir: bool,
    pub file_count: u64,
    pub status: &'a str,
    pub error_message: Option<&'a str>,
    pub started_at: &'a str,
}

#[derive(Debug, Clone, Serialize)]
pub struct TransferLogEntry {
    pub id: Uuid,
    pub protocol: String,
    pub direction: String,
    pub profile_id: Option<Uuid>,
    pub profile_name: String,
    pub local_path: String,
    pub remote_path: String,
    pub is_dir: bool,
    pub file_count: u64,
    pub status: String,
    pub error_message: Option<String>,
    pub started_at: String,
    pub finished_at: String,
}

/// 文件传输日志（用户 2026-09-01 需求："传输日志需要记录，并可在界面上查询追溯"）
/// ——和 `AuditLogRepo` 同一种"尽力而为审计"模式：写日志本身失败只 `tracing::warn!`，
/// 不影响传输命令的返回结果。
pub struct TransferLogRepo {
    pool: DbPool,
}

impl TransferLogRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub fn record(&self, input: TransferLogInput<'_>) {
        let result = (|| -> Result<(), AppError> {
            let conn = self.pool.get()?;
            conn.execute(
                "INSERT INTO transfer_log (id, protocol, direction, profile_id, profile_name, local_path, remote_path, is_dir, file_count, status, error_message, started_at, finished_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                params![
                    Uuid::new_v4().to_string(),
                    input.protocol,
                    input.direction,
                    input.profile_id.map(|id| id.to_string()),
                    input.profile_name,
                    input.local_path,
                    input.remote_path,
                    input.is_dir as i64,
                    input.file_count as i64,
                    input.status,
                    input.error_message,
                    input.started_at,
                    Utc::now().to_rfc3339(),
                ],
            )?;
            Ok(())
        })();
        if let Err(e) = result {
            tracing::warn!("failed to write transfer log: {e}");
        }
    }

    /// `search` 简单匹配本地/远程路径和连接名称——够用的"查询追溯"，不需要一整套
    /// 结构化筛选器。
    pub fn list(&self, limit: u32, offset: u32, search: Option<&str>) -> Result<Vec<TransferLogEntry>, AppError> {
        let conn = self.pool.get()?;
        let like = search.map(|s| format!("%{s}%"));
        let mut stmt = conn.prepare(
            "SELECT id, protocol, direction, profile_id, profile_name, local_path, remote_path, is_dir, file_count, status, error_message, started_at, finished_at
             FROM transfer_log
             WHERE ?1 IS NULL OR local_path LIKE ?1 OR remote_path LIKE ?1 OR profile_name LIKE ?1
             ORDER BY finished_at DESC
             LIMIT ?2 OFFSET ?3",
        )?;
        let rows = stmt
            .query_map(params![like, limit, offset], |r| {
                let profile_id: Option<String> = r.get(3)?;
                Ok(TransferLogEntry {
                    id: Uuid::parse_str(&r.get::<_, String>(0)?).unwrap_or_default(),
                    protocol: r.get(1)?,
                    direction: r.get(2)?,
                    profile_id: profile_id.and_then(|s| Uuid::parse_str(&s).ok()),
                    profile_name: r.get(4)?,
                    local_path: r.get(5)?,
                    remote_path: r.get(6)?,
                    is_dir: r.get::<_, i64>(7)? != 0,
                    file_count: r.get::<_, i64>(8)? as u64,
                    status: r.get(9)?,
                    error_message: r.get(10)?,
                    started_at: r.get(11)?,
                    finished_at: r.get(12)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn clear(&self) -> Result<(), AppError> {
        self.pool.get()?.execute("DELETE FROM transfer_log", [])?;
        Ok(())
    }
}
