use r2d2_sqlite::SqliteConnectionManager;
use std::path::Path;

use crate::error::AppError;

/// SQLite 连接池封装（CODE_DESIGN.md §二 `db/pool.rs`）。
pub type DbPool = r2d2::Pool<SqliteConnectionManager>;

pub fn create_pool(db_path: &Path) -> Result<DbPool, AppError> {
    let manager = SqliteConnectionManager::file(db_path).with_init(|conn| {
        conn.execute_batch("PRAGMA foreign_keys = OFF; PRAGMA journal_mode = WAL;")
    });
    // `min_idle(Some(0))`：不要在建池那一刻就抢着预热到 max_size 条连接——本来一个
    // 进程只建一个 pool 时这个抖动不明显，现在会话/工作区各自拆了独立数据库文件
    // （2026-08-25 数据库拆分），启动时同时建 3 个 pool，默认的"立刻预热到 10 条"
    // 会让好几条连接同时抢着对刚创建的空文件写 WAL 头，触发一串可自愈但很吵的
    // `database is locked` 重试日志。改成按需惰性建连接，启动更安静，桌面应用
    // 本来也用不上高并发连接池。
    r2d2::Pool::builder()
        .max_size(8)
        .min_idle(Some(0))
        .build(manager)
        .map_err(|e| AppError::Database(e.to_string()))
}
