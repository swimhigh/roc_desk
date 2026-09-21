use std::collections::HashMap;
use std::sync::Mutex as StdMutex;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use mysql_async::prelude::Queryable;
use mysql_async::{Opts, OptsBuilder, Pool, Row as MyRow, SslOpts, Value as MyValue};
use uuid::Uuid;

use crate::error::AppError;
use crate::sql::adapter::{AdapterSession, BackendHandleSlot, DatabaseAdapter};
use crate::sql::model::*;

pub struct MySqlAdapter;

fn to_app_err(e: mysql_async::Error) -> AppError {
    // Preserve the driver error verbatim.  TDSQL exposes a MySQL-compatible
    // proxy, and failures can happen before authentication (TCP/handshake),
    // during authentication, or while selecting the schema.  Keeping the
    // original text is essential because the command layer has a timeout
    // fallback and otherwise all three cases look identical in the UI.
    AppError::Connection(e.to_string())
}

fn build_opts(profile: &ResolvedProfile) -> Result<Opts, AppError> {
    let mut builder = OptsBuilder::default()
        .ip_or_hostname(profile.host.clone())
        .tcp_port(profile.port);
    if let Some(user) = &profile.username {
        builder = builder.user(Some(user.clone()));
    }
    if let Some(pass) = &profile.password {
        builder = builder.pass(Some(pass.clone()));
    }
    if let Some(db) = &profile.database_name {
        builder = builder.db_name(Some(db.clone()));
    }
    if profile.ssl_required {
        builder = builder.ssl_opts(Some(SslOpts::default()));
    }
    Ok(Opts::from(builder))
}

#[async_trait]
impl DatabaseAdapter for MySqlAdapter {
    fn kind(&self) -> DbKind {
        DbKind::Mysql
    }

    async fn test_connection(&self, profile: &ResolvedProfile) -> Result<DbInfo, AppError> {
        let opts = build_opts(profile)?;
        let start = Instant::now();
        let pool = Pool::new(opts);
        let mut conn = pool
            .get_conn()
            .await
            .map_err(|e| AppError::Connection(format!("MySQL TCP/握手阶段失败：{e}")))?;
        let version: String = conn
            .query_first("SELECT VERSION()")
            .await
            .map_err(|e| AppError::Database(format!("MySQL 握手后查询失败：{e}")))?
            .unwrap_or_default();
        let latency_ms = start.elapsed().as_millis() as u64;
        // `Pool::disconnect` waits for checked-out connections to be returned.
        // Keep the test connection scoped separately and drop it first, or a
        // successful probe is reported as the command's 20s timeout.
        drop(conn);
        let _ = pool.disconnect().await;
        Ok(DbInfo {
            version,
            latency_ms,
        })
    }

    async fn open_session(
        &self,
        profile: &ResolvedProfile,
    ) -> Result<std::sync::Arc<dyn AdapterSession>, AppError> {
        let opts = build_opts(profile)?;
        // 每个数据源一个共享连接池，池大小给保守上限（方案 §2.5，参考
        // rainfrog 对 Postgres 池 `max_connections(3)` 的设计），避免用户
        // 开多个标签页时把数据库连接数打满。
        let opts = OptsBuilder::from_opts(opts)
            .pool_opts(mysql_async::PoolOpts::default().with_constraints(
                mysql_async::PoolConstraints::new(0, 8).expect("valid pool constraints"),
            ));
        let pool = Pool::new(opts);
        Ok(std::sync::Arc::new(MySqlSession {
            pool,
            pending: StdMutex::new(HashMap::new()),
        }))
    }
}

struct PendingTx {
    conn: mysql_async::Conn,
    created_at: Instant,
}

pub struct MySqlSession {
    pool: Pool,
    pending: StdMutex<HashMap<Uuid, PendingTx>>,
}

