use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, State};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::coding::permission::{Decision, PermissionRule};
use crate::coding::tools::TodoItem;
use crate::coding::{CodingMode, CodingSession, CodingTarget, FileChange};
use crate::error::AppError;
use crate::mcp::{McpServer, McpServerInput};
use crate::state::AppState;
use crate::workspace::WorkspaceKind;
use crate::db::repo::coding_history_repo::{CodingHistoryDetail, CodingHistoryInput, CodingHistorySummary, WorkspaceHistorySnapshot};

#[derive(Serialize)]
pub struct CodingSessionInfo {
    pub id: Uuid,
    pub provider_id: Uuid,
    pub mode: CodingMode,
    pub target: CodingTarget,
    pub auto_allow_readonly: bool,
    pub git_repo: bool,
    pub auto_git_commit: bool,
    pub changes: Vec<FileChange>,
    pub todos: Vec<TodoItem>,
    /// 本次会话实际读到的项目记忆文件名（`AGENTS.md`/`CLAUDE.md`），前端据此
    /// 渲染"已加载 XXX"徽标；空数组表示两个文件工作区根目录都没有。
    pub project_memory_loaded: Vec<String>,
}

async fn session_info(session: &CodingSession) -> CodingSessionInfo {
    CodingSessionInfo {
        id: session.id,
        provider_id: session.provider_id,
        mode: session.mode,
        target: session.target.clone(),
        auto_allow_readonly: session.auto_allow_readonly,
        git_repo: session.git_repo,
        auto_git_commit: session.auto_git_commit,
        changes: session.changes.clone(),
        todos: session.todos.clone(),
        project_memory_loaded: session.project_memory_loaded.clone(),
    }
}

#[tauri::command]
pub async fn coding_set_provider(
    state: State<'_, AppState>,
    workspace_id: Uuid,
    provider_id: Uuid,
) -> Result<(), AppError> {
    if state.ai_provider_manager.get(provider_id)?.is_none() {
        return Err(AppError::NotFound(format!("ai provider not found: {provider_id}")));
    }
    let session = get_session(&state, workspace_id).await?;
    session.lock().await.provider_id = provider_id;
    Ok(())
}

/// 自动绑定当前工作区打开（或复用已有的）编程助手会话（DESIGN.md §3.8.1）。
#[tauri::command]
pub async fn coding_start(
    state: State<'_, AppState>,
    workspace_id: Uuid,
    provider_id: Uuid,
) -> Result<CodingSessionInfo, AppError> {
    if state.ai_provider_manager.get(provider_id)?.is_none() {
        return Err(AppError::NotFound(format!("ai provider not found: {provider_id}")));
    }

    let workspaces = state.workspaces.read().await;
    let handle = workspaces
        .get(&workspace_id)
        .ok_or_else(|| AppError::NotFound(format!("workspace not opened: {workspace_id}")))?;
    let profile = handle.profile.clone();
    let file_ops = handle.file_ops.clone();
    drop(workspaces);

    if let Some(existing) = state.coding_sessions.read().await.get(&workspace_id).cloned() {
        let guard = existing.lock().await;
        let target_matches = match (&guard.target, profile.kind, profile.connection_id) {
            (CodingTarget::Local, WorkspaceKind::Local, None) => true,
            (CodingTarget::Remote { connection_id, .. }, WorkspaceKind::Remote, Some(expected)) => *connection_id == expected,
            _ => false,
        };
        if target_matches && guard.workspace_root == profile.root_path {
            if state.ai_provider_manager.get(guard.provider_id)?.is_some() {
                return Ok(session_info(&guard).await);
            }
            drop(guard);
            let mut guard = existing.lock().await;
            guard.provider_id = provider_id;
            return Ok(session_info(&guard).await);
        }
        drop(guard);
        state.coding_sessions.write().await.remove(&workspace_id);
    }

    let target = match profile.kind {
        WorkspaceKind::Local => CodingTarget::Local,
        WorkspaceKind::Remote => CodingTarget::Remote {
            connection_id: profile
                .connection_id
                .ok_or_else(|| AppError::Internal("remote workspace missing connection_id".into()))?,
            host_label: profile.display_name.clone(),
        },
    };

    let mut session = CodingSession::new(workspace_id, profile.root_path.clone(), target, provider_id, file_ops);
    // 只在会话开始时探测一次是否在 Git 仓库里——探测本身要跑一条命令，不值得每次
    // 查询会话状态都重新问一遍；工作区在会话生命周期内也不会突然从"不是仓库"
    // 变成"是仓库"（真发生了，用户重开一次工作区/编程会话就能重新探测到）。
    session.git_repo = crate::coding::git_ops::is_git_repo(&session.target, &session.workspace_root, &state.ssh_pool).await;
    // 项目记忆（AGENTS.md/CLAUDE.md）和技能发现同样只在会话开始时做一次，
    // 和上面的 git_repo 探测是同一个时机/理由。
    session.load_project_memory().await;
    session.load_skills().await;
    let info = session_info(&session).await;
    state
        .coding_sessions
        .write()
        .await
        .insert(workspace_id, Arc::new(Mutex::new(session)));
    Ok(info)
}

