use std::collections::HashMap;
use std::sync::Mutex as StdMutex;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use deadpool_postgres::{Manager, ManagerConfig, Pool as DpPool, RecyclingMethod};
use futures_util::StreamExt;
use tokio_postgres::config::SslMode;
use tokio_postgres::{Config as PgConfig, SimpleQueryMessage};
use tokio_postgres_rustls::MakeRustlsConnect;
use uuid::Uuid;

use crate::error::AppError;
use crate::sql::adapter::{AdapterSession, BackendHandleSlot, DatabaseAdapter};
use crate::sql::model::*;

fn to_app_err(e: tokio_postgres::Error) -> AppError {
    AppError::Database(e.to_string())
}

fn build_tls_connector() -> Result<MakeRustlsConnect, AppError> {
    let mut roots = rustls::RootCertStore::empty();
    let loaded = rustls_native_certs::load_native_certs();
    for cert in loaded.certs {
        let _ = roots.add(cert);
    }
    let config = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(MakeRustlsConnect::new(config))
}

fn build_pg_config(profile: &ResolvedProfile) -> PgConfig {
    let mut config = PgConfig::new();
    config.host(&profile.host).port(profile.port);
    if let Some(db) = &profile.database_name {
        config.dbname(db);
    }
    if let Some(user) = &profile.username {
        config.user(user);
    }
    if let Some(pass) = &profile.password {
        config.password(pass);
    }
    // `Prefer` 不是"server 支持就用、验证失败就退回明文"——tokio-postgres 一旦
    // server 同意协商 TLS 就会真的走握手，验证失败直接连接失败，不会重试明文
    // （只有 server 在协议层直接拒绝 SSL 请求才会退回明文）。内网自建
    // PostgreSQL/openGauss 经常挂着自签证书，用户没勾"强制 TLS"时的合理预期是
    // "别管证书，直接连"，而不是"帮我尝试验证一次证书，验证不过就报错"——
    // 2026-09 用户实测踩过这个坑（`error performing TLS handshake`）。所以
    // 未勾选时用 `Disable` 彻底不发起 TLS 协商，勾选时才用 `Require` 强制要求
    // 且做系统证书库校验（方案 §7"禁止默认关闭证书校验"约束的是"默认"，这里
    // 默认值本来就是未勾选=明文，校验只在用户主动要求 TLS 时才生效，语义上
    // 没有违反——真正的"默认关闭校验"是指默认开着 TLS 却跳过验证）。
    config.ssl_mode(if profile.ssl_required {
        SslMode::Require
    } else {
        SslMode::Disable
    });
    config
}

pub struct PostgresAdapter {
    /// openGauss 走 PG 线协议，方言差异只体现在对象列表查询的兼容性
    /// 探测上（docs/SQL_DESKTOP_PLAN.md §4.3："禁止假设所有 PG 扩展存在"）。
    pub opengauss: bool,
}

#[async_trait]
impl DatabaseAdapter for PostgresAdapter {
    fn kind(&self) -> DbKind {
        if self.opengauss {
            DbKind::Opengauss
        } else {
            DbKind::Postgres
        }
    }

    async fn test_connection(&self, profile: &ResolvedProfile) -> Result<DbInfo, AppError> {
        let config = build_pg_config(profile);
        let tls = build_tls_connector()?;
        let start = Instant::now();
        let (client, connection) = config.connect(tls).await.map_err(to_app_err)?;
        tokio::spawn(async move {
            let _ = connection.await;
        });
        let result = run_query_on_conn(&client, "SELECT version()", 1).await?;
        let version = first_cell_text(&result);
        Ok(DbInfo {
            version,
            latency_ms: start.elapsed().as_millis() as u64,
        })
    }

    async fn open_session(
        &self,
        profile: &ResolvedProfile,
    ) -> Result<std::sync::Arc<dyn AdapterSession>, AppError> {
        let config = build_pg_config(profile);
        let tls = build_tls_connector()?;
        let manager = Manager::from_config(
            config,
            tls,
            ManagerConfig {
                recycling_method: RecyclingMethod::Fast,
            },
        );
        // 每个数据源一个共享连接池，池大小给保守上限（方案 §2.5，参考 rainfrog
        // 对 Postgres 池 `max_connections(3)` 的设计）。
        let pool = DpPool::builder(manager)
            .max_size(8)
            .build()
            .map_err(|e| AppError::Internal(format!("连接池创建失败：{e}")))?;
        Ok(std::sync::Arc::new(PgSession {
            pool,
            pending: StdMutex::new(HashMap::new()),
        }))
    }
}

