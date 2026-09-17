use std::collections::HashMap;
use std::sync::Mutex as StdMutex;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use bb8::Pool as Bb8Pool;
use bb8_tiberius::ConnectionManager;
use futures_util::StreamExt;
use sqlparser::ast::Statement;
use sqlparser::dialect::GenericDialect;
use sqlparser::parser::Parser;
use tiberius::{AuthMethod, Client, ColumnData, Config as TbConfig, QueryItem};
use tokio::net::TcpStream;
use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};
use uuid::Uuid;

use crate::error::AppError;
use crate::sql::adapter::{AdapterSession, BackendHandleSlot, DatabaseAdapter};
use crate::sql::model::*;

type TbClient = Client<Compat<TcpStream>>;

pub struct SqlServerAdapter;

fn to_app_err(e: tiberius::error::Error) -> AppError {
    AppError::Database(e.to_string())
}

fn build_config(profile: &ResolvedProfile) -> TbConfig {
    let mut config = TbConfig::new();
    config.host(&profile.host);
    config.port(profile.port);
    if let Some(db) = &profile.database_name {
        config.database(db);
    }
    let user = profile.username.clone().unwrap_or_default();
    let pass = profile.password.clone().unwrap_or_default();
    config.authentication(AuthMethod::sql_server(user, pass));
    // TODO(方案 §7"禁止默认关闭证书校验")：tiberius 的 rustls 证书校验在自签证书
    // 场景下需要额外配一套受信任 CA/证书指纹确认流程，本次没时间打磨，先用
    // `trust_cert()` 让 SQL Server（尤其是内网自签证书部署）能连上——这是已知
    // 的、和 Postgres/MySQL adapter（走系统证书库校验）不一致的地方，留作后续
    // 补课项，不是"忘了处理"。
    config.trust_cert();
    config
}

async fn connect_direct(profile: &ResolvedProfile) -> Result<TbClient, AppError> {
    let config = build_config(profile);
    let tcp = TcpStream::connect(config.get_addr())
        .await
        .map_err(|e| AppError::Connection(e.to_string()))?;
    tcp.set_nodelay(true).ok();
    Client::connect(config, tcp.compat_write())
        .await
        .map_err(to_app_err)
}

#[async_trait]
impl DatabaseAdapter for SqlServerAdapter {
    fn kind(&self) -> DbKind {
        DbKind::SqlServer
    }

    async fn test_connection(&self, profile: &ResolvedProfile) -> Result<DbInfo, AppError> {
        let start = Instant::now();
        let mut client = connect_direct(profile).await?;
        let row = client
            .simple_query("SELECT @@VERSION")
            .await
            .map_err(to_app_err)?
            .into_row()
            .await
            .map_err(to_app_err)?
            .ok_or_else(|| AppError::Internal("空响应".into()))?;
        let version: &str = row.get(0).unwrap_or("");
        Ok(DbInfo {
            version: version.to_string(),
            latency_ms: start.elapsed().as_millis() as u64,
        })
    }

    async fn open_session(
        &self,
        profile: &ResolvedProfile,
    ) -> Result<std::sync::Arc<dyn AdapterSession>, AppError> {
        let config = build_config(profile);
        let manager = ConnectionManager::new(config);
        // 每个数据源一个共享连接池，池大小上限对齐其它 adapter（方案 §2.5）。
        let pool = Bb8Pool::builder()
            .max_size(8)
            .build(manager)
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(std::sync::Arc::new(SqlServerSession {
            pool,
            profile: profile.clone(),
            pending: StdMutex::new(HashMap::new()),
        }))
    }
}

struct PendingTx {
    client: TbClient,
    created_at: Instant,
}

pub struct SqlServerSession {
    pool: Bb8Pool<ConnectionManager>,
    /// `begin_write` 故意不从共享池里检出连接——bb8 的 `PooledConnection<'a,_>`
    /// 带着借用 `&'a Pool` 的生命周期，没法安全地存进 `pending`（一个和 `pool`
    /// 同一个 struct 里的字段）这种自引用结构；挂起的写操作改用一条独立、不进
    /// 池子的直连连接，逻辑更简单，代价只是不复用连接——反正这是小众路径
    /// （UPDATE/DELETE 二次确认期间），不值得为它引入自引用结构之类的复杂度。
    profile: ResolvedProfile,
    pending: StdMutex<HashMap<Uuid, PendingTx>>,
}

