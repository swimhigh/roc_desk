use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::Instant;

use serde::Serialize;
use tauri::{AppHandle, State};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::coding::permission::{Decision, PermissionRule};
use crate::coding::skills::SkillMeta;
use crate::coding::tools::TodoItem;
use crate::coding::{
    ChangeStore, CodingMode, CodingSession, CodingTarget, FileChange, FileSyncInfo,
};
use crate::db::repo::coding_history_repo::{
    CodingHistoryDetail, CodingHistoryInput, CodingHistoryRepo, CodingHistorySummary,
    WorkspaceHistorySnapshot,
};
use crate::error::AppError;
use crate::fsops::FileOps;
use crate::mcp::{McpServer, McpServerInput};
use crate::state::AppState;
use crate::workspace::WorkspaceKind;

/// 所有"打开工作区/切换历史"路径上、对远程工作区做的探测性/同步性 SFTP·Agent
/// 往返共用的超时上限——这些操作本身都是读写几个小文件、列一个小目录，网络
/// 正常应该秒级完成，真顶到这个上限说明连接已经不可用，没必要让用户等到
/// `ssh/session.rs::EXEC_TIMEOUT` 那个给"跑命令/传大文件"用的分钟级上限。
/// 2026-09 用户反馈"打开 AI 工具/新建会话都要等很久"，根因是这类调用点散落在
/// `build_new_session` 之外（`history_list_with_import`/`coding_history_save`
/// 的工作区镜像写入等），之前只给 `build_new_session` 内部三个探测加了超时，
/// 漏了这几处——这里统一收成一个常量，新增调用点直接复用，不要再各写各的。
const WORKSPACE_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// 一次"列历史"最多**同步等**多少份工作区快照（`.rock_desk/sessions/{id}.json`）
/// 落库；按 mtime 倒序取，也就是优先保证最近的会话是最新的。超出的部分交给
/// `spawn_snapshot_backfill` 在后台补，补完通过 `coding:history-list` 事件把新
/// 列表推给前端——历史攒到几十条、每条快照又带着完整对话时间线和文件改动（动辄
/// 几兆）时，"一次全量拉回来"就是十几秒，这是用户能直接感知到的卡顿；而"列表先
/// 出来、老记录随后补齐"在体验上和一次性拉完没有区别。
const INLINE_SNAPSHOT_SYNC_LIMIT: usize = 8;
/// 会话启动探测结果的缓存有效期，见 `probe_workspace`。
const PROBE_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(60);

/// 一份工作区历史快照文件的待同步信息（`history_list_with_import` 内部用）。
#[derive(Clone)]
struct SnapshotCandidate {
    path: String,
    /// 远端 stat 出来的 mtime（unix 秒）。
    mtime: Option<i64>,
}

/// 会话启动探测（项目记忆 + 技能清单）的缓存条目。
struct ProbeCache {
    /// 一起缓存工作区根路径：同一个 workspace id 被重新指到别的目录时缓存失效。
    root_path: String,
    at: Instant,
    memory: Vec<(String, String)>,
    skills: Vec<SkillMeta>,
}

/// 编程助手在"打开 AI 工具/新建会话/切历史会话"这些路径上用到的进程内缓存。
/// 全都只是缓存——丢了最多多花一次网络往返，不影响正确性，所以不占 `AppState`
/// 的字段，就近放在这里。
#[derive(Default)]
struct CodingCaches {
    /// `(workspace_id, history_id)` -> 这份快照在远端最后一次被本进程读到/写出
    /// 时的 mtime。`history_list_with_import` 靠它判断"这个文件我已经对过账了"，
    /// 不用再整份读回来。
    ///
    /// 为什么不能只靠"远端 mtime vs 本地 SQLite 的 updated_at"判断（原来的写法）：
    /// 这两个时间戳来自两台机器的时钟，远端主机的时钟只要快几秒（很常见），每次
    /// 列历史都会判定"远端更新了"，把目录下**全部**快照重新整份拉一遍——这才是
    /// "历史会话多的时候打开 AI 工具/新建会话要等十几秒"的真正根因（不是超时，
    /// 是真的在传几十兆）。而本进程自己写出去的那份（`coding_history_save` 的
    /// 镜像）内容就等于本地 SQLite 里的那份，更没有任何理由再读回来。
    synced_snapshots: HashMap<(Uuid, Uuid), i64>,
    /// 会话启动探测结果，key 为 workspace id，见 `probe_workspace`。
    probes: HashMap<Uuid, ProbeCache>,
}

fn caches() -> &'static StdMutex<CodingCaches> {
    static CACHES: OnceLock<StdMutex<CodingCaches>> = OnceLock::new();
    CACHES.get_or_init(|| StdMutex::new(CodingCaches::default()))
}

fn mark_snapshot_synced(workspace_id: Uuid, history_id: Uuid, mtime: i64) {
    if let Ok(mut guard) = caches().lock() {
        guard
            .synced_snapshots
            .insert((workspace_id, history_id), mtime);
    }
}

fn snapshot_already_synced(workspace_id: Uuid, history_id: Uuid, mtime: i64) -> bool {
    caches()
        .lock()
        .map(|guard| guard.synced_snapshots.get(&(workspace_id, history_id)) == Some(&mtime))
        .unwrap_or(false)
}

async fn import_workspace_snapshot(
    file_ops: &dyn FileOps,
    history_repo: &CodingHistoryRepo,
    workspace_id: Uuid,
    candidate: &SnapshotCandidate,
) {
    let Ok(Ok(file)) =
        tokio::time::timeout(WORKSPACE_PROBE_TIMEOUT, file_ops.read_file(&candidate.path)).await
    else {
        return;
    };
    let Ok(snapshot) = serde_json::from_str::<WorkspaceHistorySnapshot>(&file.text) else {
        return;
    };
    if snapshot.input.workspace_id != workspace_id {
        return;
    }
    let history_id = snapshot.input.id;
    if history_repo.import_snapshot(&snapshot).is_ok() {
        // `FileContent.mtime` 是实际读到的文件版本，优先用它；部分 FileOps 未提供
        // 时回退 list_dir 的 mtime。二者都没有则下次再读一次，保证不会错过更新。
        if file.mtime > 0 {
            mark_snapshot_synced(workspace_id, history_id, file.mtime);
        } else if let Some(mtime) = candidate.mtime {
            mark_snapshot_synced(workspace_id, history_id, mtime);
        }
    }
}

