use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::credential::CredentialStore;
use crate::db::repo::sql_data_sources_repo::SqlDataSourcesRepo;
use crate::error::AppError;
use crate::sql::adapter::AdapterSession;
use crate::sql::model::*;
use crate::sql::registry;

fn credential_key(id: Uuid) -> String {
    format!("sql:{id}:password")
}

/// 数据源 CRUD + 凭据编排（对齐 `ai::providers::AiProviderManager` 的既有模式，
/// 见 docs/SQL_DESKTOP_PLAN.md §2.4）。
pub struct SqlDataSourceService {
    repo: Arc<SqlDataSourcesRepo>,
    credential_store: Arc<dyn CredentialStore>,
}

impl SqlDataSourceService {
    pub fn new(repo: Arc<SqlDataSourcesRepo>, credential_store: Arc<dyn CredentialStore>) -> Self {
        Self {
            repo,
            credential_store,
        }
    }

    pub async fn create(&self, input: DataSourceInput) -> Result<DataSourceProfile, AppError> {
        let id = Uuid::new_v4();
        let credential_ref = match &input.password {
            Some(pwd) if !pwd.is_empty() => {
                let key = credential_key(id);
                self.credential_store.set(&key, pwd).await?;
                Some(key)
            }
            _ => None,
        };
        let now = Utc::now().to_rfc3339();
        let profile = DataSourceProfile {
            id,
            name: input.name,
            db_kind: input.db_kind,
            host: input.host,
            port: input.port.or_else(|| input.db_kind.default_port()),
            database_name: input.database_name,
            default_schema: input.default_schema,
            username: input.username,
            credential_ref,
            environment: input.environment,
            group_name: input.group_name,
            readonly: input.readonly,
            ssl_required: input.ssl_required,
            created_at: now.clone(),
            updated_at: now,
            last_used_at: None,
        };
        self.repo.create(&profile)?;
        Ok(profile)
    }

    pub async fn update(&self, id: Uuid, input: DataSourceInput) -> Result<DataSourceProfile, AppError> {
        let existing = self
            .repo
            .get(id)?
            .ok_or_else(|| AppError::NotFound(format!("data source not found: {id}")))?;
        let credential_ref = match &input.password {
            Some(pwd) if !pwd.is_empty() => {
                let key = existing.credential_ref.clone().unwrap_or_else(|| credential_key(id));
                self.credential_store.set(&key, pwd).await?;
                Some(key)
            }
            _ => existing.credential_ref,
        };
        let profile = DataSourceProfile {
            id,
            name: input.name,
            db_kind: input.db_kind,
            host: input.host,
            port: input.port.or_else(|| input.db_kind.default_port()),
            database_name: input.database_name,
            default_schema: input.default_schema,
            username: input.username,
            credential_ref,
            environment: input.environment,
            group_name: input.group_name,
            readonly: input.readonly,
            ssl_required: input.ssl_required,
            created_at: existing.created_at,
            updated_at: Utc::now().to_rfc3339(),
            last_used_at: existing.last_used_at,
        };
        self.repo.update(&profile)?;
        Ok(profile)
    }

    pub async fn delete(&self, id: Uuid) -> Result<(), AppError> {
        if let Some(existing) = self.repo.get(id)? {
            if let Some(key) = existing.credential_ref {
                self.credential_store.delete(&key).await?;
            }
        }
        self.repo.delete(id)
    }

    pub fn list(&self) -> Result<Vec<DataSourceProfile>, AppError> {
        self.repo.list()
    }

    pub fn get(&self, id: Uuid) -> Result<Option<DataSourceProfile>, AppError> {
        self.repo.get(id)
    }

    pub fn touch_last_used(&self, id: Uuid) -> Result<(), AppError> {
        self.repo.touch_last_used(id, &Utc::now().to_rfc3339())
    }