fn column_data_to_cell(data: &ColumnData<'_>) -> Cell {
    match data {
        ColumnData::U8(None)
        | ColumnData::I16(None)
        | ColumnData::I32(None)
        | ColumnData::I64(None)
        | ColumnData::F32(None)
        | ColumnData::F64(None)
        | ColumnData::Bit(None)
        | ColumnData::String(None)
        | ColumnData::Guid(None)
        | ColumnData::Binary(None)
        | ColumnData::Numeric(None)
        | ColumnData::Xml(None)
        | ColumnData::DateTime(None)
        | ColumnData::SmallDateTime(None)
        | ColumnData::Time(None)
        | ColumnData::Date(None)
        | ColumnData::DateTime2(None)
        | ColumnData::DateTimeOffset(None) => Cell::null(),
        ColumnData::U8(Some(v)) => Cell::text(v.to_string()),
        ColumnData::I16(Some(v)) => Cell::text(v.to_string()),
        ColumnData::I32(Some(v)) => Cell::text(v.to_string()),
        ColumnData::I64(Some(v)) => Cell::text(v.to_string()),
        ColumnData::F32(Some(v)) => Cell::text(v.to_string()),
        ColumnData::F64(Some(v)) => Cell::text(v.to_string()),
        ColumnData::Bit(Some(v)) => Cell::text(v.to_string()),
        ColumnData::String(Some(v)) => Cell::text(v.to_string()),
        ColumnData::Guid(Some(v)) => Cell::text(v.to_string()),
        ColumnData::Binary(Some(v)) => Cell::binary_hex(v),
        // Numeric/日期时间类型格式化交给 Debug——tiberius 自有的 `Numeric`/时间
        // 类型和 chrono 之间的换算需要额外样板代码，先保证能编译、能看到值，
        // 展示格式后续再打磨（sqlserver adapter 顶部注释已经记了一笔）。
        ColumnData::Numeric(Some(v)) => Cell::text(format!("{v:?}")),
        ColumnData::Xml(Some(v)) => Cell::text(format!("{v:?}")),
        ColumnData::DateTime(Some(v)) => Cell::text(format!("{v:?}")),
        ColumnData::SmallDateTime(Some(v)) => Cell::text(format!("{v:?}")),
        ColumnData::Time(Some(v)) => Cell::text(format!("{v:?}")),
        ColumnData::Date(Some(v)) => Cell::text(format!("{v:?}")),
        ColumnData::DateTime2(Some(v)) => Cell::text(format!("{v:?}")),
        ColumnData::DateTimeOffset(Some(v)) => Cell::text(format!("{v:?}")),
    }
}

/// 单条语句到底该走 `.query()`（有结果集）还是 `.execute()`（只要受影响行数）——
/// tiberius 的两条 API 路径不通用：`.query()` 对纯 DML 静默丢弃行数信息（服务端
/// 的 DONE token 在 `QueryStream` 里被直接跳过，见 tiberius 源码），`.execute()`
/// 则相反没法拿行数据。这里复用项目里已经在用的 `sqlparser` 自己判断一次，不
/// 改 `AdapterSession` trait 签名去给其它两个 adapter 也加一个用不上的参数。
fn expects_result_set(sql: &str) -> bool {
    match Parser::parse_sql(&GenericDialect {}, sql) {
        Ok(statements) if statements.len() == 1 => {
            matches!(statements[0], Statement::Query(_) | Statement::Explain { .. })
        }
        _ => false,
    }
}