/// 会话开始时的远程探测——项目记忆（AGENTS.md/CLAUDE.md）、技能发现。两个探测
/// **并发跑，不要顺序 await**：SSH `exec`/SFTP 握手的超时上限都是分钟级（见
/// `ssh/session.rs` 的 `EXEC_TIMEOUT`），顺序跑最坏情况下（远端主机响应慢/不
/// 稳定）要等到叠加——这正是"开始/新建/切历史会话都要等很久"这类真实反馈的根因
/// 之一。另外各自套一层更短的超时（`WORKSPACE_PROBE_TIMEOUT`）：这两个探测本身
/// 都是"读两个小文件/列一个小目录"，网络正常应该秒级完成，真顶到分钟级说明这条
/// 连接已经不可用了，没必要让用户等到那个上限；它们本来就是"尽力而为"的提示词
/// 增强，超时了就当探测失败，不阻塞会话真正可用。
///
/// 结果按工作区缓存 `PROBE_CACHE_TTL`：用户连着点"新会话"、或者在历史会话之间
/// 来回切时，每次 `build_new_session` 都重新探一遍纯属浪费——这些内容在这么短的
/// 时间内不会变（真改了 `AGENTS.md`/技能，等一分钟或重开工作区即可生效）。
async fn probe_workspace(
    workspace_id: Uuid,
    root_path: &str,
    file_ops: &dyn FileOps,
) -> (Vec<(String, String)>, Vec<SkillMeta>) {
    let cached = caches()
        .lock()
        .ok()
        .and_then(|guard| match guard.probes.get(&workspace_id) {
            Some(probe) if probe.root_path == root_path && probe.at.elapsed() < PROBE_CACHE_TTL => {
                Some((probe.memory.clone(), probe.skills.clone()))
            }
            _ => None,
        });
    if let Some(cached) = cached {
        return cached;
    }

    let (memory, skills) = tokio::join!(
        async {
            tokio::time::timeout(
                WORKSPACE_PROBE_TIMEOUT,
                crate::coding::session::fetch_project_memory(file_ops),
            )
            .await
            .unwrap_or_default()
        },
        async {
            tokio::time::timeout(
                WORKSPACE_PROBE_TIMEOUT,
                crate::coding::skills::discover_skills(file_ops, root_path),
            )
            .await
            .unwrap_or_default()
        },
    );

    if let Ok(mut guard) = caches().lock() {
        guard.probes.insert(
            workspace_id,
            ProbeCache {
                root_path: root_path.to_string(),
                at: Instant::now(),
                memory: memory.clone(),
                skills: skills.clone(),
            },
        );
    }
    (memory, skills)
}

#[derive(Serialize)]
pub struct CodingSessionInfo {
    pub id: Uuid,
    pub provider_id: Uuid,
    pub mode: CodingMode,
    pub target: CodingTarget,
    pub auto_allow_readonly: bool,
    pub git_repo: bool,
    pub auto_git_commit: bool,
    /// "完全授权模式"开关的当前状态（见 `ChangeStore::full_auto` 的文档）。
    pub full_auto: bool,
    pub changes: Vec<FileChange>,
    pub todos: Vec<TodoItem>,
    /// 本次会话实际读到的项目记忆文件名（`AGENTS.md`/`CLAUDE.md`），前端据此
    /// 渲染"已加载 XXX"徽标；空数组表示两个文件工作区根目录都没有。
    pub project_memory_loaded: Vec<String>,
}

async fn session_info(session: &CodingSession) -> CodingSessionInfo {
    let store = session.change_store.lock().await;
    CodingSessionInfo {
        id: session.id,
        provider_id: session.provider_id,
        mode: session.mode,
        target: session.target.clone(),
        auto_allow_readonly: session.auto_allow_readonly,
        git_repo: store.git_repo(),
        auto_git_commit: store.auto_git_commit,
        full_auto: store.full_auto.load(std::sync::atomic::Ordering::Relaxed),
        changes: store.changes().to_vec(),
        todos: session.todos.clone(),
        project_memory_loaded: session.project_memory_loaded.clone(),
    }
}

#[tauri::command]
pub async fn coding_set_provider(
    state: State<'_, AppState>,
    app_handle: AppHandle,
    workspace_id: Uuid,
    provider_id: Uuid,
) -> Result<(), AppError> {
    let Some(provider) = state.ai_provider_manager.get(provider_id)? else {
        return Err(AppError::NotFound(format!(
            "ai provider not found: {provider_id}"
        )));
    };
    let session = get_session(&state, workspace_id).await?;
    let change_store = get_change_store(&state, workspace_id).await?;
    let mut guard = session.lock().await;
    guard.provider_id = provider_id;
    // 切换 provider 时，会话之前若已经挂了 `CodexCoreEngine`，那个引擎是绑定着
    // 切换前那个 provider 的 api_key/model/base_url 构造出来的——`send_message`
    // 的 codex 分支只看 `self.codex_engine.is_some()`，不会重新读 `provider_id`
    // 选引擎，所以不重新挂的话请求会继续悄悄发到旧 provider（2026-09 用户反馈：
    // 切换 provider 后同一会话"感觉不正常"，新建会话换新 provider 却是正常的——
    // 根因就是这里，新会话走的是 `build_new_session` 里的挂载逻辑，是新构造的）。
    guard.detach_codex_engine();
    if crate::coding::routes_to_codex_engine(&provider) {
        if let Ok(Some(api_key)) = state.ai_provider_manager.resolve_api_key(&provider).await {
            if let Err(e) = attach_codex_engine(
                &mut guard,
                &provider,
                api_key,
                &change_store,
                &state,
                &app_handle,
            )
            .await
            {
                tracing::warn!(
                    "workspace {workspace_id}: 切换 provider 后 codex engine 初始化失败，回退到自研引擎: {e}"
                );
            }
        }
    }
    Ok(())
}

