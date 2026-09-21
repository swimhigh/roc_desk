use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 首期支持的数据库类型（docs/SQL_DESKTOP_PLAN.md §1.1）。Oracle 暂不落地
/// （需要本机装 OCI/Instant Client，见方案 4.3 风险表），先占个枚举值，
/// registry 里对应分支直接返回"未实现"错误，不接可用适配器。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DbKind {
    Mysql,
    /// Tencent TDSQL through its MySQL-compatible proxy. It intentionally
    /// reuses the MySQL wire adapter while keeping a distinct profile type so
    /// TDSQL-specific defaults and diagnostics can be added without changing
    /// ordinary MySQL connections.
    Tdsql,
    Postgres,
    Opengauss,
    SqlServer,
    Oracle,
}

impl DbKind {
    pub fn default_port(self) -> Option<u16> {
        match self {
            DbKind::Mysql => Some(3306),
            // TDSQL deployments commonly expose a proxy port (15300 in the
            // reported environment) instead of the ordinary MySQL 3306.
            // The field remains editable because Tencent regions/products can
            // choose a different listener port.
            DbKind::Tdsql => Some(15300),
            DbKind::Postgres | DbKind::Opengauss => Some(5432),
            DbKind::SqlServer => Some(1433),
            DbKind::Oracle => Some(1521),
        }
    }
}

/// 数据源档案（对应 SQLite 表 `sql_data_sources`）。密码不在这里——只存
/// `credential_ref`，真正的密钥走 `AppState.credential_store`（见
/// docs/SQL_DESKTOP_PLAN.md §2.4、§5）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataSourceProfile {
    pub id: Uuid,
    pub name: String,
    pub db_kind: DbKind,
    pub host: String,
    pub port: Option<u16>,
    pub database_name: Option<String>,
    pub default_schema: Option<String>,
    pub username: Option<String>,
    pub credential_ref: Option<String>,
    pub environment: String,
    pub group_name: Option<String>,
    pub readonly: bool,
    pub ssl_required: bool,
    pub created_at: String,
    pub updated_at: String,
    pub last_used_at: Option<String>,
}

/// 新增/编辑数据源用的输入 DTO——`password` 为 `Some("")` 时视为"不修改密码"
/// （对齐 `AiProviderInput` 的既有约定，见 `ai/providers.rs`）。
#[derive(Debug, Clone, Deserialize)]
pub struct DataSourceInput {
    pub name: String,
    pub db_kind: DbKind,
    pub host: String,
    pub port: Option<u16>,
    pub database_name: Option<String>,
    pub default_schema: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub environment: String,
    pub group_name: Option<String>,
    pub readonly: bool,
    pub ssl_required: bool,
}

/// 连接后解析出的、adapter 真正拿去建连接的信息（密码已经从 keyring 取出来）。
/// 不实现 `Serialize`——绝不能被序列化传回前端。
#[derive(Debug, Clone)]
pub struct ResolvedProfile {
    pub id: Uuid,
    pub db_kind: DbKind,
    pub host: String,
    pub port: u16,
    pub database_name: Option<String>,
    pub default_schema: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub ssl_required: bool,
    pub readonly: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct DbInfo {
    pub version: String,
    pub latency_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectKind {
    Table,
    View,
    MaterializedView,
    Function,
    Procedure,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectRef {
    pub schema: String,
    pub name: String,
    pub kind: ObjectKind,
}

#[derive(Debug, Clone, Serialize)]
pub struct ObjectPage {
    pub objects: Vec<ObjectRef>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ColumnDef {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub default_value: Option<String>,
    pub is_primary_key: bool,
    /// 列注释（COMMENT）——不是所有数据库/字段都有。
    pub comment: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct IndexDef {
    pub name: String,
    pub definition: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ObjectDefinition {
    pub object: ObjectRef,
    pub columns: Vec<ColumnDef>,
    pub indexes: Vec<IndexDef>,
    /// 有 DDL/视图定义/函数体时才有值（不是所有数据库/对象类型都取得到）。
    pub ddl: Option<String>,
    /// 表/视图自身的注释（COMMENT），对象浏览器默认展示这个（2026-09 用户
    /// 反馈：单击展开时希望先看到表自身的说明，不只是列清单）。
    pub comment: Option<String>,
}

/// 结果集单元格——不用强类型枚举对齐每种驱动的原生类型，统一转成字符串
/// 展示（参考 rainfrog `Value{string,is_null,parse_error}` 的做法，见
/// docs/SQL_DESKTOP_PLAN.md §4.2.1）。`is_binary` 时 `text` 是十六进制串。
#[derive(Debug, Clone, Serialize)]
pub struct Cell {
    pub text: String,
    pub is_null: bool,
    pub is_binary: bool,
}

impl Cell {
    pub fn null() -> Self {
        Cell {
            text: String::new(),
            is_null: true,
            is_binary: false,
        }
    }

    pub fn text(text: impl Into<String>) -> Self {
        Cell {
            text: text.into(),
            is_null: false,
            is_binary: false,
        }
    }

    pub fn binary_hex(bytes: &[u8]) -> Self {
        let mut s = String::with_capacity(bytes.len() * 2);
        for b in bytes {
            s.push_str(&format!("{b:02X}"));
        }
        Cell {
            text: s,
            is_null: false,
            is_binary: true,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ColumnInfo {
    pub name: String,
    pub type_name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExecuteResult {
    pub columns: Vec<ColumnInfo>,
    pub rows: Vec<Vec<Cell>>,
    pub rows_affected: Option<u64>,
    pub truncated: bool,
    pub duration_ms: u64,
}

/// 待确认的写操作（UPDATE/DELETE 等支持事务回滚的语句）——已经在一个独立
/// 检出的连接上开了事务并执行完，展示真实受影响行数，等用户确认再
/// commit/rollback（docs/SQL_DESKTOP_PLAN.md §4.2.1 第 3 点，参考 rainfrog
/// `DbTaskResult::ConfirmTx`）。
#[derive(Debug, Clone, Serialize)]
pub struct PendingWrite {
    pub pending_id: Uuid,
    pub rows_affected: Option<u64>,
    pub preview: ExecuteResult,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryStatus {
    Running,
    Finished,
    Error,
    Cancelled,
}

#[derive(Debug, Clone, Serialize)]
pub struct QueryPoll {
    pub status: QueryStatus,
    pub result: Option<ExecuteResult>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct QueryHistoryEntry {
    pub id: Uuid,
    pub data_source_id: Uuid,
    pub title: Option<String>,
    pub sql_text: String,
    pub status: String,
    pub duration_ms: Option<i64>,
    pub row_count: Option<i64>,
    pub error_message: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceTab {
    pub id: Uuid,
    pub data_source_id: Uuid,
    pub title: String,
    /// 相对 `<sql_cache_dir>/<data_source_id>/` 的路径，指向真实的 `.sql`
    /// 文件（docs/SQL_DESKTOP_PLAN.md §4.4）——内容的唯一真相在磁盘上，
    /// 这张表只存元数据。
    pub file_path: String,
    pub result_view_mode: String,
    pub cursor_json: Option<String>,
    pub sort_order: i64,
    pub updated_at: String,
}