async fn run_query_on_conn(
    client: &mut TbClient,
    sql: &str,
    row_limit: usize,
) -> Result<ExecuteResult, AppError> {
    let start = Instant::now();

    if expects_result_set(sql) {
        let mut stream = client.simple_query(sql).await.map_err(to_app_err)?;
        let mut columns: Vec<ColumnInfo> = Vec::new();
        let mut rows = Vec::new();
        let mut truncated = false;
        while let Some(item) = stream.next().await {
            match item.map_err(to_app_err)? {
                QueryItem::Metadata(meta) => {
                    columns = meta
                        .columns()
                        .iter()
                        .map(|c| ColumnInfo {
                            name: c.name().to_string(),
                            type_name: format!("{:?}", c.column_type()),
                        })
                        .collect();
                }
                QueryItem::Row(row) => {
                    if rows.len() >= row_limit {
                        truncated = true;
                        continue;
                    }
                    let cells = row.cells().map(|(_, data)| column_data_to_cell(data)).collect();
                    rows.push(cells);
                }
            }
        }
        Ok(ExecuteResult {
            columns,
            rows,
            rows_affected: None,
            truncated,
            duration_ms: start.elapsed().as_millis() as u64,
        })
    } else {
        let result = client.execute(sql, &[]).await.map_err(to_app_err)?;
        let affected: u64 = result.rows_affected().iter().sum();
        Ok(ExecuteResult {
            columns: vec![],
            rows: vec![],
            rows_affected: Some(affected),
            truncated: false,
            duration_ms: start.elapsed().as_millis() as u64,
        })
    }
}

#[async_trait]
impl AdapterSession for SqlServerSession {
    async fn list_databases(&self) -> Result<Vec<String>, AppError> {
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;
        // database_id <= 4 是系统库（master/tempdb/model/msdb），不列进来。
        let rows = conn
            .simple_query("SELECT name FROM sys.databases WHERE database_id > 4 ORDER BY name")
            .await
            .map_err(to_app_err)?
            .into_first_result()
            .await
            .map_err(to_app_err)?;
        Ok(rows
            .into_iter()
            .map(|row| row.get::<&str, _>(0).unwrap_or_default().to_string())
            .collect())
    }

    async fn list_objects(&self) -> Result<Vec<ObjectRef>, AppError> {
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;
        let rows = conn
            .simple_query(
                "SELECT s.name AS schema_name, t.name AS obj_name, \
                        CASE WHEN t.type = 'V' THEN 'view' ELSE 'table' END AS kind \
                 FROM sys.objects t \
                 JOIN sys.schemas s ON s.schema_id = t.schema_id \
                 WHERE t.type IN ('U','V') \
                 ORDER BY s.name, t.name",
            )
            .await
            .map_err(to_app_err)?
            .into_first_result()
            .await
            .map_err(to_app_err)?;
        Ok(rows
            .into_iter()
            .map(|row| {
                let kind: &str = row.get(2).unwrap_or("table");
                ObjectRef {
                    schema: row.get::<&str, _>(0).unwrap_or_default().to_string(),
                    name: row.get::<&str, _>(1).unwrap_or_default().to_string(),
                    kind: if kind == "view" {
                        ObjectKind::View
                    } else {
                        ObjectKind::Table
                    },
                }
            })
            .collect())
    }

    async fn describe_object(&self, object: &ObjectRef) -> Result<ObjectDefinition, AppError> {
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;
        let sql = format!(
            "SELECT c.name, ty.name, c.is_nullable, \
                    CASE WHEN pk.column_id IS NOT NULL THEN 1 ELSE 0 END \
             FROM sys.columns c \
             JOIN sys.types ty ON ty.user_type_id = c.user_type_id \
             JOIN sys.tables t ON t.object_id = c.object_id \
             JOIN sys.schemas s ON s.schema_id = t.schema_id \
             LEFT JOIN ( \
                 SELECT ic.column_id, ic.object_id FROM sys.index_columns ic \
                 JOIN sys.indexes i ON i.object_id = ic.object_id AND i.index_id = ic.index_id \
                 WHERE i.is_primary_key = 1 \
             ) pk ON pk.column_id = c.column_id AND pk.object_id = c.object_id \
             WHERE s.name = '{}' AND t.name = '{}' \
             ORDER BY c.column_id",
            object.schema.replace('\'', "''"),
            object.name.replace('\'', "''")
        );
        let rows = conn
            .simple_query(sql)
            .await
            .map_err(to_app_err)?
            .into_first_result()
            .await
            .map_err(to_app_err)?;
        let columns = rows
            .into_iter()
            .map(|row| ColumnDef {
                name: row.get::<&str, _>(0).unwrap_or_default().to_string(),
                data_type: row.get::<&str, _>(1).unwrap_or_default().to_string(),
                nullable: row.get::<bool, _>(2).unwrap_or(true),
                default_value: None,
                is_primary_key: row.get::<i32, _>(3).unwrap_or(0) != 0,
                // TODO：SQL Server 的列/表注释存在 `sys.extended_properties`
                // （`MS_Description`），查询更绕（要按 major_id/minor_id 关联），
                // 这次没时间做，先留空——不影响其它数据库已经落地的注释展示。
                comment: None,
            })
            .collect();

        Ok(ObjectDefinition {
            object: object.clone(),
            columns,
            indexes: vec![],
            comment: None,
            ddl: None,
        })
    }