/// 构造一个全新的（不复用任何已有内存会话的）`CodingSession`。`resume_recent`
/// 控制要不要把最近一次持久化的会话变更记录（`coding_history`，见
/// `history_list_with_import`）灌回 `session.changes`——`coding_start`（自动绑定
/// 工作区，含"应用重启/切工作区后回来"这种场景）要传 `true`：前端
/// `restoreOrStart` 本来就会在 12 小时内把上一次的 FileChange 卡片重新显示出来
/// （见 codingStore.ts），但如果后端这边的 `changes` 一直是空的，用户点这些
/// 卡片上的"撤销"就会报"change not found"——这是改动前就存在的一个真实 bug，
/// 不是这次新引入的行为。`coding_new_session`（用户显式点"新建会话"）要传
/// `false`：用户此时的意图就是清空重来，不该把旧变更悄悄带进新会话。
async fn build_new_session(
    state: &State<'_, AppState>,
    workspace_id: Uuid,
    provider_id: Uuid,
    resume_recent: bool,
    override_id: Option<Uuid>,
    app_handle: &AppHandle,
) -> Result<(CodingSession, Arc<Mutex<ChangeStore>>), AppError> {
    let workspaces = state.workspaces.read().await;
    let handle = workspaces
        .get(&workspace_id)
        .ok_or_else(|| AppError::NotFound(format!("workspace not opened: {workspace_id}")))?;
    let profile = handle.profile.clone();
    let file_ops = handle.file_ops.clone();
    drop(workspaces);

    let target = match profile.kind {
        WorkspaceKind::Local => CodingTarget::Local,
        WorkspaceKind::Remote => {
            let connection_id = profile.connection_id.ok_or_else(|| {
                AppError::Internal("remote workspace missing connection_id".into())
            })?;
            // "Remote" 工作区可能是 SSH 也可能是 Agent 连接——两者共用同一个
            // `WorkspaceKind`，靠连接档案的 `protocol` 字段区分该走哪套 `CodingTarget`。
            let connection = state
                .connection_manager
                .get(connection_id)?
                .ok_or_else(|| {
                    AppError::NotFound(format!("connection not found: {connection_id}"))
                })?;
            match connection.protocol {
                crate::connection::Protocol::Agent => CodingTarget::Agent {
                    connection_id,
                    host_label: profile.display_name.clone(),
                },
                _ => CodingTarget::Remote {
                    connection_id,
                    host_label: profile.display_name.clone(),
                },
            }
        }
    };

    // 会话开始时的远程探测——项目记忆（AGENTS.md/CLAUDE.md）、技能发现——各自
    // 最多只做一次（都不是会在会话生命周期内变化的东西，真变了就等用户重开
    // 一次工作区/编程会话）。**并发跑，不要顺序 await**：SSH `exec`/SFTP 握手
    // 的超时上限都是分钟级（见 `ssh/session.rs` 的 `EXEC_TIMEOUT`），顺序跑
    // 最坏情况下（比如这次连的远程主机本身响应慢/不稳定）要等到叠加——这正是
    // "开始/新建/切历史会话都要等很久"这类真实反馈的根因之一。
    //
    // 注意：**不在这里探测 Git 仓库**——`git_repo` 只是"自动 Git 提交"这个可选
    // 功能的开关依据，2026-09 用户明确要求"工作区不依赖 git，把这个探测去掉"。
    // 现在默认 `git_repo = false`（`CodingSession::new` 里的默认值），只有用户
    // 真的去点"自动提交"开关时（`coding_set_auto_git_commit`）才按需探测一次
    // ——不用 git 的工作区完全不产生这个探测的网络往返/超时风险。
    //
    // 另外套一层更短的超时（`WORKSPACE_PROBE_TIMEOUT`，独立于 `EXEC_TIMEOUT`
    // 那个分钟级的上限）——这两个探测本身都是"读两个小文件/列一个小目录"，
    // 网络正常的话应该秒级完成，真要顶到分钟级说明这条连接已经不可用了，
    // 没必要让用户等到那个上限才看到结果；这两个探测本来就是"尽力而为"的
    // 增强（提示词的锦上添花），超时了就当探测失败处理，不阻塞会话真正可用。
    let (memory, skills) =
        probe_workspace(workspace_id, &profile.root_path, file_ops.as_ref()).await;

    let id = override_id.unwrap_or_else(Uuid::new_v4);
    let change_store = Arc::new(Mutex::new(ChangeStore::new(
        id,
        profile.root_path.clone(),
        target.clone(),
        file_ops.clone(),
        false,
    )));
    let mut session = CodingSession::new(
        id,
        workspace_id,
        profile.root_path.clone(),
        target,
        provider_id,
        file_ops,
        change_store.clone(),
    );
    session.apply_project_memory(memory);
    session.apply_skills(skills);

    // 双引擎路由（docs/CODEX_INTEGRATION_PLAN.md Phase 3/4/6，2026-09 改为"codex 为核心"）：
    // 不再局限于 `CodingTarget::Local`——`SessionExecTarget`（`codex_exec_target.rs`）
    // 已经按目标类型（Local/Remote-SSH/Agent-Windows）翻译 codex-core 的 `cwd`/路径，
    // Local/Remote/Agent 三态都默认尝试挂载 `CodexCoreEngine`；任何一步失败（初始化
    // 报错、或者 provider 协议本身跟 codex-core 写死的 Responses API 不兼容）都静默
    // 回退到自研引擎，不能因为这条路径的问题让"开始 AI 会话"本身失败。
    if let Ok(Some(provider)) = state.ai_provider_manager.get(provider_id) {
        if crate::coding::routes_to_codex_engine(&provider) {
            match state.ai_provider_manager.resolve_api_key(&provider).await {
                Ok(Some(api_key)) => {
                    if let Err(e) = attach_codex_engine(&mut session, &provider, api_key, &change_store, state, app_handle).await {
                        tracing::warn!("workspace {workspace_id}: codex engine 初始化失败，回退到自研引擎: {e}");
                    }
                }
                Ok(None) => tracing::warn!("workspace {workspace_id}: provider {} 命中 codex 路由但没有配置 API Key，回退到自研引擎", provider.name),
                Err(e) => tracing::warn!("workspace {workspace_id}: 读取 API Key 失败，回退到自研引擎: {e}"),
            }
        }
    }

    if resume_recent {
        if let Ok(histories) = history_list_with_import(state, workspace_id).await {
            if let Some(latest) = histories.first() {
                let recent = chrono::DateTime::parse_from_rfc3339(&latest.updated_at)
                    .map(|t| {
                        chrono::Utc::now().signed_duration_since(t.with_timezone(&chrono::Utc))
                            <= chrono::Duration::hours(12)
                    })
                    .unwrap_or(false);
                if recent {
                    if let Ok(Some(detail)) = state.coding_history.get(latest.id) {
                        if let Ok(changes) =
                            serde_json::from_value::<Vec<FileChange>>(detail.changes)
                        {
                            change_store.lock().await.restore(changes);
                        }
                    }
                }
            }
        }
    }

    Ok((session, change_store))
}

