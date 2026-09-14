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
    ///
    /// 2026-09 用户实测长期复现、定位到的根因：这里原来是纯同步阻塞调用
    /// （`self.pool.get()` 等一个连接池里的连接、`conn.execute` 直接做同步磁盘
    /// I/O），直接跑在调用方所在的 tokio 任务里。`run_command_gated_shared_
    /// with_status_in_context` 命中已有的"允许"权限规则时，会在真正执行命令之前
    /// 先调用一次这个函数——如果这次同步调用因为连接池竞争/SQLite 忙等而卡住，
    /// 卡住的是**这个 tokio 任务本身**，不是某个可以被外层 `tokio::time::timeout`
    /// 打断的独立 await：一个同步阻塞调用不会把控制权交还给执行器，包住它的
    /// timeout（比如 `SshSession::exec` 那层 120 秒超时）永远没有机会被轮询到，
    /// 表现就是"无论等多久都不报错、也不继续"——诊断日志显示卡死点前所有异步
    /// 环节全部正常也是这个原因：卡住的根本不是某次 await，是这次同步调用没有
    /// 把线程让出去。改成 `spawn_blocking` 之后，真正的阻塞 I/O 挪到 tokio 专门
    /// 给同步任务准备的线程池上跑，调用方这边永远是非阻塞的"发射后不管"，不会
    /// 再拖累任何外层的超时/调度。
    pub fn record(
        &self,
        session_id: Uuid,
        target_label: &str,
        command: &str,
        outcome: &str,
        output_summary: Option<&str>,
    ) {
        let pool = self.pool.clone();
        let target_label = target_label.to_string();
        let command = command.to_string();
        let outcome = outcome.to_string();
        let output_summary = output_summary.map(|s| s.to_string());
        tokio::task::spawn_blocking(move || {
            let result = (|| -> Result<(), AppError> {
                let conn = pool.get()?;
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
        });
    }
}
