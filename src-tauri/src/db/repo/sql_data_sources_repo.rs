use rusqlite::{params, OptionalExtension};
use uuid::Uuid;

use crate::db::DbPool;
use crate::error::AppError;
use crate::sql::model::{DataSourceProfile, DbKind};

pub struct SqlDataSourcesRepo {
    pool: DbPool,
}

fn db_kind_to_str(kind: DbKind) -> &'static str {
    match kind {
        DbKind::Mysql => "mysql",
        DbKind::Tdsql => "tdsql",
        DbKind::Postgres => "postgres",
        DbKind::Opengauss => "opengauss",
        DbKind::SqlServer => "sqlserver",
        DbKind::Oracle => "oracle",
    }
}

fn db_kind_from_str(s: &str) -> DbKind {
    match s {
        "mysql" => DbKind::Mysql,
        "tdsql" => DbKind::Tdsql,
        "opengauss" => DbKind::Opengauss,
        "sqlserver" => DbKind::SqlServer,
        "oracle" => DbKind::Oracle,
        _ => DbKind::Postgres,
    }
}

impl SqlDataSourcesRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub fn create(&self, p: &DataSourceProfile) -> Result<(), AppError> {
        let conn = self.pool.get()?;
        conn.execute(
            "INSERT INTO sql_data_sources
                (id, name, db_kind, host, port, database_name, default_schema, username,
                 credential_ref, environment, group_name, readonly, ssl_required,
                 created_at, updated_at, last_used_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
            params![
                p.id.to_string(),
                p.name,
                db_kind_to_str(p.db_kind),
                p.host,
                p.port,
                p.database_name,
                p.default_schema,
                p.username,
                p.credential_ref,
                p.environment,
                p.group_name,
                p.readonly as i64,
                p.ssl_required as i64,
                p.created_at,
                p.updated_at,
                p.last_used_at,
            ],
        )?;
        Ok(())
    }

    pub fn update(&self, p: &DataSourceProfile) -> Result<(), AppError> {
        let conn = self.pool.get()?;
        conn.execute(
            "UPDATE sql_data_sources SET name=?2, db_kind=?3, host=?4, port=?5, database_name=?6,
                default_schema=?7, username=?8, credential_ref=?9, environment=?10, group_name=?11,
                readonly=?12, ssl_required=?13, updated_at=?14
             WHERE id=?1",
            params![
                p.id.to_string(),
                p.name,
                db_kind_to_str(p.db_kind),
                p.host,
                p.port,
                p.database_name,
                p.default_schema,
                p.username,
                p.credential_ref,
                p.environment,
                p.group_name,
                p.readonly as i64,
                p.ssl_required as i64,
                p.updated_at,
            ],
        )?;
        Ok(())
    }

    pub fn touch_last_used(&self, id: Uuid, ts: &str) -> Result<(), AppError> {
        let conn = self.pool.get()?;
        conn.execute(
            "UPDATE sql_data_sources SET last_used_at = ?2 WHERE id = ?1",
            params![id.to_string(), ts],
        )?;
        Ok(())
    }

    pub fn delete(&self, id: Uuid) -> Result<(), AppError> {
        let conn = self.pool.get()?;
        conn.execute(
            "DELETE FROM sql_data_sources WHERE id = ?1",
            params![id.to_string()],
        )?;
        Ok(())
    }

    pub fn get(&self, id: Uuid) -> Result<Option<DataSourceProfile>, AppError> {
        let conn = self.pool.get()?;
        conn.query_row(
            "SELECT id,name,db_kind,host,port,database_name,default_schema,username,
                    credential_ref,environment,group_name,readonly,ssl_required,
                    created_at,updated_at,last_used_at
             FROM sql_data_sources WHERE id = ?1",
            params![id.to_string()],
            Self::map_row,
        )
        .optional()
        .map_err(AppError::from)
    }

    pub fn list(&self) -> Result<Vec<DataSourceProfile>, AppError> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT id,name,db_kind,host,port,database_name,default_schema,username,
                    credential_ref,environment,group_name,readonly,ssl_required,
                    created_at,updated_at,last_used_at
             FROM sql_data_sources ORDER BY name",
        )?;
        let rows = stmt
            .query_map([], Self::map_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    fn map_row(row: &rusqlite::Row) -> rusqlite::Result<DataSourceProfile> {
        let id: String = row.get(0)?;
        let db_kind: String = row.get(2)?;
        Ok(DataSourceProfile {
            id: Uuid::parse_str(&id).unwrap_or_else(|_| Uuid::nil()),
            name: row.get(1)?,
            db_kind: db_kind_from_str(&db_kind),
            host: row.get(3)?,
            port: row.get::<_, Option<i64>>(4)?.map(|p| p as u16),
            database_name: row.get(5)?,
            default_schema: row.get(6)?,
            username: row.get(7)?,
            credential_ref: row.get(8)?,
            environment: row.get(9)?,
            group_name: row.get(10)?,
            readonly: row.get::<_, i64>(11)? != 0,
            ssl_required: row.get::<_, i64>(12)? != 0,
            created_at: row.get(13)?,
            updated_at: row.get(14)?,
            last_used_at: row.get(15)?,
        })
    }
}
