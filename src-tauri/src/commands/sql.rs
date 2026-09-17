use std::sync::Arc;

use chrono::Utc;
use serde::Serialize;
use sqlparser::ast::Statement;
use tauri::{AppHandle, State};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::coding::{ChangeStore, CodingTarget, FileChange, FileSyncInfo};
use crate::error::AppError;
use crate::fsops::local::LocalFileOps;
use crate::fsops::FileOps;
use crate::sql::adapter::{new_backend_handle_slot, AdapterSession};
use crate::sql::data_editor::{self, AlterOp, CellInput, NamedCell};
use crate::sql::model::*;
use crate::sql::policy::{self, ExecutionKind};
use crate::sql::registry;
use crate::sql::transfer::{TransferFormat, TransferProgress};
use crate::state::AppState;

fn require_writable(profile: &DataSourceProfile) -> Result<(), AppError> {
    if profile.readonly {
        return Err(AppError::PermissionDenied(
            "该数据源已开启只读模式，拒绝执行写操作".into(),
        ));
    }
    Ok(())
}

/// `sql_execute` 的返回形状——`Started` 对应可以直接放行的语句（返回
/// `query_id`，前端轮询 `sql_poll_query`）；`NeedsConfirmation` 对应 DDL 类
/// 语句，前端弹确认框后带着 `confirmed=true` 重新调一次 `sql_execute`；
/// `PendingWrite` 对应 UPDATE/DELETE 类语句，已经在独立事务里跑完、展示真实
/// 受影响行数，调 `sql_confirm_write`/`sql_rollback_write` 收尾
/// （docs/SQL_DESKTOP_PLAN.md §4.2.1 第 3 点）。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind")]
pub enum ExecuteOutcome {
    Started { query_id: Uuid },
    NeedsConfirmation,
    PendingWrite(PendingWrite),
}

/// 挂起写事务超过这个时长没确认就自动回滚（方案 §10"多窗口/多会话资源泄漏"）。
const PENDING_WRITE_MAX_AGE_SECS: u64 = 10 * 60;

/// `sql_test_connection` 里裸调用 `adapter.test_connection` 完全没有超时兜底——
/// 目标主机防火墙静默丢包（不是直接 RST 拒绝）时，TCP 连接会一直卡在等待
/// 重传，前端的"测试连接"按钮会永远停在"测试中…"（2026-09 用户实测反馈）。
/// 参照 `ssh::session::exec`/`coding::webfetch` 等既有超时约定，给测试连接
/// 也套一层 `tokio::time::timeout`，保证前端一定能在有限时间内拿到结果。
const TEST_CONNECTION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

fn statement_allows_readonly(statement: &Statement) -> bool {
    matches!(statement, Statement::Query(_) | Statement::Explain { .. })
}

async fn open_session(
    state: &State<'_, AppState>,
    data_source_id: Uuid,
) -> Result<Arc<dyn AdapterSession>, AppError> {
    state.sql_session_manager.sweep_stale_pending_writes(PENDING_WRITE_MAX_AGE_SECS).await;
    state.sql_session_manager.get_or_open(data_source_id).await
}

fn require_data_source(state: &State<'_, AppState>, id: Uuid) -> Result<DataSourceProfile, AppError> {
    state
        .sql_data_source_service
        .get(id)?
        .ok_or_else(|| AppError::NotFound(format!("data source not found: {id}")))
}

// ---------------------------------------------------------------------------
// 数据源 CRUD
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn sql_list_data_sources(state: State<'_, AppState>) -> Result<Vec<DataSourceProfile>, AppError> {
    state.sql_data_source_service.list()
}

#[tauri::command]
pub async fn sql_save_data_source(
    state: State<'_, AppState>,
    id: Option<Uuid>,
    input: DataSourceInput,
) -> Result<DataSourceProfile, AppError> {
    match id {
        Some(id) => state.sql_data_source_service.update(id, input).await,
        None => state.sql_data_source_service.create(input).await,
    }
}

