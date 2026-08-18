use rusqlite::Connection;

use crate::error::AppError;

/// 迁移脚本按文件名顺序编译进二进制（CODE_DESIGN.md §二 `db/migrate.rs`）。
/// 中间的编号空缺（0002~0005）会在对应功能实现时补上，参见 0006 迁移文件头部注释。
const MIGRATIONS: &[(&str, &str)] = &[
    ("0001_init", include_str!("../../migrations/0001_init.sql")),
    (
        "0002_connections",
        include_str!("../../migrations/0002_connections.sql"),
    ),
    (
        "0003_logs_fts",
        include_str!("../../migrations/0003_logs_fts.sql"),
    ),
    (
        "0005_audit_log",
        include_str!("../../migrations/0005_audit_log.sql"),
    ),
    (
        "0006_workspaces",
        include_str!("../../migrations/0006_workspaces.sql"),
    ),
    (
        "0007_ai_providers",
        include_str!("../../migrations/0007_ai_providers.sql"),
    ),
    (
        "0008_browser_history",
        include_str!("../../migrations/0008_browser_history.sql"),
    ),
];

/// 启动时自动 apply 尚未执行过的迁移。
pub fn run_migrations(conn: &Connection) -> Result<(), AppError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            name TEXT PRIMARY KEY,
            applied_at TEXT NOT NULL DEFAULT (datetime('now'))
        );",
    )?;

    for (name, sql) in MIGRATIONS {
        let already_applied: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM schema_migrations WHERE name = ?1)",
            [name],
            |row| row.get(0),
        )?;
        if already_applied {
            continue;
        }
        conn.execute_batch(sql)?;
        conn.execute(
            "INSERT INTO schema_migrations (name) VALUES (?1)",
            [name],
        )?;
        tracing::info!("applied migration {}", name);
    }

    Ok(())
}