struct PendingTx {
    conn: deadpool_postgres::Object,
    created_at: Instant,
}

pub struct PgSession {
    pool: DpPool,
    pending: StdMutex<HashMap<Uuid, PendingTx>>,
}

/// bytea 的简单查询协议文本形如 `\x0a1b...`——没法从纯文本里拿到真实类型
/// 信息（简单查询协议不带 OID），只能靠这个格式启发式判断"这看起来是不是
/// 十六进制编码的二进制数据"，和 rainfrog 用类型信息判断相比不够精确，但
/// 足够覆盖 bytea 这个最常见的场景。
fn looks_like_pg_bytea_hex(text: &str) -> bool {
    text.len() >= 2
        && text.starts_with("\\x")
        && text[2..].chars().all(|c| c.is_ascii_hexdigit())
}

fn simple_row_to_cells(row: &tokio_postgres::SimpleQueryRow, column_count: usize) -> Vec<Cell> {
    (0..column_count)
        .map(|i| match row.get(i) {
            None => Cell::null(),
            Some(text) if looks_like_pg_bytea_hex(text) => Cell::binary_hex(
                &(2..text.len())
                    .step_by(2)
                    .filter_map(|start| u8::from_str_radix(&text[start..start + 2], 16).ok())
                    .collect::<Vec<u8>>(),
            ),
            Some(text) => Cell::text(text.to_string()),
        })
        .collect()
}

/// 元数据/内部查询也一律走这条路径，不用 `client.query`/`query_one`（扩展协议，
/// 强类型反序列化）——2026-09 实测在 openGauss 上 `client.query_one("SELECT
/// pg_backend_pid()", &[])` 会直接 panic（`error retrieving column 0: error
/// deserializing column 0`）：openGauss 这类 PG 分支/兼容库对某些内置函数
/// 返回值的 OID 上报和原生 PostgreSQL 不完全一致，tokio-postgres 的
/// `FromSql` 严格按 OID 校验类型，对不上就直接 panic，而不是返回一个可以
/// `?` 掉的 `Result`。简单查询协议（文本）没有这个强类型校验环节，所有值都是
/// 字符串，天然规避了这一整类"方言 OID 和标准 PostgreSQL 对不上"的兼容性
/// 问题（方案 §4.3"禁止假设所有 PG 扩展存在"）。代价是拿不到列的原生类型名
/// （`ColumnInfo.type_name` 留空）和没法用 `$1` 占位符绑参数——内部查询的
/// 参数都是我们自己拼出来的标识符/受控值，用 `escape_literal` 转义后直接
/// 拼进 SQL 文本即可，不是拼用户任意输入。
async fn run_query_on_conn(
    client: &tokio_postgres::Client,
    sql: &str,
    row_limit: usize,
) -> Result<ExecuteResult, AppError> {
    let start = Instant::now();
    let stream = client.simple_query_raw(sql).await.map_err(to_app_err)?;
    tokio::pin!(stream);

    let mut columns: Vec<ColumnInfo> = Vec::new();
    let mut rows = Vec::new();
    let mut rows_affected: Option<u64> = None;
    let mut truncated = false;

    while let Some(message) = stream.next().await {
        match message.map_err(to_app_err)? {
            SimpleQueryMessage::RowDescription(cols) => {
                columns = cols
                    .iter()
                    .map(|c| ColumnInfo {
                        name: c.name().to_string(),
                        type_name: String::new(),
                    })
                    .collect();
            }
            SimpleQueryMessage::Row(row) => {
                let column_count = if columns.is_empty() {
                    row.len()
                } else {
                    columns.len()
                };
                if rows.len() >= row_limit {
                    truncated = true;
                    continue;
                }
                rows.push(simple_row_to_cells(&row, column_count));
            }
            SimpleQueryMessage::CommandComplete(n) => {
                rows_affected = Some(n);
            }
            _ => {}
        }
    }

    Ok(ExecuteResult {
        columns,
        rows,
        rows_affected,
        truncated,
        duration_ms: start.elapsed().as_millis() as u64,
    })
}

