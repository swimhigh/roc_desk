use std::sync::{Arc, Mutex as StdMutex};

use async_trait::async_trait;
use uuid::Uuid;

use crate::error::AppError;
use crate::sql::model::*;

/// 正在执行的查询用来记录"服务端怎么杀我"的信息（Postgres backend pid、
/// MySQL connection id、SQL Server SPID）——`execute_sql` 拿到连接后立刻
/// 把这个信息写进去，`kill_backend` 才有东西可用。取消令牌本身只能中断
/// 本地 tokio 任务，不会让数据库停止已经下发的语句，这是 rainfrog 踩过的
/// 坑，见 docs/SQL_DESKTOP_PLAN.md §4.2.1 第 2 点。
pub type BackendHandleSlot = Arc<StdMutex<Option<String>>>;

pub fn new_backend_handle_slot() -> BackendHandleSlot {
    Arc::new(StdMutex::new(None))
}

/// Preview SQL uses the adapter dialect and always quotes metadata identifiers.
pub fn preview_sql(kind: DbKind, object: &ObjectRef) -> String {
    let quote = |s: &str| match kind {
        DbKind::Mysql | DbKind::Tdsql => format!("`{}`", s.replace('`', "``")),
        DbKind::SqlServer => format!("[{}]", s.replace(']', "]]")),
        _ => format!("\"{}\"", s.replace('"', "\"\"")),
    };
    let table = if object.schema.is_empty() { quote(&object.name) } else { format!("{}.{}", quote(&object.schema), quote(&object.name)) };
    match kind {
        DbKind::SqlServer => format!("SELECT TOP (100) * FROM {table};"),
        DbKind::Oracle => format!("SELECT * FROM {table} FETCH FIRST 100 ROWS ONLY;"),
        _ => format!("SELECT * FROM {table} LIMIT 100;"),
    }
}

/// 数据库类型 -> 具体连接实现的工厂接口（docs/SQL_DESKTOP_PLAN.md §4.2）。
#[async_trait]
pub trait DatabaseAdapter: Send + Sync {
    fn kind(&self) -> DbKind;
    async fn test_connection(&self, profile: &ResolvedProfile) -> Result<DbInfo, AppError>;
    async fn open_session(
        &self,
        profile: &ResolvedProfile,
    ) -> Result<Arc<dyn AdapterSession>, AppError>;
}

/// 一个已建立连接（池）的数据源会话——`sql::service::SqlSessionManager` 按
/// `data_source_id` 持有一份，多个标签页/窗口共享同一个连接池（方案 §2.5）。
#[async_trait]
pub trait AdapterSession: Send + Sync {
    /// 服务器上所有数据库/Catalog 的名字，当前连接的那个也在里面——对象浏览器
    /// 用它在 Schema 树上方补一层"数据库"（2026-09 用户反馈：之前直接从
    /// Schema 开始，缺了这一层，看不出服务器上还有哪些库）。
    async fn list_databases(&self) -> Result<Vec<String>, AppError>;
    async fn list_objects(&self) -> Result<Vec<ObjectRef>, AppError>;
    async fn describe_object(&self, object: &ObjectRef) -> Result<ObjectDefinition, AppError>;

    /// 直接执行并等待结果——用于 `Normal` 分类的语句（只读查询、INSERT）。
    /// 调用方（`executor::QueryExecutor`）负责把这个 future `tokio::spawn`
    /// 出去做到不阻塞 UI，以及配合 `backend_handle` 做真正的取消。
    async fn execute_sql(
        &self,
        sql: &str,
        row_limit: usize,
        backend_handle: BackendHandleSlot,
    ) -> Result<ExecuteResult, AppError>;

    /// 用 `execute_sql` 存进 `BackendHandleSlot` 的信息，在服务端强制终止
    /// 这条语句（`SELECT pg_cancel_backend()`/`KILL QUERY`/`KILL <spid>`）。
    async fn kill_backend(&self, handle: &str) -> Result<(), AppError>;

    /// `Transaction` 分类的语句（UPDATE/DELETE）：在独立检出的连接上开事务、
    /// 执行、返回真实受影响行数，事务保持挂起直到 `commit_write`/
    /// `rollback_write`。
    async fn begin_write(&self, sql: &str) -> Result<PendingWrite, AppError>;
    async fn commit_write(&self, pending_id: Uuid) -> Result<ExecuteResult, AppError>;
    async fn rollback_write(&self, pending_id: Uuid) -> Result<(), AppError>;

    /// 挂起事务的存活时长超过这个阈值就应该被 `service` 层的懒清扫回收
    /// （自动 rollback），避免用户开了确认框却一直不点，连接被永久占着
    /// （方案 §10 风险表"多窗口/多会话资源泄漏"）。返回超时前应该被回收的
    /// pending_id 列表。
    async fn sweep_stale_pending_writes(&self, max_age_secs: u64) -> Vec<Uuid>;
}