#[tauri::command]
pub async fn sql_delete_data_source(state: State<'_, AppState>, id: Uuid) -> Result<(), AppError> {
    state.sql_session_manager.close(id).await;
    state.sql_data_source_service.delete(id).await
}

#[tauri::command]
pub async fn sql_test_connection(
    state: State<'_, AppState>,
    id: Option<Uuid>,
    input: DataSourceInput,
) -> Result<DbInfo, AppError> {
    let password = match &input.password {
        Some(p) if !p.is_empty() => Some(p.clone()),
        _ => match id {
            Some(id) => {
                let existing = require_data_source(&state, id)?;
                match existing.credential_ref {
                    Some(key) => state.credential_store.get(&key).await?,
                    None => None,
                }
            }
            None => None,
        },
    };
    let resolved = ResolvedProfile {
        id: id.unwrap_or_else(Uuid::new_v4),
        db_kind: input.db_kind,
        host: input.host.clone(),
        port: input.port.unwrap_or_else(|| input.db_kind.default_port().unwrap_or(0)),
        database_name: input.database_name.clone(),
        default_schema: input.default_schema.clone(),
        username: input.username.clone(),
        password,
        ssl_required: input.ssl_required,
        readonly: input.readonly,
    };
    let adapter = registry::create_adapter(input.db_kind)?;
    match tokio::time::timeout(TEST_CONNECTION_TIMEOUT, adapter.test_connection(&resolved)).await {
        Ok(result) => result,
        Err(_) => Err(AppError::Connection(format!(
            "连接测试超时（{}s），请检查主机地址/端口/网络策略是否正确",
            TEST_CONNECTION_TIMEOUT.as_secs()
        ))),
    }
}

// ---------------------------------------------------------------------------
// 会话 / 对象浏览
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn sql_preview_template(state: State<'_, AppState>, data_source_id: Uuid, object: ObjectRef) -> Result<String, AppError> {
    let profile = require_data_source(&state, data_source_id)?;
    Ok(crate::sql::adapter::preview_sql(profile.db_kind, &object))
}

#[tauri::command]
pub async fn sql_open_session(state: State<'_, AppState>, data_source_id: Uuid) -> Result<(), AppError> {
    open_session(&state, data_source_id).await?;
    Ok(())
}

#[tauri::command]
pub async fn sql_close_session(state: State<'_, AppState>, data_source_id: Uuid) -> Result<(), AppError> {
    state.sql_session_manager.close(data_source_id).await;
    Ok(())
}

#[tauri::command]
pub async fn sql_list_databases(
    state: State<'_, AppState>,
    data_source_id: Uuid,
) -> Result<Vec<String>, AppError> {
    let session = open_session(&state, data_source_id).await?;
    session.list_databases().await
}

#[tauri::command]
pub async fn sql_current_database(
    state: State<'_, AppState>,
    data_source_id: Uuid,
) -> Result<Option<String>, AppError> {
    state.sql_session_manager.current_database_name(data_source_id).await
}

#[tauri::command]
pub async fn sql_switch_database(
    state: State<'_, AppState>,
    data_source_id: Uuid,
    database_name: String,
) -> Result<(), AppError> {
    state.sql_session_manager.switch_database(data_source_id, database_name).await;
    Ok(())
}

#[tauri::command]
pub async fn sql_list_objects(
    state: State<'_, AppState>,
    data_source_id: Uuid,
) -> Result<ObjectPage, AppError> {
    let session = open_session(&state, data_source_id).await?;
    let objects = session.list_objects().await?;
    Ok(ObjectPage { objects })
}