fn value_to_cell(value: &MyValue) -> Cell {
    match value {
        MyValue::NULL => Cell::null(),
        MyValue::Bytes(bytes) => match std::str::from_utf8(bytes) {
            Ok(s) => Cell::text(s.to_string()),
            Err(_) => Cell::binary_hex(bytes),
        },
        MyValue::Int(i) => Cell::text(i.to_string()),
        MyValue::UInt(u) => Cell::text(u.to_string()),
        MyValue::Float(f) => Cell::text(f.to_string()),
        MyValue::Double(d) => Cell::text(d.to_string()),
        MyValue::Date(y, mo, d, h, mi, s, micro) => Cell::text(format!(
            "{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}.{micro:06}"
        )),
        MyValue::Time(neg, d, h, mi, s, micro) => {
            let sign = if *neg { "-" } else { "" };
            Cell::text(format!("{sign}{d}d {h:02}:{mi:02}:{s:02}.{micro:06}"))
        }
    }
}

fn row_to_cells(row: &MyRow) -> Vec<Cell> {
    (0..row.len())
        .map(|i| match row.as_ref(i) {
            Some(v) => value_to_cell(v),
            None => Cell::null(),
        })
        .collect()
}

async fn run_query_on_conn(
    conn: &mut mysql_async::Conn,
    sql: &str,
    row_limit: usize,
) -> Result<ExecuteResult, AppError> {
    let start = Instant::now();
    let mut result = conn.query_iter(sql).await.map_err(to_app_err)?;
    let columns: Vec<ColumnInfo> = result
        .columns()
        .map(|cols| {
            cols.iter()
                .map(|c| ColumnInfo {
                    name: c.name_str().to_string(),
                    type_name: format!("{:?}", c.column_type()),
                })
                .collect()
        })
        .unwrap_or_default();

    let mut rows = Vec::new();
    let mut truncated = false;
    while let Some(row) = result.next().await.map_err(to_app_err)? {
        if rows.len() >= row_limit {
            truncated = true;
            // 已经拿到限制行数之后仍要把剩余结果读空——mysql_async 同一个
            // 连接在结果集没消费完之前不能发下一条语句（见 `QueryResult`
            // 文档注释），`drop_result` 之类的清理交给 `result` 被 drop 时
            // 自动处理，这里只是不再往 `rows` 里塞。
            continue;
        }
        rows.push(row_to_cells(&row));
    }
    let rows_affected = if columns.is_empty() {
        Some(result.affected_rows())
    } else {
        None
    };
    drop(result);

    Ok(ExecuteResult {
        columns,
        rows,
        rows_affected,
        truncated,
        duration_ms: start.elapsed().as_millis() as u64,
    })
}

#[async_trait]
impl AdapterSession for MySqlSession {
    async fn list_databases(&self) -> Result<Vec<String>, AppError> {
        let mut conn = self.pool.get_conn().await.map_err(to_app_err)?;
        let rows: Vec<String> = conn
            .query(
                "SELECT schema_name FROM information_schema.schemata \
                 WHERE schema_name NOT IN ('information_schema','mysql','performance_schema','sys') \
                 ORDER BY schema_name",
            )
            .await
            .map_err(to_app_err)?;
        Ok(rows)
    }

    async fn list_objects(&self) -> Result<Vec<ObjectRef>, AppError> {
        let mut conn = self.pool.get_conn().await.map_err(to_app_err)?;
        let rows: Vec<(String, String, String)> = conn
            .query(
                "SELECT table_schema, table_name, table_type FROM information_schema.tables \
                 WHERE table_schema NOT IN ('information_schema','mysql','performance_schema','sys') \
                 ORDER BY table_schema, table_name",
            )
            .await
            .map_err(to_app_err)?;
        Ok(rows
            .into_iter()
            .map(|(schema, name, table_type)| ObjectRef {
                schema,
                name,
                kind: if table_type.eq_ignore_ascii_case("VIEW") {
                    ObjectKind::View
                } else {
                    ObjectKind::Table
                },
            })
            .collect())
    }

