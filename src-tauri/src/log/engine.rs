use rusqlite::params;
use serde::{Deserialize, Serialize};

use super::parser::parse_log_line;
use crate::db::DbPool;
use crate::error::AppError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogSearchResult {
    pub file_path: String,
    pub line_number: i64,
    pub timestamp: Option<String>,
    pub log_level: Option<String>,
    pub host_name: String,
    pub snippet: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LogQuery {
    pub query: String,
    pub limit: Option<usize>,
}

/// 模式 B：本地索引搜索（DESIGN.md §3.4.2）。FTS5 查询封装。
pub struct LogSearchEngine {
    pool: DbPool,
}

impl LogSearchEngine {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    /// 逐行导入日志内容并建立 FTS5 索引（DESIGN.md §3.4.2）。调用方负责把远程/本地
    /// 文件先落到可迭代的字符串行来源——见 `importer.rs` 处理具体的文件读取。
    pub fn import_lines<'a>(
        &self,
        file_path: &str,
        host_name: &str,
        lines: impl Iterator<Item = &'a str>,
    ) -> Result<usize, AppError> {
        let mut conn = self.pool.get()?;
        let tx = conn.transaction()?;
        let mut count = 0usize;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO logs (content, file_path, line_number, timestamp, log_level, host_name)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?;
            for (line_num, line) in lines.enumerate() {
                let parsed = parse_log_line(line);
                stmt.execute(params![
                    line,
                    file_path,
                    (line_num + 1) as i64,
                    parsed.timestamp,
                    parsed.level,
                    host_name,
                ])?;
                count += 1;
            }
        }
        tx.commit()?;

        tx_record_job(&self.pool, file_path, host_name, count)?;
        Ok(count)
    }

    pub fn search(&self, query: &LogQuery) -> Result<Vec<LogSearchResult>, AppError> {
        let conn = self.pool.get()?;
        let limit = query.limit.unwrap_or(200) as i64;
        let mut stmt = conn.prepare(
            "SELECT file_path, line_number, timestamp, log_level, host_name,
                    snippet(logs, 0, '<mark>', '</mark>', '...', 20)
             FROM logs WHERE logs MATCH ?1 ORDER BY rank LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![query.query, limit], |row| {
                Ok(LogSearchResult {
                    file_path: row.get(0)?,
                    line_number: row.get(1)?,
                    timestamp: row.get(2)?,
                    log_level: row.get(3)?,
                    host_name: row.get(4)?,
                    snippet: row.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn index_stats(&self) -> Result<IndexStats, AppError> {
        let conn = self.pool.get()?;
        let row_count: i64 = conn.query_row("SELECT COUNT(*) FROM logs", [], |r| r.get(0))?;
        let job_count: i64 = conn.query_row("SELECT COUNT(*) FROM log_import_jobs", [], |r| r.get(0))?;
        Ok(IndexStats { row_count, job_count })
    }

    /// LRU 清理：删除超过 `older_than_days` 天未刷新的导入任务及其索引行
    /// （DESIGN.md §十-3：FTS5 索引通常是原文 1.5-2 倍大，需要配额策略）。
    pub fn clear_older_than(&self, older_than_days: i64) -> Result<usize, AppError> {
        let mut conn = self.pool.get()?;
        let tx = conn.transaction()?;
        let stale_paths: Vec<String> = {
            let mut stmt = tx.prepare(
                "SELECT file_path FROM log_import_jobs WHERE created_at < datetime('now', ?1)",
            )?;
            let rows = stmt
                .query_map(params![format!("-{older_than_days} days")], |r| r.get(0))?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        };
        let mut removed = 0usize;
        for path in &stale_paths {
            removed += tx.execute("DELETE FROM logs WHERE file_path = ?1", params![path])?;
        }
        tx.execute(
            "DELETE FROM log_import_jobs WHERE created_at < datetime('now', ?1)",
            params![format!("-{older_than_days} days")],
        )?;
        tx.commit()?;
        Ok(removed)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexStats {
    pub row_count: i64,
    pub job_count: i64,
}

fn tx_record_job(pool: &DbPool, file_path: &str, host_name: &str, count: usize) -> Result<(), AppError> {
    let conn = pool.get()?;
    conn.execute(
        "INSERT INTO log_import_jobs (id, host_name, file_path, status, bytes_total, bytes_done, created_at)
         VALUES (?1, ?2, ?3, 'done', ?4, ?4, datetime('now'))",
        params![uuid::Uuid::new_v4().to_string(), host_name, file_path, count as i64],
    )?;
    Ok(())
}