/// 把列信息拼成一份可读的"伪 DDL"摘要——不追求语法上能重新执行，只是给
/// AI 当上下文用（方案 §4.4"schema_cache 只允许存 DDL/结构信息"）。真正的
/// 视图/函数 DDL（adapter 能拿到的话）优先使用。
fn synthesize_ddl(def: &ObjectDefinition) -> String {
    if let Some(ddl) = &def.ddl {
        return ddl.clone();
    }
    let mut out = format!("-- {}.{} 列结构（自动生成的摘要，不是原始 DDL）\n", def.object.schema, def.object.name);
    for col in &def.columns {
        out.push_str(&format!(
            "{}  {}{}{}\n",
            col.name,
            col.data_type,
            if col.nullable { "" } else { " NOT NULL" },
            if col.is_primary_key { " PRIMARY KEY" } else { "" }
        ));
    }
    for idx in &def.indexes {
        out.push_str(&format!("-- INDEX {}: {}\n", idx.name, idx.definition));
    }
    out
}

#[tauri::command]
pub async fn sql_describe_object(
    state: State<'_, AppState>,
    data_source_id: Uuid,
    object: ObjectRef,
) -> Result<ObjectDefinition, AppError> {
    let session = open_session(&state, data_source_id).await?;
    let def = session.describe_object(&object).await?;
    let cache_text = synthesize_ddl(&def);
    let _ = state
        .sql_workspace_cache
        .write_schema_ddl(data_source_id, &object.schema, &object.name, &cache_text);
    Ok(def)
}

// ---------------------------------------------------------------------------
// 表数据浏览 / 编辑（2026-09 用户反馈："右键查看表的 Schema 和数据，在查看表
// 的界面里可以修改表的数据（含表字段）"）——单元格编辑/加删行不经过
// `sql_execute` 的 Confirm/Transaction 确认闸门，理由见
// `sql::data_editor::build_update_cell` 的文档注释；表结构修改
// （`sql_generate_alter_table`）反过来刻意只生成 DDL 文本、不在这里直接执行，
// 交给调用方（前端）塞进一个新标签页走正常的 DDL 确认流程。
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn sql_table_page(
    state: State<'_, AppState>,
    data_source_id: Uuid,
    object: ObjectRef,
    limit: usize,
    offset: usize,
) -> Result<ExecuteResult, AppError> {
    let profile = require_data_source(&state, data_source_id)?;
    let session = open_session(&state, data_source_id).await?;
    let def = session.describe_object(&object).await?;
    let order_col = def.columns.iter().find(|c| c.is_primary_key).map(|c| c.name.clone());
    let sql = data_editor::build_page_query(profile.db_kind, &object, order_col.as_deref(), limit, offset);
    session.execute_sql(&sql, limit, new_backend_handle_slot()).await
}

#[tauri::command]
pub async fn sql_table_row_count(
    state: State<'_, AppState>,
    data_source_id: Uuid,
    object: ObjectRef,
) -> Result<u64, AppError> {
    let profile = require_data_source(&state, data_source_id)?;
    let session = open_session(&state, data_source_id).await?;
    let sql = data_editor::build_count_query(profile.db_kind, &object);
    let result = session.execute_sql(&sql, 1, new_backend_handle_slot()).await?;
    let text = result
        .rows
        .first()
        .and_then(|r| r.first())
        .map(|c| c.text.clone())
        .unwrap_or_default();
    Ok(text.parse().unwrap_or(0))
}

#[tauri::command]
pub async fn sql_table_update_cell(
    state: State<'_, AppState>,
    data_source_id: Uuid,
    object: ObjectRef,
    pk: Vec<NamedCell>,
    column: String,
    value: CellInput,
) -> Result<(), AppError> {
    let profile = require_data_source(&state, data_source_id)?;
    require_writable(&profile)?;
    let session = open_session(&state, data_source_id).await?;
    let sql = data_editor::build_update_cell(profile.db_kind, &object, &pk, &column, &value);
    session.execute_sql(&sql, 0, new_backend_handle_slot()).await?;
    record_history(&state, data_source_id, &sql, "finished", None, Some(1), None);
    Ok(())
}

