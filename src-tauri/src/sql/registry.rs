use std::sync::Arc;

use crate::error::AppError;
use crate::sql::adapter::DatabaseAdapter;
use crate::sql::adapters::{mysql::MySqlAdapter, postgres::PostgresAdapter, sqlserver::SqlServerAdapter};
use crate::sql::model::DbKind;

/// 类型 -> adapter 工厂（docs/SQL_DESKTOP_PLAN.md §4.1 `registry.rs`）。Oracle
/// 首期不接（需要本机装 OCI/Instant Client，见方案风险表），命中时直接给出
/// 明确的"未实现"错误，不影响其它数据库正常使用。
pub fn create_adapter(kind: DbKind) -> Result<Arc<dyn DatabaseAdapter>, AppError> {
    match kind {
        DbKind::Mysql | DbKind::Tdsql => Ok(Arc::new(MySqlAdapter)),
        DbKind::Postgres => Ok(Arc::new(PostgresAdapter { opengauss: false })),
        DbKind::Opengauss => Ok(Arc::new(PostgresAdapter { opengauss: true })),
        DbKind::SqlServer => Ok(Arc::new(SqlServerAdapter)),
        DbKind::Oracle => Err(AppError::Internal(
            "Oracle 适配器暂未接入（需要本机安装 OCI/Instant Client），敬请期待".into(),
        )),
    }
}