/// 构造并挂载 `CodexCoreEngine`——起一个专属的 `RocDeskExecServer`（本地
/// loopback WebSocket，见 `codex-engine` crate），把 codex-core 的 `fs/*`/
/// `process/*` 请求路由到 `SessionExecTarget`（复用现有 `ChangeStore`/权限规则
/// 引擎），再用 roc_desk 自己管理的 API Key 构造 `ThreadManager`/`CodexThread`。
/// `codex_home` 用本机临时目录、按 session id 分开，不是用户全局的 `~/.codex`
/// ——不需要和真的装了官方 codex CLI 的用户产生状态冲突，`Config.ephemeral =
/// true` 也没打算依赖这份状态跨进程重启存活。
async fn attach_codex_engine(
    session: &mut CodingSession,
    provider: &crate::ai::AiProvider,
    api_key: String,
    change_store: &Arc<Mutex<ChangeStore>>,
    state: &State<'_, AppState>,
    app_handle: &AppHandle,
) -> Result<(), AppError> {
    let turn_id = Arc::new(std::sync::Mutex::new(Uuid::new_v4()));
    let codex_home = std::env::temp_dir()
        .join("roc_desk")
        .join("codex_home")
        .join(session.id.to_string());
    // codex-core 内部用 `AbsolutePathBuf`/`PathUri` 处理 `cwd`，这两个类型按"运行
    // roc_desk 的这台宿主机（Windows）的原生路径语法"解析——对 `CodingTarget::Local`
    // 这没问题（`workspace_root` 本来就是本机真实路径）；但对 Remote(SSH)/Agent 这种
    // 远程目标，`workspace_root` 是远端主机上的路径（比如 SSH 上的
    // `/data/lipeng/kgmscli`），不是 Windows 绝对路径语法，直接传给 codex-core 会被
    // 它内部的路径归一化按 Windows 语义强行拼出一个跟真实远程路径毫不相干的本机路径。
    // 这里改传一个真实存在于本机磁盘的占位目录哄 codex-core——它自己从不读写这个
    // 目录的内容，所有 `fs/*`/`process/*` 请求最终都会经过 `ExecTarget` 转发到
    // SSH/Agent，`SessionExecTarget::resolve_workspace_path` 负责把"相对这个占位
    // 目录的路径"翻译回真正的远程 `workspace_root`。
    let local_cwd_placeholder = if matches!(session.target, CodingTarget::Local) {
        None
    } else {
        let placeholder = codex_home.join("cwd");
        tokio::fs::create_dir_all(&placeholder)
            .await
            .map_err(|e| AppError::Internal(format!("创建 codex cwd 占位目录失败: {e}")))?;
        Some(placeholder)
    };
    let engine_cwd = local_cwd_placeholder
        .clone()
        .unwrap_or_else(|| std::path::PathBuf::from(&session.workspace_root));
    let exec_target: Arc<dyn codex_engine::ExecTarget> =
        Arc::new(crate::coding::codex_exec_target::SessionExecTarget::new(
            session.id,
            session.workspace_root.clone(),
            session.target.clone(),
            local_cwd_placeholder,
            session.codex_mode.clone(),
            session.codex_auto_allow_readonly.clone(),
            session.file_ops.clone(),
            change_store.clone(),
            turn_id.clone(),
            state.ssh_pool.clone(),
            state.agent_pool.clone(),
            state.audit_log.clone(),
            state.command_confirms.clone(),
            state.permission_rules.clone(),
            app_handle.clone(),
        ));
    let exec_server = codex_engine::RocDeskExecServer::bind(exec_target)
        .await
        .map_err(|e| AppError::Internal(format!("exec-server bind: {e}")))?;
    let options = codex_engine::CodexCoreEngineOptions {
        api_key,
        model: Some(provider.model.clone()),
        // roc_desk 配置的 provider 永远传自己的 `api_base`，不用 codex 内建的
        // 官方 OpenAI provider——即使 URL 命中了 `routes_to_codex_engine` 的
        // "看起来像官方/Azure/Bedrock" 启发式，也应该按用户实际配置的地址请求，
        // 而不是悄悄地打真正的 api.openai.com。
        base_url: Some(provider.api_base.clone()),
        codex_home,
        cwd: engine_cwd,
        exec_server_url: exec_server.websocket_url(),
        environment_id: format!("roc-desk-{}", session.id),
    };
    // `CodexCoreEngineHandle::spawn`（不是直接 `CodexCoreEngine::new`）——见
    // codex-engine/src/engine.rs 顶部注释：codex-core 的深层异步调用链在默认
    // 线程栈大小下会栈溢出（Windows 上是进程级致命错误），必须挪到专用大栈
    // 线程上跑，这里只是异步等一个"初始化完成"的信号。
    let engine = codex_engine::CodexCoreEngineHandle::spawn(options)
        .await
        .map_err(|e| AppError::Internal(format!("codex core engine: {e}")))?;
    session.attach_codex_engine(engine, exec_server, turn_id);
    Ok(())
}