#[tauri::command]
pub async fn sql_table_delete_row(
    state: State<'_, AppState>,
    data_source_id: Uuid,
    object: ObjectRef,
    pk: Vec<NamedCell>,
) -> Result<(), AppError> {
    let profile = require_data_source(&state, data_source_id)?;
    require_writable(&profile)?;
    let session = open_session(&state, data_source_id).await?;
    let sql = data_editor::build_delete_row(profile.db_kind, &object, &pk);
    session.execute_sql(&sql, 0, new_backend_handle_slot()).await?;
    record_history(&state, data_source_id, &sql, "finished", None, Some(1), None);
    Ok(())
}

#[tauri::command]
pub async fn sql_table_insert_row(
    state: State<'_, AppState>,
    data_source_id: Uuid,
    object: ObjectRef,
    values: Vec<NamedCell>,
) -> Result<(), AppError> {
    let profile = require_data_source(&state, data_source_id)?;
    require_writable(&profile)?;
    let session = open_session(&state, data_source_id).await?;
    let sql = data_editor::build_insert_row(profile.db_kind, &object, &values);
    session.execute_sql(&sql, 0, new_backend_handle_slot()).await?;
    record_history(&state, data_source_id, &sql, "finished", None, Some(1), None);
    Ok(())
}

/// 只生成 DDL 文本，不执行——见本节顶部的说明。
#[tauri::command]
pub fn sql_generate_alter_table(
    state: State<'_, AppState>,
    data_source_id: Uuid,
    object: ObjectRef,
    ops: Vec<AlterOp>,
) -> Result<Vec<String>, AppError> {
    let profile = require_data_source(&state, data_source_id)?;
    Ok(data_editor::build_alter_table(profile.db_kind, &object, &ops))
}

// ---------------------------------------------------------------------------
// 执行 / 取消 / 写操作确认
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn sql_execute(
    state: State<'_, AppState>,
    data_source_id: Uuid,
    sql: String,
    confirmed: bool,
) -> Result<ExecuteOutcome, AppError> {
    let profile = require_data_source(&state, data_source_id)?;
    let (statement, execution_kind) = policy::parse_and_classify(&sql, profile.db_kind)?;

    if profile.readonly && !statement_allows_readonly(&statement) {
        return Err(AppError::PermissionDenied(
            "该数据源已开启只读模式，拒绝执行写操作".into(),
        ));
    }

    let session = open_session(&state, data_source_id).await?;

    match execution_kind {
        ExecutionKind::Transaction => {
            let pending = session.begin_write(&sql).await?;
            record_history(
                &state,
                data_source_id,
                &sql,
                "pending_confirm",
                None,
                pending.rows_affected.map(|n| n as i64),
                None,
            );
            Ok(ExecuteOutcome::PendingWrite(pending))
        }
        ExecutionKind::Confirm if !confirmed => Ok(ExecuteOutcome::NeedsConfirmation),
        ExecutionKind::Normal | ExecutionKind::Confirm => {
            // 完成态（耗时、行数）要等 `sql_poll_query` 拿到结果才知道，这里先
            // 记一条"已发起"的审计行，保证至少"什么时候执行过什么"有迹可查
            // （方案 §7 审计要求）；不追加"完成后补写这条记录"的第二次落库，
            // 避免为一条历史记录多引入一套跨 command 的状态匹配逻辑。
            record_history(&state, data_source_id, &sql, "started", None, None, None);
            let query_id = state.sql_executor.spawn_with_backend_handle(move |slot| {
                let session = session.clone();
                let sql = sql.clone();
                async move { session.execute_sql(&sql, 1000, slot).await }
            });
            Ok(ExecuteOutcome::Started { query_id })
        }
    }
}

fn record_history(
    state: &State<'_, AppState>,
    data_source_id: Uuid,
    sql: &str,
    status: &str,
    duration_ms: Option<i64>,
    row_count: Option<i64>,
    error_message: Option<&str>,
) {
    let _ = state.sql_query_history.record(
        data_source_id,
        sql,
        status,
        duration_ms,
        row_count,
        error_message,
        &Utc::now().to_rfc3339(),
    );
}