    async fn describe_object(&self, object: &ObjectRef) -> Result<ObjectDefinition, AppError> {
        let mut conn = self.pool.get_conn().await.map_err(to_app_err)?;
        let columns: Vec<(String, String, String, Option<String>, String, Option<String>)> = conn
            .exec(
                "SELECT column_name, column_type, is_nullable, column_default, column_key, \
                        NULLIF(column_comment, '') \
                 FROM information_schema.columns WHERE table_schema = ? AND table_name = ? \
                 ORDER BY ordinal_position",
                (&object.schema, &object.name),
            )
            .await
            .map_err(to_app_err)?;
        let columns = columns
            .into_iter()
            .map(|(name, data_type, nullable, default_value, key, comment)| ColumnDef {
                name,
                data_type,
                nullable: nullable.eq_ignore_ascii_case("YES"),
                default_value,
                is_primary_key: key == "PRI",
                comment,
            })
            .collect();

        let table_comment: Option<String> = conn
            .exec_first(
                "SELECT NULLIF(table_comment, '') FROM information_schema.tables \
                 WHERE table_schema = ? AND table_name = ?",
                (&object.schema, &object.name),
            )
            .await
            .map_err(to_app_err)?
            .flatten();

        let indexes: Vec<(String, String)> = conn
            .exec(
                "SELECT index_name, GROUP_CONCAT(column_name ORDER BY seq_in_index) \
                 FROM information_schema.statistics WHERE table_schema = ? AND table_name = ? \
                 GROUP BY index_name",
                (&object.schema, &object.name),
            )
            .await
            .map_err(to_app_err)?;
        let indexes = indexes
            .into_iter()
            .map(|(name, cols)| IndexDef {
                definition: format!("({cols})"),
                name,
            })
            .collect();

        Ok(ObjectDefinition {
            object: object.clone(),
            columns,
            indexes,
            ddl: None,
            comment: table_comment,
        })
    }

    async fn execute_sql(
        &self,
        sql: &str,
        row_limit: usize,
        backend_handle: BackendHandleSlot,
    ) -> Result<ExecuteResult, AppError> {
        let mut conn = self.pool.get_conn().await.map_err(to_app_err)?;
        let conn_id: u64 = conn
            .query_first("SELECT CONNECTION_ID()")
            .await
            .map_err(to_app_err)?
            .unwrap_or(0);
        *backend_handle.lock().unwrap() = Some(conn_id.to_string());
        run_query_on_conn(&mut conn, sql, row_limit).await
    }

    async fn kill_backend(&self, handle: &str) -> Result<(), AppError> {
        let mut conn = self.pool.get_conn().await.map_err(to_app_err)?;
        conn.query_drop(format!("KILL QUERY {handle}"))
            .await
            .map_err(to_app_err)
    }

    async fn begin_write(&self, sql: &str) -> Result<PendingWrite, AppError> {
        let mut conn = self.pool.get_conn().await.map_err(to_app_err)?;
        conn.query_drop("START TRANSACTION")
            .await
            .map_err(to_app_err)?;
        let preview = match run_query_on_conn(&mut conn, sql, 0).await {
            Ok(result) => result,
            Err(e) => {
                let _ = conn.query_drop("ROLLBACK").await;
                return Err(e);
            }
        };
        let pending_id = Uuid::new_v4();
        self.pending.lock().unwrap().insert(
            pending_id,
            PendingTx {
                conn,
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
        let mut conn = pending.conn;
        conn.query_drop("COMMIT").await.map_err(to_app_err)?;
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
        let mut conn = pending.conn;
        conn.query_drop("ROLLBACK").await.map_err(to_app_err)
    }

    async fn sweep_stale_pending_writes(&self, max_age_secs: u64) -> Vec<Uuid> {
        let stale: Vec<(Uuid, mysql_async::Conn)> = {
            let mut guard = self.pending.lock().unwrap();
            let stale_ids: Vec<Uuid> = guard
                .iter()
                .filter(|(_, tx)| tx.created_at.elapsed() > Duration::from_secs(max_age_secs))
                .map(|(id, _)| *id)
                .collect();
            stale_ids
                .into_iter()
                .filter_map(|id| guard.remove(&id).map(|tx| (id, tx.conn)))
                .collect()
        };
        let mut swept = Vec::with_capacity(stale.len());
        for (id, mut conn) in stale {
            let _ = conn.query_drop("ROLLBACK").await;
            swept.push(id);
        }
        swept
    }
}