#[tauri::command]
pub async fn coding_new_session(
    state: State<'_, AppState>,
    workspace_id: Uuid,
    provider_id: Uuid,
) -> Result<CodingSessionInfo, AppError> {
    state.coding_sessions.write().await.remove(&workspace_id);
    coding_start(state, workspace_id, provider_id).await
}

/// 释放一个工作区的常驻编程助手会话（前端有界保活策略的 LRU 淘汰、或工作区
/// 彻底关闭时调用）。会话内存直接丢弃——对话内容早已在每次
/// `sendMessage`/`acceptChange` 等操作后通过 `saveCurrentHistory` 落库，
/// 不需要在这里补一次"关闭前保存"。找不到对应会话（从未打开过，或已经被
/// 释放过）不算错误，幂等处理。
#[tauri::command]
pub async fn coding_close(state: State<'_, AppState>, workspace_id: Uuid) -> Result<(), AppError> {
    state.coding_sessions.write().await.remove(&workspace_id);
    Ok(())
}

#[tauri::command]
pub async fn coding_set_mode(state: State<'_, AppState>, workspace_id: Uuid, mode: CodingMode) -> Result<(), AppError> {
    let session = get_session(&state, workspace_id).await?;
    session.lock().await.mode = mode;
    Ok(())
}

#[tauri::command]
pub async fn coding_set_auto_allow_readonly(
    state: State<'_, AppState>,
    workspace_id: Uuid,
    enabled: bool,
) -> Result<(), AppError> {
    let session = get_session(&state, workspace_id).await?;
    session.lock().await.auto_allow_readonly = enabled;
    Ok(())
}

/// 开关"每次 Accept 一条变更就自动 git add + commit"（DESIGN.md §3.8.2）；
/// `session.git_repo` 为 false（工作区根目录不是 Git 仓库）时前端应该把这个开关
/// disable 掉，这里不重复做校验——就算开了也不会有任何效果（`accept_change` 里
/// `auto_git_commit && git_repo` 才会真的去跑 git 命令）。
#[tauri::command]
pub async fn coding_set_auto_git_commit(
    state: State<'_, AppState>,
    workspace_id: Uuid,
    enabled: bool,
) -> Result<(), AppError> {
    let session = get_session(&state, workspace_id).await?;
    session.lock().await.auto_git_commit = enabled;
    Ok(())
}

#[tauri::command]
pub async fn coding_send_message(
    state: State<'_, AppState>,
    app_handle: AppHandle,
    workspace_id: Uuid,
    text: String,
    attachments: Option<Vec<crate::coding::ChatAttachment>>,
) -> Result<String, AppError> {
    let session = get_session(&state, workspace_id).await?;
    let mut session = session.lock().await;
    session
        .send_message(
            &text,
            &attachments.unwrap_or_default(),
            &state.ai_provider_manager,
            &state.ssh_pool,
            &state.audit_log,
            &state.command_confirms,
            &state.permission_rules,
            &state.question_confirms,
            &state.mcp_manager,
            &app_handle,
        )
        .await
}

/// "输入优化"按钮（DESIGN.md §3.8 附件/优化输入需求）：用当前会话绑定的
/// Provider 把用户还没发出去的草稿改写一遍，只返回改写结果，不进入对话历史
/// （复用 `AiChatClient::complete_once`，见其文档）。
const OPTIMIZE_PROMPT_SYSTEM: &str =
    "你是一个提示词优化助手，任务是把用户写给 AI 编程助手的草稿指令改写得更清晰、具体、可执行。\
     要求：1) 保留用户的原始意图，不要编造用户没提到的具体文件名/路径/技术选型等事实性细节；\
     2) 把模糊的描述具体化，必要时补充\"预期效果\"\"验收标准\"这类结构，让编程助手能一次理解到位；\
     3) 只输出改写后的指令本身，不要输出任何解释、前后缀说明或引号。";