#[tauri::command]
pub async fn sql_poll_query(state: State<'_, AppState>, query_id: Uuid) -> Result<QueryPoll, AppError> {
    state.sql_executor.poll(query_id).await
}

#[tauri::command]
pub async fn sql_cancel(
    state: State<'_, AppState>,
    data_source_id: Uuid,
    query_id: Uuid,
) -> Result<(), AppError> {
    // 先中断本地任务，再尝试真正杀掉服务端正在执行的语句——两步都要做，
    // 缺第二步就是 rainfrog 文档里点名的那个坑（docs/SQL_DESKTOP_PLAN.md
    // §4.2.1 第 2 点）。
    let backend_handle = state.sql_executor.abort(query_id);
    if let Some(handle) = backend_handle {
        let session = open_session(&state, data_source_id).await?;
        session.kill_backend(&handle).await?;
    }
    Ok(())
}

#[tauri::command]
pub async fn sql_confirm_write(
    state: State<'_, AppState>,
    data_source_id: Uuid,
    pending_id: Uuid,
) -> Result<ExecuteResult, AppError> {
    let session = open_session(&state, data_source_id).await?;
    session.commit_write(pending_id).await
}

#[tauri::command]
pub async fn sql_rollback_write(
    state: State<'_, AppState>,
    data_source_id: Uuid,
    pending_id: Uuid,
) -> Result<(), AppError> {
    let session = open_session(&state, data_source_id).await?;
    session.rollback_write(pending_id).await
}

/// EXPLAIN——首期只有 MySQL/PostgreSQL/openGauss 支持（都是简单的
/// `EXPLAIN <sql>` 前缀语法）；SQL Server 需要单独的 SET SHOWPLAN 会话开关，
/// 本次没做，明确报错而不是假装支持（方案 §4.3"缺少依赖时有明确提示"的
/// 同一种坦诚态度）。
#[tauri::command]
pub async fn sql_explain(
    state: State<'_, AppState>,
    data_source_id: Uuid,
    sql: String,
) -> Result<ExecuteResult, AppError> {
    let profile = require_data_source(&state, data_source_id)?;
    if matches!(profile.db_kind, DbKind::SqlServer) {
        return Err(AppError::Internal(
            "SQL Server 的执行计划获取暂未实现（需要单独的 SET SHOWPLAN 会话开关）".into(),
        ));
    }
    let session = open_session(&state, data_source_id).await?;
    let explain_sql = format!("EXPLAIN {sql}");
    let slot = crate::sql::adapter::new_backend_handle_slot();
    session.execute_sql(&explain_sql, 1000, slot).await
}

#[tauri::command]
pub fn sql_query_history(
    state: State<'_, AppState>,
    data_source_id: Uuid,
    limit: i64,
) -> Result<Vec<QueryHistoryEntry>, AppError> {
    state.sql_query_history.list(data_source_id, limit)
}

// ---------------------------------------------------------------------------
// 工作区标签页（本地目录缓存，方案 §4.4）
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn sql_workspace_tabs_list(
    state: State<'_, AppState>,
    data_source_id: Uuid,
) -> Result<Vec<WorkspaceTab>, AppError> {
    state.sql_workspace_tabs.list_for_data_source(data_source_id)
}

#[tauri::command]
pub fn sql_workspace_tab_create(
    state: State<'_, AppState>,
    data_source_id: Uuid,
    title: String,
) -> Result<WorkspaceTab, AppError> {
    let tab_id = Uuid::new_v4();
    let file_path = state.sql_workspace_cache.create_tab_file(data_source_id, tab_id)?;
    let now = Utc::now().to_rfc3339();
    let tab = WorkspaceTab {
        id: tab_id,
        data_source_id,
        title,
        file_path,
        result_view_mode: "table".to_string(),
        cursor_json: None,
        sort_order: 0,
        updated_at: now,
    };
    state.sql_workspace_tabs.create(&tab)?;
    Ok(tab)
}