/// 单元格取值的安全下标访问——正常情况下不会越界，但"某一行列数比预期少"
/// 理论上可能发生（比如方言返回了意料之外的结果形状），用 `unwrap_or_default`
/// 而不是索引 panic，元数据查询解析失败不应该带崩整个查询任务。
fn cell_text(row: &[Cell], idx: usize) -> String {
    row.get(idx).map(|c| c.text.clone()).unwrap_or_default()
}

fn cell_text_opt(row: &[Cell], idx: usize) -> Option<String> {
    row.get(idx).filter(|c| !c.is_null).map(|c| c.text.clone())
}

fn first_cell_text(result: &ExecuteResult) -> String {
    result.rows.first().map(|r| cell_text(r, 0)).unwrap_or_default()
}

/// 内部拼 SQL 用的字符串字面量转义——这里传入的值都是我们自己已经从数据库
/// 元数据里读回来的 schema/table 名，不是用户输入，转义只是防御性的最低
/// 成本处理，不是应对恶意输入的完整方案。
fn escape_literal(s: &str) -> String {
    s.replace('\'', "''")
}

#[async_trait]
impl AdapterSession for PgSession {
    async fn list_databases(&self) -> Result<Vec<String>, AppError> {
        let client = self.pool.get().await.map_err(|e| AppError::Database(e.to_string()))?;
        let result = run_query_on_conn(
            &client,
            "select datname from pg_database where datistemplate = false order by datname",
            usize::MAX,
        )
        .await?;
        Ok(result.rows.into_iter().map(|row| cell_text(&row, 0)).collect())
    }

    async fn list_objects(&self) -> Result<Vec<ObjectRef>, AppError> {
        let client = self.pool.get().await.map_err(|e| AppError::Database(e.to_string()))?;
        // 优先用 pg_catalog（能顺带拿到物化视图/函数），openGauss 或权限受限
        // 场景下可能不完全兼容，退回 information_schema 版本（参考 rainfrog
        // `MENU_QUERY`/`SIMPLE_MENU_QUERY` 双查询兜底的思路，见方案 §4.2.1）。
        const RICH_QUERY: &str = "select n.nspname as schema, c.relname as name, \
            case when c.relkind in ('r','p','f') then 'table' \
                 when c.relkind = 'v' then 'view' \
                 when c.relkind = 'm' then 'materialized_view' end as kind \
            from pg_class c join pg_namespace n on n.oid = c.relnamespace \
            where n.nspname not in ('pg_catalog','information_schema') \
              and c.relkind in ('r','p','f','v','m') \
            order by n.nspname, c.relname";
        const FALLBACK_QUERY: &str = "select table_schema as schema, table_name as name, \
            case when table_type = 'VIEW' then 'view' else 'table' end as kind \
            from information_schema.tables \
            where table_schema not in ('pg_catalog','information_schema') \
            order by table_schema, table_name";

        let result = match run_query_on_conn(&client, RICH_QUERY, usize::MAX).await {
            Ok(r) => r,
            Err(_) => run_query_on_conn(&client, FALLBACK_QUERY, usize::MAX).await?,
        };
        Ok(result
            .rows
            .into_iter()
            .map(|row| {
                let kind_str = cell_text(&row, 2);
                ObjectRef {
                    schema: cell_text(&row, 0),
                    name: cell_text(&row, 1),
                    kind: match kind_str.as_str() {
                        "view" => ObjectKind::View,
                        "materialized_view" => ObjectKind::MaterializedView,
                        _ => ObjectKind::Table,
                    },
                }
            })
            .collect())
    }