#[tauri::command]
pub async fn coding_optimize_prompt(state: State<'_, AppState>, workspace_id: Uuid, text: String) -> Result<String, AppError> {
    let session = get_session(&state, workspace_id).await?;
    let provider_id = session.lock().await.provider_id;
    let provider = state
        .ai_provider_manager
        .get(provider_id)?
        .ok_or_else(|| AppError::NotFound(format!("ai provider not found: {provider_id}")))?;
    let api_key = state.ai_provider_manager.resolve_api_key(&provider).await?;
    crate::ai::chat::AiChatClient::new()
        .complete_once(&provider, api_key.as_deref(), OPTIMIZE_PROMPT_SYSTEM, &text)
        .await
}

/// 响应 `coding:question-request`（`question` 工具的结构化提问，见
/// `coding/session.rs::ask_user`）——和 `coding_confirm_command` 是同一个
/// oneshot 应答模式，只是这里传回一段文本而不是 bool。
#[tauri::command]
pub async fn coding_answer_question(state: State<'_, AppState>, request_id: Uuid, answer: String) -> Result<(), AppError> {
    state.question_confirms.resolve(request_id, answer).await;
    Ok(())
}

// ---- 权限规则（REQUIREMENTS.md §3.7 权限引擎升级）----

#[tauri::command]
pub async fn permission_rule_list(state: State<'_, AppState>) -> Result<Vec<PermissionRule>, AppError> {
    state.permission_rules.list()
}

#[derive(serde::Deserialize)]
pub struct PermissionRuleInput {
    pub tool: String,
    pub pattern: String,
    pub decision: String,
}

