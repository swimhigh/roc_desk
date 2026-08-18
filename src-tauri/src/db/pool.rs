use r2d2_sqlite::SqliteConnectionManager;
use std::path::Path;

use crate::error::AppError;

/// SQLite 连接池封装（CODE_DESIGN.md §二 `db/pool.rs`）。
pub type DbPool = r2d2::Pool<SqliteConnectionManager>;

pub fn create_pool(db_path: &Path) -> Result<DbPool, AppError> {
    let manager = SqliteConnectionManager::file(db_path).with_init(|conn| {
        conn.execute_batch("PRAGMA foreign_keys = OFF; PRAGMA journal_mode = WAL;")
    });
    r2d2::Pool::new(manager).map_err(|e| AppError::Database(e.to_string()))
}
