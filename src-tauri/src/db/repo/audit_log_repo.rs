use chrono::Utc;
use rusqlite::params;
use uuid::Uuid;

use crate::db::DbPool;
use crate::error::AppError;

pub struct AuditLogRepo {
    pool: DbPool,
}

impl AuditLogRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    /// 记录一次 `run_command` 调用尝试（DESIGN.md §3.8.2.1）——无论是被黑名单拦截、
    /// 被用户拒绝还是实际执行，都要留痕，所以这是个"尽力而为"的写入：审计写入本身
    /// 失败不应该阻断命令的执行流程，调用方只 `tracing::warn!` 不向上传播错误。
    pub fn record(&self, session_id: Uuid, target_label: &str, command: &str, outcome: &str, output_summary: Option<&str>) {
        let result = (|| -> Result<(), AppError> {
            let conn = self.pool.get()?;
            conn.execute(
                "INSERT INTO command_audit_log (id, session_id, target_label, command, outcome, output_summary, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    Uuid::new_v4().to_string(),
                    session_id.to_string(),
                    target_label,
                    command,
                    outcome,
                    output_summary,
                    Utc::now().to_rfc3339(),
                ],
            )?;
            Ok(())
        })();
        if let Err(e) = result {
            tracing::warn!("failed to write command audit log: {e}");
        }
    }
}