#[tauri::command]
pub fn sql_workspace_tab_delete(
    state: State<'_, AppState>,
    data_source_id: Uuid,
    tab_id: Uuid,
) -> Result<(), AppError> {
    if let Some(tab) = state.sql_workspace_tabs.get(tab_id)? {
        let _ = state.sql_workspace_cache.delete_tab_file(data_source_id, &tab.file_path);
    }
    state.sql_workspace_tabs.delete(tab_id)
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn sql_workspace_tab_update_meta(
    state: State<'_, AppState>,
    tab_id: Uuid,
    title: String,
    result_view_mode: String,
    cursor_json: Option<String>,
    sort_order: i64,
) -> Result<(), AppError> {
    state.sql_workspace_tabs.update_meta(
        tab_id,
        &title,
        &result_view_mode,
        cursor_json.as_deref(),
        sort_order,
        &Utc::now().to_rfc3339(),
    )
}

#[tauri::command]
pub fn sql_tab_read_content(
    state: State<'_, AppState>,
    data_source_id: Uuid,
    tab_id: Uuid,
) -> Result<String, AppError> {
    let tab = state
        .sql_workspace_tabs
        .get(tab_id)?
        .ok_or_else(|| AppError::NotFound(format!("tab not found: {tab_id}")))?;
    state.sql_workspace_cache.read_tab_content(data_source_id, &tab.file_path)
}

/// 人手动编辑走这个直接落盘，不经过 `ChangeStore`——Diff/Accept 流程是给
/// AI 改动准备的确认闸门，用户自己在编辑器里敲字不需要对自己确认一遍
/// （方案 §8："AI 生成 SQL"和"人手改 SQL"在撤销栈里是同一套语义"指的是
/// AI 一侧改动要走确认，不是说人手编辑也要走）。
#[tauri::command]
pub fn sql_tab_write_content(
    state: State<'_, AppState>,
    data_source_id: Uuid,
    tab_id: Uuid,
    content: String,
) -> Result<(), AppError> {
    let tab = state
        .sql_workspace_tabs
        .get(tab_id)?
        .ok_or_else(|| AppError::NotFound(format!("tab not found: {tab_id}")))?;
    state
        .sql_workspace_cache
        .write_tab_content(data_source_id, &tab.file_path, &content)?;
    state.sql_workspace_tabs.update_meta(
        tab_id,
        &tab.title,
        &tab.result_view_mode,
        tab.cursor_json.as_deref(),
        tab.sort_order,
        &Utc::now().to_rfc3339(),
    )
}

// ---------------------------------------------------------------------------
// AI 面板（方案 §8、§4.4）——生成/优化/修复走 ChangeStore 确认闸门，
// 解释直接返回文本。
// ---------------------------------------------------------------------------

async fn get_or_create_sql_change_store(
    state: &State<'_, AppState>,
    data_source_id: Uuid,
) -> Arc<Mutex<ChangeStore>> {
    if let Some(store) = state.sql_changes.read().await.get(&data_source_id) {
        return store.clone();
    }
    let workspace_root = state
        .sql_workspace_cache
        .data_source_root(data_source_id)
        .to_string_lossy()
        .to_string();
    let file_ops: Arc<dyn FileOps> = Arc::new(LocalFileOps);
    let store = Arc::new(Mutex::new(ChangeStore::new(
        data_source_id,
        workspace_root,
        CodingTarget::Local,
        file_ops,
        false,
    )));
    state
        .sql_changes
        .write()
        .await
        .insert(data_source_id, store.clone());
    store
}

async fn stage_ai_result(
    state: &State<'_, AppState>,
    app_handle: &AppHandle,
    data_source_id: Uuid,
    tab_id: Uuid,
    new_sql: String,
) -> Result<FileChange, AppError> {
    let tab = state
        .sql_workspace_tabs
        .get(tab_id)?
        .ok_or_else(|| AppError::NotFound(format!("tab not found: {tab_id}")))?;
    let abs_path = state
        .sql_workspace_cache
        .guard_path(data_source_id, &tab.file_path)?
        .to_string_lossy()
        .to_string();
    let store = get_or_create_sql_change_store(state, data_source_id).await;
    let mut guard = store.lock().await;
    let (change, _sync) = guard
        .stage(
            &abs_path,
            new_sql,
            Uuid::new_v4(),
            &state.ssh_pool,
            &state.agent_pool,
            app_handle,
        )
        .await?;
    Ok(change)
}

#[tauri::command]
pub async fn sql_ai_generate(
    state: State<'_, AppState>,
    app_handle: AppHandle,
    data_source_id: Uuid,
    tab_id: Uuid,
    provider_id: Uuid,
    instruction: String,
    schema_context: String,
    current_sql: String,
) -> Result<FileChange, AppError> {
    let sql = state
        .sql_ai_assistant
        .generate_sql(provider_id, &instruction, &schema_context, &current_sql)
        .await?;
    stage_ai_result(&state, &app_handle, data_source_id, tab_id, sql).await
}

#[tauri::command]
pub async fn sql_ai_explain(
    state: State<'_, AppState>,
    provider_id: Uuid,
    sql: String,
    schema_context: String,
) -> Result<String, AppError> {
    state.sql_ai_assistant.explain_sql(provider_id, &sql, &schema_context).await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn sql_ai_optimize(
    state: State<'_, AppState>,
    app_handle: AppHandle,
    data_source_id: Uuid,
    tab_id: Uuid,
    provider_id: Uuid,
    sql: String,
    explain_output: Option<String>,
    schema_context: String,
) -> Result<FileChange, AppError> {
    let optimized = state
        .sql_ai_assistant
        .optimize_sql(provider_id, &sql, explain_output.as_deref(), &schema_context)
        .await?;
    stage_ai_result(&state, &app_handle, data_source_id, tab_id, optimized).await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn sql_ai_fix_error(
    state: State<'_, AppState>,
    app_handle: AppHandle,
    data_source_id: Uuid,
    tab_id: Uuid,
    provider_id: Uuid,
    sql: String,
    error_message: String,
    schema_context: String,
) -> Result<FileChange, AppError> {
    let fixed = state
        .sql_ai_assistant
        .fix_error(provider_id, &sql, &error_message, &schema_context)
        .await?;
    stage_ai_result(&state, &app_handle, data_source_id, tab_id, fixed).await
}

#[tauri::command]
pub async fn sql_accept_change(
    state: State<'_, AppState>,
    app_handle: AppHandle,
    data_source_id: Uuid,
    change_id: Uuid,
) -> Result<FileSyncInfo, AppError> {
    let store = get_or_create_sql_change_store(&state, data_source_id).await;
    let mut guard = store.lock().await;
    guard
        .accept(change_id, &state.ssh_pool, &state.agent_pool, &app_handle)
        .await
}

#[tauri::command]
pub async fn sql_reject_change(
    state: State<'_, AppState>,
    data_source_id: Uuid,
    change_id: Uuid,
) -> Result<(), AppError> {
    let store = get_or_create_sql_change_store(&state, data_source_id).await;
    let mut guard = store.lock().await;
    guard.reject(change_id)
}

#[tauri::command]
pub async fn sql_undo_change(
    state: State<'_, AppState>,
    data_source_id: Uuid,
    change_id: Uuid,
) -> Result<FileSyncInfo, AppError> {
    let store = get_or_create_sql_change_store(&state, data_source_id).await;
    let mut guard = store.lock().await;
    guard.undo(change_id).await
}

#[tauri::command]
pub async fn sql_revert_turn(
    state: State<'_, AppState>,
    data_source_id: Uuid,
    turn_id: Uuid,
) -> Result<Vec<FileSyncInfo>, AppError> {
    let store = get_or_create_sql_change_store(&state, data_source_id).await;
    let mut guard = store.lock().await;
    guard.revert_turn(turn_id).await
}

// ---------------------------------------------------------------------------
// 导出 / 导入（2026-09 用户反馈："右键表/Schema/数据库可以导出导入数据，
// 导入前提示先备份，导出要考虑大数据量的断点续传"）——具体的分批查询/写盘/
// 断点续传逻辑在 `sql::transfer`，这里只是把 Tauri command 接上去。导入前
// 的"提示备份"是纯前端确认弹窗职责，后端不做自动备份（那是另一套量级的
// 功能，见 `sql::transfer` 模块文档）。
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn sql_export_start(
    state: State<'_, AppState>,
    data_source_id: Uuid,
    object: ObjectRef,
    format: TransferFormat,
    file_path: String,
    resume: bool,
) -> Result<Uuid, AppError> {
    let profile = require_data_source(&state, data_source_id)?;
    let session = open_session(&state, data_source_id).await?;
    let def = session.describe_object(&object).await?;
    let order_col = def.columns.iter().find(|c| c.is_primary_key).map(|c| c.name.clone());
    state
        .sql_transfer_manager
        .start_export(session, profile.db_kind, object, order_col, format, file_path, resume)
        .await
}

#[tauri::command]
pub fn sql_export_poll(state: State<'_, AppState>, export_id: Uuid) -> Result<TransferProgress, AppError> {
    state.sql_transfer_manager.poll(export_id)
}

#[tauri::command]
pub fn sql_export_cancel(state: State<'_, AppState>, export_id: Uuid) -> Result<(), AppError> {
    state.sql_transfer_manager.cancel(export_id);
    Ok(())
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn sql_import_start(
    state: State<'_, AppState>,
    data_source_id: Uuid,
    object: ObjectRef,
    file_path: String,
    has_header: bool,
    resume: bool,
) -> Result<Uuid, AppError> {
    let profile = require_data_source(&state, data_source_id)?;
    require_writable(&profile)?;
    let session = open_session(&state, data_source_id).await?;
    // 有表头就用 CSV 第一行的列名（要求和表里的列名完全一致）；没有表头就
    // 只能假定 CSV 列顺序和 `describe_object` 返回的表列顺序一一对应——
    // 这是没有表头时唯一能确定映射关系的办法，前端应该在勾掉"含表头"选项
    // 时明确提示这个假定。
    let columns = if has_header {
        read_csv_header(&file_path)?
    } else {
        let def = session.describe_object(&object).await?;
        def.columns.into_iter().map(|c| c.name).collect()
    };
    state
        .sql_transfer_manager
        .start_import(session, profile.db_kind, object, columns, file_path, has_header, resume)
        .await
}

fn read_csv_header(file_path: &str) -> Result<Vec<String>, AppError> {
    use std::io::BufRead;
    let file = std::fs::File::open(file_path)?;
    let mut reader = std::io::BufReader::new(file);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    Ok(line.trim_end_matches(['\r', '\n']).split(',').map(|s| s.trim().to_string()).collect())
}

#[tauri::command]
pub fn sql_import_poll(state: State<'_, AppState>, import_id: Uuid) -> Result<TransferProgress, AppError> {
    state.sql_transfer_manager.poll(import_id)
}

#[tauri::command]
pub fn sql_import_cancel(state: State<'_, AppState>, import_id: Uuid) -> Result<(), AppError> {
    state.sql_transfer_manager.cancel(import_id);
    Ok(())
}

/// 导出"当前结果网格"（用户已经跑出来、就在界面上的这一批行），跟
/// `sql_export_start` 那个面向整张表、支持断点续传的大数据导出是两回事——
/// 这里数据已经在前端内存里了，序列化成 CSV/JSON 文本后直接落盘即可，不用
/// 走后端分批查询。落盘路径来自用户在原生保存对话框里选的位置，不做工作区
/// 越界检查（不属于任何工作区目录语义）。
#[tauri::command]
pub async fn sql_write_text_file(path: String, content: String) -> Result<(), AppError> {
    tokio::fs::write(&path, content).await?;
    Ok(())
}
