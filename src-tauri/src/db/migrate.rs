use rusqlite::Connection;

use crate::error::AppError;

/// 迁移脚本按文件名顺序编译进二进制（CODE_DESIGN.md §二 `db/migrate.rs`）。
/// 中间的编号空缺（0004）会在对应功能实现时补上，参见 0006 迁移文件头部注释。
///
/// 三份独立的迁移列表对应三个物理上分开的数据库文件（用户 2026-08-25 需求："SESSION
/// 和工作区的数据存不同文件夹里"）：会话（SSH/RDP 连接档案 + 分组 + known_hosts，
/// 0002/0010）和工作区（0006）各自单独存一份，不再和其余业务数据挤在同一个
/// `roc_desk.db` 里——迁移文件本身的内容不变，只是改成对着不同的连接跑。
const MAIN_MIGRATIONS: &[(&str, &str)] = &[
    ("0001_init", include_str!("../../migrations/0001_init.sql")),
    (
        "0003_logs_fts",
        include_str!("../../migrations/0003_logs_fts.sql"),
    ),
    (
        "0005_audit_log",
        include_str!("../../migrations/0005_audit_log.sql"),
    ),
    (
        "0007_ai_providers",
        include_str!("../../migrations/0007_ai_providers.sql"),
    ),
    (
        "0008_browser_history",
        include_str!("../../migrations/0008_browser_history.sql"),
    ),
    (
        "0009_coding_history",
        include_str!("../../migrations/0009_coding_history.sql"),
    ),
    (
        "0011_permission_rules",
        include_str!("../../migrations/0011_permission_rules.sql"),
    ),
    (
        "0012_mcp_servers",
        include_str!("../../migrations/0012_mcp_servers.sql"),
    ),
];

const SESSIONS_MIGRATIONS: &[(&str, &str)] = &[
    (
        "0002_connections",
        include_str!("../../migrations/0002_connections.sql"),
    ),
    (
        "0010_connection_protocol",
        include_str!("../../migrations/0010_connection_protocol.sql"),
    ),
];

const WORKSPACES_MIGRATIONS: &[(&str, &str)] = &[(
    "0006_workspaces",
    include_str!("../../migrations/0006_workspaces.sql"),
)];

fn apply(conn: &Connection, migrations: &[(&str, &str)]) -> Result<(), AppError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            name TEXT PRIMARY KEY,
            applied_at TEXT NOT NULL DEFAULT (datetime('now'))
        );",
    )?;

    for (name, sql) in migrations {
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

/// 启动时自动 apply 尚未执行过的迁移——主库（AI Provider/日志索引/审计日志等，
/// 不含会话和工作区，那两个各自有独立的数据库文件）。
pub fn run_main_migrations(conn: &Connection) -> Result<(), AppError> {
    apply(conn, MAIN_MIGRATIONS)
}

pub fn run_sessions_migrations(conn: &Connection) -> Result<(), AppError> {
    apply(conn, SESSIONS_MIGRATIONS)
}

pub fn run_workspaces_migrations(conn: &Connection) -> Result<(), AppError> {
    apply(conn, WORKSPACES_MIGRATIONS)
}

/// 一次性把老版本共享数据库（`roc_desk.db`）里的表数据搬到刚拆分出来的独立数据库
/// 文件——只在新库是这次启动才第一次创建、且老库里确实存在对应的表时才搬，调用方
/// 负责判断"新库是不是刚创建"（`!new_db_path.exists()`，必须在 `create_pool` 之前
/// 判断，因为拿连接就会把文件建出来）。搬完不删老库里的旧表，比删除更安全——
/// 多占的磁盘空间可以忽略不计，留着也不会被新代码路径读到。
pub fn migrate_legacy_data(old_db_path: &std::path::Path, new_conn: &Connection, tables: &[&str]) -> Result<(), AppError> {
    if !old_db_path.exists() {
        return Ok(());
    }
    let old_path_str = old_db_path.to_string_lossy().to_string();
    new_conn.execute("ATTACH DATABASE ?1 AS legacy", [old_path_str])?;

    for table in tables {
        let table_exists: bool = new_conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM legacy.sqlite_master WHERE type='table' AND name=?1)",
                [table],
                |row| row.get(0),
            )
            .unwrap_or(false);
        if !table_exists {
            continue;
        }
        // 表名不能参数化绑定，但 `tables` 是调用方写死的内部常量列表，不是外部输入。
        new_conn.execute(&format!("INSERT OR IGNORE INTO {t} SELECT * FROM legacy.{t}", t = table), [])?;
        tracing::info!("migrated legacy table {} into new database", table);
    }

    new_conn.execute_batch("DETACH DATABASE legacy")?;
    Ok(())
}