#[tauri::command]
pub async fn permission_rule_create(state: State<'_, AppState>, input: PermissionRuleInput) -> Result<PermissionRule, AppError> {
    let rule = PermissionRule {
        id: Uuid::new_v4(),
        tool: input.tool,
        pattern: input.pattern,
        decision: Decision::from_str(&input.decision),
        enabled: true,
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    state.permission_rules.create(&rule)?;
    Ok(rule)
}

#[tauri::command]
pub async fn permission_rule_delete(state: State<'_, AppState>, id: Uuid) -> Result<(), AppError> {
    state.permission_rules.delete(id)
}

// ---- MCP 服务器管理（REQUIREMENTS.md §3.7"未实现：MCP 客户端"补上的部分）----

#[tauri::command]
pub async fn mcp_server_list(state: State<'_, AppState>) -> Result<Vec<McpServer>, AppError> {
    state.mcp_manager.list()
}

#[tauri::command]
pub async fn mcp_server_create(state: State<'_, AppState>, input: McpServerInput) -> Result<McpServer, AppError> {
    state.mcp_manager.create(input).await
}

#[tauri::command]
pub async fn mcp_server_update(state: State<'_, AppState>, id: Uuid, input: McpServerInput) -> Result<McpServer, AppError> {
    state.mcp_manager.update(id, input).await
}

#[tauri::command]
pub async fn mcp_server_delete(state: State<'_, AppState>, id: Uuid) -> Result<(), AppError> {
    state.mcp_manager.delete(id).await
}

#[tauri::command]
pub async fn coding_accept_change(
    state: State<'_, AppState>,
    app_handle: AppHandle,
    workspace_id: Uuid,
    change_id: Uuid,
) -> Result<(), AppError> {
    let session = get_session(&state, workspace_id).await?;
    let mut guard = session.lock().await;
    guard.accept_change(change_id, &state.ssh_pool, &app_handle).await
}

#[tauri::command]
pub async fn coding_reject_change(state: State<'_, AppState>, workspace_id: Uuid, change_id: Uuid) -> Result<(), AppError> {
    let session = get_session(&state, workspace_id).await?;
    let mut guard = session.lock().await;
    guard.reject_change(change_id)
}

#[tauri::command]
pub async fn coding_undo_change(state: State<'_, AppState>, workspace_id: Uuid, change_id: Uuid) -> Result<(), AppError> {
    let session = get_session(&state, workspace_id).await?;
    let mut guard = session.lock().await;
    guard.undo_change(change_id).await
}

#[tauri::command]
pub async fn coding_redo_change(state: State<'_, AppState>, workspace_id: Uuid) -> Result<Option<Uuid>, AppError> {
    let session = get_session(&state, workspace_id).await?;
    let mut guard = session.lock().await;
    guard.redo_change().await
}

/// 响应 `coding:command-confirm-request`（DESIGN.md §3.8.2.1）。
#[tauri::command]
pub async fn coding_confirm_command(state: State<'_, AppState>, request_id: Uuid, allow: bool) -> Result<(), AppError> {
    state.command_confirms.resolve(request_id, allow).await;
    Ok(())
}

async fn get_session(state: &State<'_, AppState>, workspace_id: Uuid) -> Result<Arc<Mutex<CodingSession>>, AppError> {
    state
        .coding_sessions
        .read()
        .await
        .get(&workspace_id)
        .cloned()
        .ok_or_else(|| AppError::NotFound(format!("no coding session for workspace {workspace_id}, call coding_start first")))
}

#[tauri::command]
pub async fn coding_history_save(state: State<'_, AppState>, input: CodingHistoryInput) -> Result<(), AppError> {
    state.coding_history.save(&input)?;
    if let Some(detail) = state.coding_history.get(input.id)? {
        let snapshot = WorkspaceHistorySnapshot { input: input.clone(), created_at: detail.summary.created_at, updated_at: detail.summary.updated_at };
        if let Some(handle) = state.workspaces.read().await.get(&input.workspace_id).cloned() {
            let dir = format!("{}/.rock_desk/sessions", handle.profile.root_path.trim_end_matches(['/', '\\']));
            let path = format!("{dir}/{}.json", input.id);
            let _ = handle.file_ops.create_dir(&format!("{}/.rock_desk", handle.profile.root_path.trim_end_matches(['/', '\\']))).await;
            let _ = handle.file_ops.create_dir(&dir).await;
            if let Ok(json) = serde_json::to_string_pretty(&snapshot) {
                if let Err(error) = handle.file_ops.write_file(&path, &json, None).await {
                    tracing::warn!(%error, %path, "failed to mirror coding history into workspace cache");
                    let fallback = handle.fallback_cache_dir.join("sessions");
                    if std::fs::create_dir_all(&fallback).is_ok() {
                        let _ = std::fs::write(fallback.join(format!("{}.json", input.id)), json);
                    }
                }
            }
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn coding_history_list(state: State<'_, AppState>, workspace_id: Uuid) -> Result<Vec<CodingHistorySummary>, AppError> {
    if let Some(handle) = state.workspaces.read().await.get(&workspace_id).cloned() {
        let dir = format!("{}/.rock_desk/sessions", handle.profile.root_path.trim_end_matches(['/', '\\']));
        if let Ok(entries) = handle.file_ops.list_dir(&dir).await {
            for entry in entries.into_iter().filter(|entry| !entry.is_dir && entry.name.ends_with(".json")) {
                if let Ok(file) = handle.file_ops.read_file(&entry.path).await {
                    if let Ok(snapshot) = serde_json::from_str::<WorkspaceHistorySnapshot>(&file.text) {
                        if snapshot.input.workspace_id == workspace_id {
                            let _ = state.coding_history.import_snapshot(&snapshot);
                        }
                    }
                }
            }
        }
        let fallback = handle.fallback_cache_dir.join("sessions");
        if let Ok(entries) = std::fs::read_dir(fallback) {
            for entry in entries.flatten() {
                if let Ok(bytes) = std::fs::read(entry.path()) {
                    if let Ok(snapshot) = serde_json::from_slice::<WorkspaceHistorySnapshot>(&bytes) {
                        if snapshot.input.workspace_id == workspace_id {
                            let _ = state.coding_history.import_snapshot(&snapshot);
                        }
                    }
                }
            }
        }
    }
    state.coding_history.list(workspace_id)
}

#[tauri::command]
pub async fn coding_history_get(state: State<'_, AppState>, id: Uuid) -> Result<Option<CodingHistoryDetail>, AppError> {
    state.coding_history.get(id)
}

#[tauri::command]
pub async fn coding_history_rename(state: State<'_, AppState>, id: Uuid, title: String) -> Result<(), AppError> {
    state.coding_history.rename(id, title.trim())
}

#[tauri::command]
pub async fn coding_history_delete(state: State<'_, AppState>, id: Uuid) -> Result<(), AppError> {
    state.coding_history.delete(id)
}