/// 自动绑定当前工作区打开（或复用已有的）编程助手会话（DESIGN.md §3.8.1）。
#[tauri::command]
pub async fn coding_start(
    state: State<'_, AppState>,
    app_handle: AppHandle,
    workspace_id: Uuid,
    provider_id: Uuid,
) -> Result<CodingSessionInfo, AppError> {
    if state.ai_provider_manager.get(provider_id)?.is_none() {
        return Err(AppError::NotFound(format!(
            "ai provider not found: {provider_id}"
        )));
    }

    let workspaces = state.workspaces.read().await;
    let handle = workspaces
        .get(&workspace_id)
        .ok_or_else(|| AppError::NotFound(format!("workspace not opened: {workspace_id}")))?;
    let profile = handle.profile.clone();
    drop(workspaces);

    if let Some(existing) = state
        .coding_sessions
        .read()
        .await
        .get(&workspace_id)
        .cloned()
    {
        let guard = existing.lock().await;
        let target_matches = match (&guard.target, profile.kind, profile.connection_id) {
            (CodingTarget::Local, WorkspaceKind::Local, None) => true,
            (CodingTarget::Remote { connection_id, .. }, WorkspaceKind::Remote, Some(expected)) => {
                *connection_id == expected
            }
            (CodingTarget::Agent { connection_id, .. }, WorkspaceKind::Remote, Some(expected)) => {
                *connection_id == expected
            }
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

    let (session, change_store) =
        build_new_session(&state, workspace_id, provider_id, true, None, &app_handle).await?;
    let info = session_info(&session).await;
    state
        .coding_sessions
        .write()
        .await
        .insert(workspace_id, Arc::new(Mutex::new(session)));
    state
        .coding_changes
        .write()
        .await
        .insert(workspace_id, change_store);
    Ok(info)
}

#[tauri::command]
pub async fn coding_new_session(
    state: State<'_, AppState>,
    app_handle: AppHandle,
    workspace_id: Uuid,
    provider_id: Uuid,
) -> Result<CodingSessionInfo, AppError> {
    if state.ai_provider_manager.get(provider_id)?.is_none() {
        return Err(AppError::NotFound(format!(
            "ai provider not found: {provider_id}"
        )));
    }
    state.coding_sessions.write().await.remove(&workspace_id);
    state.coding_changes.write().await.remove(&workspace_id);
    let (session, change_store) =
        build_new_session(&state, workspace_id, provider_id, false, None, &app_handle).await?;
    let info = session_info(&session).await;
    state
        .coding_sessions
        .write()
        .await
        .insert(workspace_id, Arc::new(Mutex::new(session)));
    state
        .coding_changes
        .write()
        .await
        .insert(workspace_id, change_store);
    Ok(info)
}

/// 释放一个工作区的常驻编程助手会话（前端有界保活策略的 LRU 淘汰、或工作区
/// 彻底关闭时调用）。会话内存直接丢弃——对话内容早已在每次
/// `sendMessage`/`acceptChange` 等操作后通过 `saveCurrentHistory` 落库，
/// 不需要在这里补一次"关闭前保存"。找不到对应会话（从未打开过，或已经被
/// 释放过）不算错误，幂等处理。
#[tauri::command]
pub async fn coding_close(state: State<'_, AppState>, workspace_id: Uuid) -> Result<(), AppError> {
    state.coding_sessions.write().await.remove(&workspace_id);
    state.coding_changes.write().await.remove(&workspace_id);
    Ok(())
}

#[tauri::command]
pub async fn coding_set_mode(
    state: State<'_, AppState>,
    workspace_id: Uuid,
    mode: CodingMode,
) -> Result<(), AppError> {
    let session = get_session(&state, workspace_id).await?;
    let mut session = session.lock().await;
    session.mode = mode;
    session.codex_mode.store(
        match mode {
            CodingMode::Plan => 0,
            CodingMode::Build => 1,
        },
        std::sync::atomic::Ordering::Relaxed,
    );
    Ok(())
}

#[tauri::command]
pub async fn coding_set_auto_allow_readonly(
    state: State<'_, AppState>,
    workspace_id: Uuid,
    enabled: bool,
) -> Result<(), AppError> {
    let session = get_session(&state, workspace_id).await?;
    let mut session = session.lock().await;
    session.auto_allow_readonly = enabled;
    session
        .codex_auto_allow_readonly
        .store(enabled, std::sync::atomic::Ordering::Relaxed);
    Ok(())
}

/// 开关"每次 Accept 一条变更就自动 git add + commit"（DESIGN.md §3.8.2）；
/// `session.git_repo` 为 false（工作区根目录不是 Git 仓库）时前端应该把这个开关
/// disable 掉，这里不重复做校验——就算开了也不会有任何效果（`ChangeStore::accept`
/// 里 `auto_git_commit && git_repo` 才会真的去跑 git 命令）。走独立的
/// `coding_changes` 锁，不碰 `CodingSession`——和下面 accept/reject/undo 系列
/// 命令同理，见 `ChangeStore` 文档。
#[tauri::command]
pub async fn coding_set_auto_git_commit(
    state: State<'_, AppState>,
    workspace_id: Uuid,
    enabled: bool,
) -> Result<(), AppError> {
    let store = get_change_store(&state, workspace_id).await?;
    // 会话开始时不再预先探测 Git 仓库（2026-09 用户要求，见 `build_new_session`
    // 里的说明）——只有用户真的点开这个开关、且当前判定还不是 git 仓库时，
    // 才按需探测一次；已经判定过是仓库就不用重复探测。这样"工作区不用 git"
    // 的用户完全碰不到这个探测，只有真的想用自动提交的人才会等这一次。
    {
        let mut guard = store.lock().await;
        if enabled && !guard.git_repo() {
            let target = guard.target().clone();
            let workspace_root = guard.workspace_root().to_string();
            drop(guard);
            let git_repo = tokio::time::timeout(
                WORKSPACE_PROBE_TIMEOUT,
                crate::coding::git_ops::is_git_repo(
                    &target,
                    &workspace_root,
                    &state.ssh_pool,
                    &state.agent_pool,
                ),
            )
            .await
            .unwrap_or(false);
            if !git_repo {
                return Err(AppError::Internal(
                    "当前工作区不是 Git 仓库，无法开启自动提交".to_string(),
                ));
            }
            guard = store.lock().await;
            guard.set_git_repo(true);
        }
        guard.auto_git_commit = enabled;
    }
    Ok(())
}

/// "完全授权模式"开关（用户 2026-09 反馈："工具改动 20 多个文件都需要用户确认，
/// 这个过程太繁琐，需要有完全授权模式"）：开启后 AI 提出的文件改动不再生成 Diff
/// 等 Accept，直接落盘。会话级——每个工作区的编程助手会话各自独立开关。
#[tauri::command]
pub async fn coding_set_full_auto(
    state: State<'_, AppState>,
    workspace_id: Uuid,
    enabled: bool,
) -> Result<(), AppError> {
    let store = get_change_store(&state, workspace_id).await?;
    let session_id = {
        let store = store.lock().await;
        store
            .full_auto
            .store(enabled, std::sync::atomic::Ordering::Relaxed);
        store.session_id()
    };
    if enabled {
        state.command_confirms.allow_session(session_id).await;
    }
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
    // "停止"按钮用的取消信号——必须在拿 `coding_sessions` 那把锁之前注册好，
    // 且存在独立的 `coding_cancel_tokens` 里，不能等锁拿到之后再注册：
    // `coding_cancel_turn` 本身不需要（也不能）等这把锁，不然长对话轮次进行
    // 期间它会被一起卡住，"停止"就失去意义了（见 `state.rs` 里这个字段的
    // 文档注释）。同一个工作区如果上一轮还留着一个陈旧的 token（正常不会，
    // 但防御一下），直接覆盖。
    let cancel_token = tokio_util::sync::CancellationToken::new();
    state
        .coding_cancel_tokens
        .lock()
        .unwrap()
        .insert(workspace_id, cancel_token.clone());
    let mut session = session.lock().await;
    let result = session
        .send_message(
            &text,
            &attachments.unwrap_or_default(),
            &state.ai_provider_manager,
            &state.ssh_pool,
            &state.agent_pool,
            &state.audit_log,
            &state.command_confirms,
            &state.permission_rules,
            &state.question_confirms,
            &state.mcp_manager,
            &app_handle,
            &cancel_token,
        )
        .await;
    // 这一轮已经结束（不管成功/失败/被取消），把 token 摘掉——如果还留着，
    // 下一轮开始前会被新 token 覆盖，摘不摘掉不影响正确性，只是不让 map
    // 里堆积用不到的旧 token。
    state
        .coding_cancel_tokens
        .lock()
        .unwrap()
        .remove(&workspace_id);
    result
}

/// "停止"按钮——只是把 `workspace_id` 对应的 `CancellationToken` 标记为已取消，
/// 不需要拿 `coding_sessions` 的锁，所以哪怕 `coding_send_message` 那一轮还卡在
/// 一次很慢的网络请求里，这个命令也能立刻返回、让取消信号马上生效（见
/// `state.rs` 里 `coding_cancel_tokens` 的文档注释）。当前没有正在跑的轮次
/// （map 里找不到这个 workspace_id）不算错误，静默忽略。
#[tauri::command]
pub async fn coding_cancel_turn(
    state: State<'_, AppState>,
    workspace_id: Uuid,
) -> Result<(), AppError> {
    if let Some(token) = state.coding_cancel_tokens.lock().unwrap().get(&workspace_id) {
        token.cancel();
    }
    Ok(())
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
pub async fn coding_optimize_prompt(
    state: State<'_, AppState>,
    workspace_id: Uuid,
    text: String,
) -> Result<String, AppError> {
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
pub async fn coding_answer_question(
    state: State<'_, AppState>,
    request_id: Uuid,
    answer: String,
) -> Result<(), AppError> {
    state.question_confirms.resolve(request_id, answer).await;
    Ok(())
}

// ---- 权限规则（REQUIREMENTS.md §3.7 权限引擎升级）----

#[tauri::command]
pub async fn permission_rule_list(
    state: State<'_, AppState>,
) -> Result<Vec<PermissionRule>, AppError> {
    state.permission_rules.list()
}

#[derive(serde::Deserialize)]
pub struct PermissionRuleInput {
    pub tool: String,
    pub pattern: String,
    pub decision: String,
}

#[tauri::command]
pub async fn permission_rule_create(
    state: State<'_, AppState>,
    input: PermissionRuleInput,
) -> Result<PermissionRule, AppError> {
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
pub async fn mcp_server_create(
    state: State<'_, AppState>,
    input: McpServerInput,
) -> Result<McpServer, AppError> {
    state.mcp_manager.create(input).await
}

#[tauri::command]
pub async fn mcp_server_update(
    state: State<'_, AppState>,
    id: Uuid,
    input: McpServerInput,
) -> Result<McpServer, AppError> {
    state.mcp_manager.update(id, input).await
}

#[tauri::command]
pub async fn mcp_server_delete(state: State<'_, AppState>, id: Uuid) -> Result<(), AppError> {
    state.mcp_manager.delete(id).await
}

// Accept/Reject/Undo/Redo/RevertTurn 都直接操作 `state.coding_changes` 里独立
// 加锁的 `ChangeStore`，完全不碰 `state.coding_sessions`/`CodingSession` 的锁——
// 这样即使 AI 还在同一个工作区里跑一轮可能长达一两分钟的对话（`session.lock()`
// 被 `coding_send_message` 一直持有），用户点某张已经弹出来的文件改动卡片的
// "应用/拒绝"也能立刻生效，不用排队等那一轮说完（2026-09 用户真实反馈：一次改
// 20 多个文件、逐个确认很繁琐，且中途点确认没反应，见 `ChangeStore` 文档）。
#[tauri::command]
pub async fn coding_accept_change(
    state: State<'_, AppState>,
    app_handle: AppHandle,
    workspace_id: Uuid,
    change_id: Uuid,
) -> Result<FileSyncInfo, AppError> {
    let store = get_change_store(&state, workspace_id).await?;
    let mut guard = store.lock().await;
    guard
        .accept(change_id, &state.ssh_pool, &state.agent_pool, &app_handle)
        .await
}

#[tauri::command]
pub async fn coding_reject_change(
    state: State<'_, AppState>,
    workspace_id: Uuid,
    change_id: Uuid,
) -> Result<(), AppError> {
    let store = get_change_store(&state, workspace_id).await?;
    let mut guard = store.lock().await;
    guard.reject(change_id)
}

#[tauri::command]
pub async fn coding_undo_change(
    state: State<'_, AppState>,
    workspace_id: Uuid,
    change_id: Uuid,
) -> Result<FileSyncInfo, AppError> {
    let store = get_change_store(&state, workspace_id).await?;
    let mut guard = store.lock().await;
    guard.undo(change_id).await
}

#[tauri::command]
pub async fn coding_redo_change(
    state: State<'_, AppState>,
    workspace_id: Uuid,
) -> Result<Option<FileSyncInfo>, AppError> {
    let store = get_change_store(&state, workspace_id).await?;
    let mut guard = store.lock().await;
    guard.redo().await
}

/// 撤销某一轮对话里 AI 做出的全部已应用改动（DESIGN.md/项目内部设计讨论：
/// 参考 Cursor/Windsurf 的按轮次整体撤销，但不依赖 git）。
#[tauri::command]
pub async fn coding_revert_turn(
    state: State<'_, AppState>,
    workspace_id: Uuid,
    turn_id: Uuid,
) -> Result<Vec<FileSyncInfo>, AppError> {
    let store = get_change_store(&state, workspace_id).await?;
    let mut guard = store.lock().await;
    guard.revert_turn(turn_id).await
}

/// 响应 `coding:command-confirm-request`（DESIGN.md §3.8.2.1）。
#[tauri::command]
pub async fn coding_confirm_command(
    state: State<'_, AppState>,
    request_id: Uuid,
    allow: bool,
) -> Result<(), AppError> {
    state.command_confirms.resolve(request_id, allow).await;
    Ok(())
}

async fn get_session(
    state: &State<'_, AppState>,
    workspace_id: Uuid,
) -> Result<Arc<Mutex<CodingSession>>, AppError> {
    state
        .coding_sessions
        .read()
        .await
        .get(&workspace_id)
        .cloned()
        .ok_or_else(|| {
            AppError::NotFound(format!(
                "no coding session for workspace {workspace_id}, call coding_start first"
            ))
        })
}

async fn get_change_store(
    state: &State<'_, AppState>,
    workspace_id: Uuid,
) -> Result<Arc<Mutex<ChangeStore>>, AppError> {
    state
        .coding_changes
        .read()
        .await
        .get(&workspace_id)
        .cloned()
        .ok_or_else(|| {
            AppError::NotFound(format!(
                "no coding session for workspace {workspace_id}, call coding_start first"
            ))
        })
}

#[tauri::command]
pub async fn coding_history_save(
    state: State<'_, AppState>,
    input: CodingHistoryInput,
) -> Result<(), AppError> {
    // 前端不知道也不需要填 `messages`（发给 AI 的真实对话上下文，见
    // `CodingHistoryInput::messages` 的文档）——这里用当前存活会话的最新内容
    // 覆盖，保证落库的历史记录能被 `coding_history_resume` 真正续上。找不到
    // 存活会话（理论上不会发生：调用这个命令前一定已经 `coding_start` 过）就
    // 保留输入原样（`#[serde(default)]` 下是 `Value::Null`）。
    let mut input = input;
    if let Some(session) = state
        .coding_sessions
        .read()
        .await
        .get(&input.workspace_id)
        .cloned()
    {
        let messages = session.lock().await.messages_snapshot();
        input.messages = serde_json::to_value(&messages).unwrap_or_default();
    }
    state.coding_history.save(&input)?;
    if let Some(detail) = state.coding_history.get(input.id)? {
        let snapshot = WorkspaceHistorySnapshot {
            input: input.clone(),
            created_at: detail.summary.created_at,
            updated_at: detail.summary.updated_at,
        };
        if let Some(handle) = state
            .workspaces
            .read()
            .await
            .get(&input.workspace_id)
            .cloned()
        {
            let dir = format!(
                "{}/.rock_desk/sessions",
                handle.profile.root_path.trim_end_matches(['/', '\\'])
            );
            let path = format!("{dir}/{}.json", input.id);
            if let Ok(json) = serde_json::to_string_pretty(&snapshot) {
                // 工作区镜像是第二份、尽力而为的持久化；SQLite 已经在上面同步成功。
                // 绝不能让一次慢/断开的远程 SFTP 写入阻塞"新会话"按钮，所以放到后台。
                let history_id = input.id;
                tokio::spawn(async move {
                    let mirrored = tokio::time::timeout(WORKSPACE_PROBE_TIMEOUT, async {
                        handle
                            .file_ops
                            .create_dir(&format!(
                                "{}/.rock_desk",
                                handle.profile.root_path.trim_end_matches(['/', '\\'])
                            ))
                            .await?;
                        handle.file_ops.create_dir(&dir).await?;
                        handle.file_ops.write_file(&path, &json, None).await?;
                        Ok::<(), AppError>(())
                    })
                    .await;
                    if !matches!(mirrored, Ok(Ok(()))) {
                        tracing::warn!(%path, "failed to mirror coding history into workspace cache");
                        let fallback = handle.fallback_cache_dir.join("sessions");
                        if std::fs::create_dir_all(&fallback).is_ok() {
                            let _ =
                                std::fs::write(fallback.join(format!("{history_id}.json")), json);
                        }
                    }
                });
            }
        }
    }
    Ok(())
}

/// 列历史前先把工作区自带的 `.rock_desk/sessions/*.json` 快照（`coding_history_save`
/// 写的那份，见其注释——本地 SQLite 之外的第二份持久化，远程工作区也能带着走）
/// 导入本地 SQLite 缓存，再从缓存里查。`build_new_session` 复用同一个函数来找
/// "最近一次会话"用于恢复 `session.changes`，两处口径必须一致，不能各写一份。
async fn history_list_with_import(
    state: &State<'_, AppState>,
    workspace_id: Uuid,
) -> Result<Vec<CodingHistorySummary>, AppError> {
    if let Some(handle) = state.workspaces.read().await.get(&workspace_id).cloned() {
        let dir = format!(
            "{}/.rock_desk/sessions",
            handle.profile.root_path.trim_end_matches(['/', '\\'])
        );
        // `list_dir`本身也套超时——连接卡住时，宁可这次列历史看不到远程新增的
        // 快照（下次再列自然会补上），也不能让"打开 AI 工具"卡死在这一步
        // （2026-09 用户反馈"打开远程工作区的 AI 工具等很久才看到历史界面"）。
        let entries = tokio::time::timeout(WORKSPACE_PROBE_TIMEOUT, handle.file_ops.list_dir(&dir))
            .await
            .unwrap_or(Err(AppError::Internal("list_dir timed out".into())));
        if let Ok(entries) = entries {
            // 本地已知的 id -> updated_at（unix 秒）——`.rock_desk/sessions/` 会
            // 随这个工作区历史上开过的每个会话持续增长，`coding_history_save`
            // 又几乎每隔几秒/每次 Accept 就重写一次当前活跃会话对应的文件；不做
            // 这层短路的话，每次列历史都要把目录下全部快照文件整份用 SFTP/Agent
            // 协议读一遍再反序列化，文件一多、远程延迟一高就会非常慢——这正是
            // 用户反馈"打开历史会话/新建会话半天没反应"的根因（不是卡死，是真的
            // 在老老实实地把几十个文件挨个读一遍）。文件名本身就是 `{id}.json`，
            // 不需要真读内容就能判断"这个 id 本地是不是已经有更新的版本"。
            // 不能拿远端文件 mtime 和 SQLite 的 updated_at 作比较：它们来自不同机器
            // 的时钟，远端稍快几秒就会让每次列历史都误判成"全部变更"，反复传回每个
            // 大快照。改用本进程实际同步过的 `(history id, remote mtime)` 标记。
            //
            // 初次打开一个带有很多历史的远程工作区时，也只同步最新几条：历史列表要
            // 先可用，老会话等用户点击时 `coding_history_get` 会精确按需刷新，无须在
            // 打开面板的关键路径上下载几十份完整 timeline/messages 快照。
            let mut to_fetch: Vec<SnapshotCandidate> = entries
                .into_iter()
                .filter(|entry| !entry.is_dir && entry.name.ends_with(".json"))
                .filter_map(|entry| {
                    let id = entry.name.strip_suffix(".json").and_then(|value| Uuid::parse_str(value).ok());
                    let synced = matches!((id, entry.modified), (Some(id), Some(mtime)) if snapshot_already_synced(workspace_id, id, mtime));
                    (!synced).then_some(SnapshotCandidate { path: entry.path, mtime: entry.modified })
                })
                .collect();
            to_fetch
                .sort_by_key(|candidate| std::cmp::Reverse(candidate.mtime.unwrap_or_default()));
            to_fetch.truncate(INLINE_SNAPSHOT_SYNC_LIMIT);
            let fetches = to_fetch.iter().map(|candidate| {
                import_workspace_snapshot(
                    handle.file_ops.as_ref(),
                    state.coding_history.as_ref(),
                    workspace_id,
                    candidate,
                )
            });
            futures_util::future::join_all(fetches).await;
        }
        let fallback = handle.fallback_cache_dir.join("sessions");
        if let Ok(entries) = std::fs::read_dir(fallback) {
            for entry in entries.flatten() {
                if let Ok(bytes) = std::fs::read(entry.path()) {
                    if let Ok(snapshot) = serde_json::from_slice::<WorkspaceHistorySnapshot>(&bytes)
                    {
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
pub async fn coding_history_list(
    state: State<'_, AppState>,
    workspace_id: Uuid,
) -> Result<Vec<CodingHistorySummary>, AppError> {
    history_list_with_import(&state, workspace_id).await
}

/// 单条历史记录的"以工作区目录为准"刷新——`history_list_with_import` 是列表场景
/// 用的批量对账（列一次目录、跳过没变化的文件），这里是打开/查看具体某一条
/// 历史时用的精确单文件对账：工作区数据要"存哪儿就以哪儿为准"（2026-09 用户
/// 明确要求"工作区本身的会话历史和数据全存工作区目录，不是本地目录"）——如果
/// 换了一台机器/换了另一个 roc_desk.exe 副本（便携版，全局状态是按 exe 所在
/// 目录分开存的，见 `resolve_app_data_dir`），本地 SQLite 缓存里这条记录可能
/// 是旧的甚至压根没有，只有工作区目录下的 `.rock_desk/sessions/{id}.json`
/// 才是所有副本共享的真相来源。读取失败（网络问题/文件不存在）不当错误处理，
/// 静默回退到本地缓存已有的内容——这是"尽量拿到最新"，不是"必须拿到最新"。
async fn refresh_history_from_workspace(
    state: &State<'_, AppState>,
    workspace_id: Uuid,
    history_id: Uuid,
) {
    let Some(handle) = state.workspaces.read().await.get(&workspace_id).cloned() else {
        return;
    };
    let path = format!(
        "{}/.rock_desk/sessions/{history_id}.json",
        handle.profile.root_path.trim_end_matches(['/', '\\'])
    );
    if let Ok(Ok(file)) =
        tokio::time::timeout(WORKSPACE_PROBE_TIMEOUT, handle.file_ops.read_file(&path)).await
    {
        if let Ok(snapshot) = serde_json::from_str::<WorkspaceHistorySnapshot>(&file.text) {
            if snapshot.input.workspace_id == workspace_id {
                let _ = state.coding_history.import_snapshot(&snapshot);
                return;
            }
        }
    }
    let fallback = handle
        .fallback_cache_dir
        .join("sessions")
        .join(format!("{history_id}.json"));
    if let Ok(bytes) = std::fs::read(fallback) {
        if let Ok(snapshot) = serde_json::from_slice::<WorkspaceHistorySnapshot>(&bytes) {
            if snapshot.input.workspace_id == workspace_id {
                let _ = state.coding_history.import_snapshot(&snapshot);
            }
        }
    }
}

#[tauri::command]
pub async fn coding_history_get(
    state: State<'_, AppState>,
    id: Uuid,
) -> Result<Option<CodingHistoryDetail>, AppError> {
    if let Some(existing) = state.coding_history.get(id)? {
        refresh_history_from_workspace(&state, existing.workspace_id, id).await;
    }
    state.coding_history.get(id)
}

/// 打开一条历史记录并真正接续对话（不是只读回放）——用户 2026-09 反馈"历史会话
/// 只能只读不能继续修改"。把持久化的 `messages`（发给 AI 的真实对话上下文）和
/// `changes`（文件改动记录）都灌回一个新构造的 `CodingSession`/`ChangeStore`，
/// 替换掉这个工作区当前的活跃会话；`session.id` 复用这条历史记录自己的 id，
/// 后续 `coding_send_message` 产生新对话轮次时 `saveCurrentHistory` 更新的还是
/// 同一行记录，不会分裂出一条新历史。
#[tauri::command]
pub async fn coding_history_resume(
    state: State<'_, AppState>,
    app_handle: AppHandle,
    workspace_id: Uuid,
    history_id: Uuid,
) -> Result<CodingSessionInfo, AppError> {
    // 打开历史会话续聊时，`workspace_id`/`history_id` 都是调用方直接给的，不
    // 依赖本地 SQLite 是否已经见过这条记录——即使换了一台机器/换了另一个
    // roc_desk.exe 便携版副本、本地缓存里压根没有这条历史，也能直接从工作区
    // 目录把它找回来（见 `refresh_history_from_workspace` 的文档）。
    refresh_history_from_workspace(&state, workspace_id, history_id).await;
    let detail = state
        .coding_history
        .get(history_id)?
        .ok_or_else(|| AppError::NotFound(format!("history not found: {history_id}")))?;
    if detail.workspace_id != workspace_id {
        return Err(AppError::Internal("这条历史记录不属于当前工作区".into()));
    }
    if state
        .ai_provider_manager
        .get(detail.summary.provider_id)?
        .is_none()
    {
        return Err(AppError::NotFound(
            "这条历史记录关联的 AI 供应商已被删除，请先在模型管理里重新配置后再试".into(),
        ));
    }

    // `override_id: Some(history_id)` ——之前是构造完会话再 `session.id =
    // history_id` 事后覆盖，但 codex 引擎的挂载（`build_new_session` 内部）要用
    // 最终的 session id 构造 `SessionExecTarget`/审计日志/事件广播，事后覆盖会
    // 导致这些地方用的还是构造时随手生成的临时 id，和前端实际展示的 session id
    // 对不上。
    let (mut session, change_store) = build_new_session(
        &state,
        workspace_id,
        detail.summary.provider_id,
        false,
        Some(history_id),
        &app_handle,
    )
    .await?;
    session.mode = if detail.summary.mode == "build" {
        CodingMode::Build
    } else {
        CodingMode::Plan
    };
    let messages: Vec<serde_json::Value> =
        serde_json::from_value(detail.messages.clone()).unwrap_or_default();
    session.restore_messages(messages);
    let changes: Vec<FileChange> =
        serde_json::from_value(detail.changes.clone()).unwrap_or_default();
    change_store.lock().await.restore(changes);

    let info = session_info(&session).await;
    state
        .coding_sessions
        .write()
        .await
        .insert(workspace_id, Arc::new(Mutex::new(session)));
    state
        .coding_changes
        .write()
        .await
        .insert(workspace_id, change_store);
    Ok(info)
}

#[tauri::command]
pub async fn coding_history_rename(
    state: State<'_, AppState>,
    id: Uuid,
    title: String,
) -> Result<(), AppError> {
    state.coding_history.rename(id, title.trim())
}

#[tauri::command]
pub async fn coding_history_delete(state: State<'_, AppState>, id: Uuid) -> Result<(), AppError> {
    state.coding_history.delete(id)
}