    /// 拿真实密码建连接用——绝不能把结果 `Serialize` 传回前端。
    pub async fn resolve(&self, id: Uuid) -> Result<ResolvedProfile, AppError> {
        let profile = self
            .repo
            .get(id)?
            .ok_or_else(|| AppError::NotFound(format!("data source not found: {id}")))?;
        let password = match &profile.credential_ref {
            Some(key) => self.credential_store.get(key).await?,
            None => None,
        };
        Ok(ResolvedProfile {
            id: profile.id,
            db_kind: profile.db_kind,
            host: profile.host,
            port: profile.port.unwrap_or_else(|| {
                profile.db_kind.default_port().unwrap_or(0)
            }),
            database_name: profile.database_name,
            default_schema: profile.default_schema,
            username: profile.username,
            password,
            ssl_required: profile.ssl_required,
            readonly: profile.readonly,
        })
    }
}

/// 每个数据源一份共享的连接会话（跨标签页/窗口复用同一个连接池，方案
/// §2.5）；懒建立、显式关闭。
pub struct SqlSessionManager {
    ds_service: Arc<SqlDataSourceService>,
    sessions: RwLock<HashMap<Uuid, Arc<dyn AdapterSession>>>,
    /// "切换数据库"覆盖的目标库名（2026-09 用户反馈：对象浏览器缺了"数据库"
    /// 这一层，选完数据源应该还能看到/切换服务器上的其它库）。Postgres 一条
    /// 物理连接绑死一个数据库，没法像 MySQL/SQL Server 的 `USE` 那样在同一条
    /// 连接上切换，所以"切换数据库"统一实现成"关掉当前会话、记下目标库名、
    /// 下次 `get_or_open` 用这个库名重新建一份新连接池"，三种数据库行为一致，
    /// 不用为 MySQL/SQL Server 单独走 `USE` 这条更省事但语义不统一的路径。
    current_database: RwLock<HashMap<Uuid, String>>,
}

impl SqlSessionManager {
    pub fn new(ds_service: Arc<SqlDataSourceService>) -> Self {
        Self {
            ds_service,
            sessions: RwLock::new(HashMap::new()),
            current_database: RwLock::new(HashMap::new()),
        }
    }

    pub async fn get_or_open(&self, data_source_id: Uuid) -> Result<Arc<dyn AdapterSession>, AppError> {
        if let Some(session) = self.sessions.read().await.get(&data_source_id) {
            return Ok(session.clone());
        }
        let mut profile = self.ds_service.resolve(data_source_id).await?;
        if let Some(db) = self.current_database.read().await.get(&data_source_id) {
            profile.database_name = Some(db.clone());
        }
        let adapter = registry::create_adapter(profile.db_kind)?;
        let session = adapter.open_session(&profile).await?;
        self.sessions
            .write()
            .await
            .insert(data_source_id, session.clone());
        let _ = self.ds_service.touch_last_used(data_source_id);
        Ok(session)
    }

    pub async fn close(&self, data_source_id: Uuid) {
        self.sessions.write().await.remove(&data_source_id);
    }

    /// 关掉当前会话并记下目标库名，下次 `get_or_open` 会用新库名重新连接
    /// （见 `current_database` 字段文档）。
    pub async fn switch_database(&self, data_source_id: Uuid, database_name: String) {
        self.close(data_source_id).await;
        self.current_database
            .write()
            .await
            .insert(data_source_id, database_name);
    }

    /// 当前实际连着的库名——切换过就是覆盖值，没切换过就是数据源档案自带的
    /// `database_name`，供前端在数据库列表里高亮"当前"这一项。
    pub async fn current_database_name(&self, data_source_id: Uuid) -> Result<Option<String>, AppError> {
        if let Some(db) = self.current_database.read().await.get(&data_source_id) {
            return Ok(Some(db.clone()));
        }
        Ok(self
            .ds_service
            .get(data_source_id)?
            .and_then(|p| p.database_name))
    }

    /// 挂起写事务空闲超时自动回滚（方案 §10 风险表"多窗口/多会话资源泄漏"）；
    /// 每次 `sql_execute`/`sql_open_session` 顺手扫一下，不单独起后台定时器。
    pub async fn sweep_stale_pending_writes(&self, max_age_secs: u64) {
        let sessions: Vec<Arc<dyn AdapterSession>> =
            self.sessions.read().await.values().cloned().collect();
        for session in sessions {
            session.sweep_stale_pending_writes(max_age_secs).await;
        }
    }
}