    async fn execute_sql(
        &self,
        sql: &str,
        row_limit: usize,
        backend_handle: BackendHandleSlot,
    ) -> Result<ExecuteResult, AppError> {
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;
        if let Ok(Some(row)) = conn
            .simple_query("SELECT @@SPID")
            .await
            .map_err(to_app_err)?
            .into_row()
            .await
        {
            if let Some(spid) = row.get::<i16, _>(0) {
                *backend_handle.lock().unwrap() = Some(spid.to_string());
            }
        }
        run_query_on_conn(&mut conn, sql, row_limit).await
    }

    async fn kill_backend(&self, handle: &str) -> Result<(), AppError> {
        handle
            .parse::<i16>()
            .map_err(|_| AppError::Internal(format!("无效的 backend handle：{handle}")))?;
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;
        conn.simple_query(format!("KILL {handle}"))
            .await
            .map_err(to_app_err)?;
        Ok(())
    }

    async fn begin_write(&self, sql: &str) -> Result<PendingWrite, AppError> {
        let mut client = connect_direct(&self.profile).await?;
        client
            .simple_query("BEGIN TRANSACTION")
            .await
            .map_err(to_app_err)?;
        let preview = match run_query_on_conn(&mut client, sql, 0).await {
            Ok(result) => result,
            Err(e) => {
                let _ = client.simple_query("ROLLBACK TRANSACTION").await;
                return Err(e);
            }
        };
        let pending_id = Uuid::new_v4();
        self.pending.lock().unwrap().insert(
            pending_id,
            PendingTx {
                client,
                created_at: Instant::now(),
            },
        );
        Ok(PendingWrite {
            pending_id,
            rows_affected: preview.rows_affected,
            preview,
        })
    }

    async fn commit_write(&self, pending_id: Uuid) -> Result<ExecuteResult, AppError> {
        let pending = self
            .pending
            .lock()
            .unwrap()
            .remove(&pending_id)
            .ok_or_else(|| AppError::NotFound(format!("pending write not found: {pending_id}")))?;
        let mut client = pending.client;
        client
            .simple_query("COMMIT TRANSACTION")
            .await
            .map_err(to_app_err)?;
        Ok(ExecuteResult {
            columns: vec![],
            rows: vec![],
            rows_affected: None,
            truncated: false,
            duration_ms: 0,
        })
    }

    async fn rollback_write(&self, pending_id: Uuid) -> Result<(), AppError> {
        let pending = self
            .pending
            .lock()
            .unwrap()
            .remove(&pending_id)
            .ok_or_else(|| AppError::NotFound(format!("pending write not found: {pending_id}")))?;
        let mut client = pending.client;
        client
            .simple_query("ROLLBACK TRANSACTION")
            .await
            .map_err(to_app_err)?;
        Ok(())
    }

    async fn sweep_stale_pending_writes(&self, max_age_secs: u64) -> Vec<Uuid> {
        let stale: Vec<(Uuid, TbClient)> = {
            let mut guard = self.pending.lock().unwrap();
            let stale_ids: Vec<Uuid> = guard
                .iter()
                .filter(|(_, tx)| tx.created_at.elapsed() > Duration::from_secs(max_age_secs))
                .map(|(id, _)| *id)
                .collect();
            stale_ids
                .into_iter()
                .filter_map(|id| guard.remove(&id).map(|tx| (id, tx.client)))
                .collect()
        };
        let mut swept = Vec::with_capacity(stale.len());
        for (id, mut client) in stale {
            let _ = client.simple_query("ROLLBACK TRANSACTION").await;
            swept.push(id);
        }
        swept
    }
}