    async fn describe_object(&self, object: &ObjectRef) -> Result<ObjectDefinition, AppError> {
        let client = self.pool.get().await.map_err(|e| AppError::Database(e.to_string()))?;
        let schema = escape_literal(&object.schema);
        let name = escape_literal(&object.name);

        let column_rows = run_query_on_conn(
            &client,
            &format!(
                "select c.column_name, c.data_type, c.is_nullable, c.column_default, \
                        col_description(format('%I.%I', c.table_schema, c.table_name)::regclass::oid, c.ordinal_position) \
                 from information_schema.columns c \
                 where c.table_schema = '{schema}' and c.table_name = '{name}' \
                 order by c.ordinal_position"
            ),
            usize::MAX,
        )
        .await?;
        let pk_rows = run_query_on_conn(
            &client,
            &format!(
                "select kcu.column_name from information_schema.table_constraints tc \
                 join information_schema.key_column_usage kcu \
                   on tc.constraint_name = kcu.constraint_name and tc.table_schema = kcu.table_schema \
                 where tc.table_schema = '{schema}' and tc.table_name = '{name}' and tc.constraint_type = 'PRIMARY KEY'"
            ),
            usize::MAX,
        )
        .await?;
        let pk_names: std::collections::HashSet<String> =
            pk_rows.rows.iter().map(|r| cell_text(r, 0)).collect();

        let columns = column_rows
            .rows
            .into_iter()
            .map(|row| {
                let name = cell_text(&row, 0);
                ColumnDef {
                    is_primary_key: pk_names.contains(&name),
                    name,
                    data_type: cell_text(&row, 1),
                    nullable: cell_text(&row, 2) == "YES",
                    default_value: cell_text_opt(&row, 3),
                    comment: cell_text_opt(&row, 4),
                }
            })
            .collect();

        let comment_result = run_query_on_conn(
            &client,
            &format!("select obj_description('{schema}.{name}'::regclass::oid, 'pg_class')"),
            1,
        )
        .await
        .ok();
        let comment = comment_result.and_then(|r| r.rows.first().and_then(|row| cell_text_opt(row, 0)));

        let index_result = run_query_on_conn(
            &client,
            &format!(
                "select indexname, indexdef from pg_indexes where schemaname = '{schema}' and tablename = '{name}'"
            ),
            usize::MAX,
        )
        .await?;
        let indexes = index_result
            .rows
            .into_iter()
            .map(|row| IndexDef {
                name: cell_text(&row, 0),
                definition: cell_text(&row, 1),
            })
            .collect();

        let ddl = if matches!(object.kind, ObjectKind::View | ObjectKind::MaterializedView) {
            let relkind = if matches!(object.kind, ObjectKind::MaterializedView) {
                "m"
            } else {
                "v"
            };
            let view_def = run_query_on_conn(
                &client,
                &format!(
                    "select pg_get_viewdef(c.oid, true) from pg_class c \
                     join pg_namespace n on n.oid = c.relnamespace \
                     where n.nspname = '{schema}' and c.relname = '{name}' and c.relkind = '{relkind}'"
                ),
                1,
            )
            .await?;
            view_def.rows.first().and_then(|r| cell_text_opt(r, 0))
        } else {
            None
        };

        Ok(ObjectDefinition {
            object: object.clone(),
            columns,
            indexes,
            ddl,
            comment,
        })
    }

    async fn execute_sql(
        &self,
        sql: &str,
        row_limit: usize,
        backend_handle: BackendHandleSlot,
    ) -> Result<ExecuteResult, AppError> {
        let client = self.pool.get().await.map_err(|e| AppError::Database(e.to_string()))?;
        let pid_result = run_query_on_conn(&client, "SELECT pg_backend_pid()", 1).await?;
        let pid_text = first_cell_text(&pid_result);
        if !pid_text.is_empty() {
            *backend_handle.lock().unwrap() = Some(pid_text);
        }
        run_query_on_conn(&client, sql, row_limit).await
    }

    async fn kill_backend(&self, handle: &str) -> Result<(), AppError> {
        if handle.is_empty() || !handle.chars().all(|c| c.is_ascii_digit()) {
            return Err(AppError::Internal(format!("无效的 backend handle：{handle}")));
        }
        let client = self.pool.get().await.map_err(|e| AppError::Database(e.to_string()))?;
        run_query_on_conn(&client, &format!("SELECT pg_cancel_backend({handle})"), 1).await?;
        Ok(())
    }

    async fn begin_write(&self, sql: &str) -> Result<PendingWrite, AppError> {
        let conn = self.pool.get().await.map_err(|e| AppError::Database(e.to_string()))?;
        conn.batch_execute("BEGIN").await.map_err(to_app_err)?;
        let preview = match run_query_on_conn(&conn, sql, 0).await {
            Ok(result) => result,
            Err(e) => {
                let _ = conn.batch_execute("ROLLBACK").await;
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
        pending
            .conn
            .batch_execute("COMMIT")
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
        pending
            .conn
            .batch_execute("ROLLBACK")
            .await
            .map_err(to_app_err)
    }

    async fn sweep_stale_pending_writes(&self, max_age_secs: u64) -> Vec<Uuid> {
        let stale: Vec<(Uuid, deadpool_postgres::Object)> = {
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
        for (id, conn) in stale {
            let _ = conn.batch_execute("ROLLBACK").await;
            swept.push(id);
        }
        swept
    }
}
