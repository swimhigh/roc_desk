use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::Instant;

use serde::Serialize;
use serde_json::json;
use tauri::{AppHandle, Emitter, State};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::coding::permission::{Decision, PermissionRule};
use crate::coding::skills::SkillMeta;
use crate::coding::tools::TodoItem;
use crate::coding::{
    ChangeStatus, ChangeStore, CodingMode, CodingSession, CodingTarget, FileChange, FileSyncInfo,
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
        auto_allow_readonly: store
            .auto_allow_readonly
            .load(std::sync::atomic::Ordering::Relaxed),
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
    workspace_id: Uuid,
    provider_id: Uuid,
) -> Result<(), AppError> {
    let Some(_provider) = state.ai_provider_manager.get(provider_id)? else {
        return Err(AppError::NotFound(format!(
            "ai provider not found: {provider_id}"
        )));
    };
    let session = get_session(&state, workspace_id).await?;
    let mut guard = session.lock().await;
    guard.provider_id = provider_id;
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

/// 自动绑定当前工作区打开（或复用已有的）编程助手会话（DESIGN.md §3.8.1）。
#[tauri::command]
pub async fn coding_start(
    state: State<'_, AppState>,
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
        build_new_session(&state, workspace_id, provider_id, true, None).await?;
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
        build_new_session(&state, workspace_id, provider_id, false, None).await?;
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
    Ok(())
}

/// "自动放行只读命令"开关——走独立的 `coding_changes` 锁，不碰 `CodingSession`
/// 自己那把锁（和 `coding_set_full_auto`/`coding_set_auto_git_commit` 同理，见
/// `ChangeStore::auto_allow_readonly` 的文档）：这个值原来是 `CodingSession` 的
/// 普通字段，`send_message` 处理一轮对话期间会一直持有会话锁，用户在 AI 任务
/// 运行中点这个开关会排队等当前这轮说完才生效，界面上看起来"点了没反应"
/// （2026-09 用户反馈）。
#[tauri::command]
pub async fn coding_set_auto_allow_readonly(
    state: State<'_, AppState>,
    workspace_id: Uuid,
    enabled: bool,
) -> Result<(), AppError> {
    let store = get_change_store(&state, workspace_id).await?;
    store
        .lock()
        .await
        .auto_allow_readonly
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

// ---- Skills 查看 / 导入（`.rock_desk/skills/<name>/SKILL.md`，见
// `coding::skills` 顶部文档；此前只有后端发现+运行时加载，没有图形化管理入口，
// 2026-09 用户反馈"看不到导入技能的地方"补上）----

#[tauri::command]
pub async fn skill_list(
    state: State<'_, AppState>,
    workspace_id: Uuid,
) -> Result<Vec<SkillMeta>, AppError> {
    let handle = state
        .workspaces
        .read()
        .await
        .get(&workspace_id)
        .cloned()
        .ok_or_else(|| AppError::NotFound(format!("workspace not opened: {workspace_id}")))?;
    Ok(crate::coding::skills::discover_skills(handle.file_ops.as_ref(), &handle.profile.root_path).await)
}

#[tauri::command]
pub async fn skill_delete(
    state: State<'_, AppState>,
    workspace_id: Uuid,
    name: String,
) -> Result<(), AppError> {
    let handle = state
        .workspaces
        .read()
        .await
        .get(&workspace_id)
        .cloned()
        .ok_or_else(|| AppError::NotFound(format!("workspace not opened: {workspace_id}")))?;
    let skills =
        crate::coding::skills::discover_skills(handle.file_ops.as_ref(), &handle.profile.root_path).await;
    let skill = skills
        .into_iter()
        .find(|s| s.name == name)
        .ok_or_else(|| AppError::NotFound(format!("未找到技能：{name}")))?;
    handle.file_ops.delete(&skill.dir, true).await?;
    if let Ok(mut guard) = caches().lock() {
        guard.probes.remove(&workspace_id);
    }
    Ok(())
}

/// `skill_import` 处理压缩包时用到的临时解压目录——不管导入成功还是中途报错都要
/// 清掉，用 `Drop` 保证这一点，不用在每个 `?` 提前返回的分支各写一遍清理代码。
struct TempDirGuard(std::path::PathBuf);
impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// 把本地磁盘上一个技能目录（含 `SKILL.md`，可以是已解压的文件夹，也可以是
/// `.zip`/`.tar.gz`/`.tgz` 压缩包——2026-09 用户需求：技能包大多是压缩包分发，
/// 不该逼用户自己先手动解压）导入到当前工作区的 `.rock_desk/skills/<name>/`。
/// `local_path` 是前端文件选择器选中的路径，始终指本地磁盘（UI 所在机器）；
/// 目标工作区可能是远程 SSH/Agent，因此不能像 Explorer 的"复制"那样用
/// `FileOps::copy`（只支持同一实现内部复制），要用 `copy_between` 跨两种
/// `FileOps` 实现直接读字节写字节（和 `agent_upload_entry` 上传到远程是同一个
/// 思路）。技能名从 `SKILL.md` frontmatter 的 `name` 字段取，缺失时退化为文件夹名，
/// 和 `discover_skills` 解析已有技能时的规则一致。
#[tauri::command]
pub async fn skill_import(
    state: State<'_, AppState>,
    workspace_id: Uuid,
    local_path: String,
) -> Result<SkillMeta, AppError> {
    let handle = state
        .workspaces
        .read()
        .await
        .get(&workspace_id)
        .cloned()
        .ok_or_else(|| AppError::NotFound(format!("workspace not opened: {workspace_id}")))?;

    let raw_path = local_path.trim_end_matches(['/', '\\']).to_string();
    let is_archive = crate::coding::skills::is_archive_path(&raw_path);
    // 压缩包先解压到一个临时目录，剩下的逻辑（读 SKILL.md、拷进工作区）和"直接
    // 选中已解压文件夹"完全一样，只是源路径换成了解压出来的技能根目录；
    // `_temp_guard` 离开作用域时自动删掉这个临时目录（成功/失败都清）。
    let (effective_path, _temp_guard) = if is_archive {
        let extract_dir =
            std::env::temp_dir().join(format!("roc_desk-skill-{}", Uuid::new_v4()));
        let skill_root = crate::coding::skills::extract_skill_archive(
            std::path::Path::new(&raw_path),
            &extract_dir,
        )?;
        (
            skill_root.to_string_lossy().into_owned(),
            Some(TempDirGuard(extract_dir)),
        )
    } else {
        (raw_path.clone(), None)
    };

    let local_ops = crate::fsops::local::LocalFileOps;
    let skill_md_content = local_ops
        .read_file(&format!("{effective_path}/SKILL.md"))
        .await
        .map_err(|_| AppError::Internal(format!("{effective_path} 下没有找到 SKILL.md，不是一个合法的技能目录")))?;
    let (fields, _) = crate::coding::skills::parse_frontmatter(&skill_md_content.text);
    // 压缩包场景下兜底名字取压缩包自己的文件名（去掉扩展名）而不是解压临时目录名——
    // 后者要么是随机 uuid（压缩包根目录直接有 SKILL.md 时），要么是压缩包内部的
    // 文件夹名（嵌套一层时），都不如"用户自己选的这个压缩包叫什么"更贴近意图。
    let folder_name = if is_archive {
        let base = raw_path.rsplit(['/', '\\']).next().unwrap_or(&raw_path);
        base.trim_end_matches(".zip")
            .trim_end_matches(".tar.gz")
            .trim_end_matches(".tgz")
            .to_string()
    } else {
        effective_path
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(&effective_path)
            .to_string()
    };
    let name = fields.get("name").cloned().unwrap_or(folder_name);
    let description = fields.get("description").cloned().unwrap_or_default();

    let root = handle.profile.root_path.trim_end_matches(['/', '\\']);
    let rock_desk_dir = format!("{root}/.rock_desk");
    let skills_root = format!("{rock_desk_dir}/skills");
    let dest = format!("{skills_root}/{name}");
    // 中间目录第一次导入技能时未必存在，远程 `create_dir` 只建单层（见
    // `FileOps::create_dir` 文档），要逐层建；已存在时报错直接忽略。
    let _ = handle.file_ops.create_dir(&rock_desk_dir).await;
    let _ = handle.file_ops.create_dir(&skills_root).await;
    // 同名技能已存在时先删再拷贝，把"导入"当成"导入/更新"——用户改了本地
    // SKILL.md 想重新导入覆盖是常见操作，不该逼着先手动删除旧版本。
    if handle.file_ops.list_dir(&dest).await.is_ok() {
        handle.file_ops.delete(&dest, true).await?;
    }

    let should_cancel = || false;
    let file_count = std::sync::atomic::AtomicU64::new(0);
    crate::fsops::copy_between(
        &local_ops,
        &effective_path,
        handle.file_ops.as_ref(),
        &dest,
        true,
        &None,
        &should_cancel,
        &file_count,
    )
    .await?;

    if let Ok(mut guard) = caches().lock() {
        guard.probes.remove(&workspace_id);
    }
    Ok(SkillMeta {
        name,
        description,
        dir: dest,
    })
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
    let result = guard
        .accept(change_id, &state.ssh_pool, &state.agent_pool, &app_handle)
        .await;
    if result.is_ok() {
        if let Some(turn_id) = guard.changes().iter().find(|c| c.id == change_id).map(|c| c.turn_id) {
            let still_pending = guard
                .changes()
                .iter()
                .any(|c| c.turn_id == turn_id && c.status == ChangeStatus::Pending);
            drop(guard);
            if !still_pending {
                maybe_auto_continue(&state, &app_handle, workspace_id, turn_id);
            }
        }
    }
    result
}

#[tauri::command]
pub async fn coding_reject_change(
    state: State<'_, AppState>,
    app_handle: AppHandle,
    workspace_id: Uuid,
    change_id: Uuid,
) -> Result<(), AppError> {
    let store = get_change_store(&state, workspace_id).await?;
    let mut guard = store.lock().await;
    let result = guard.reject(change_id);
    if result.is_ok() {
        if let Some(turn_id) = guard.changes().iter().find(|c| c.id == change_id).map(|c| c.turn_id) {
            let still_pending = guard
                .changes()
                .iter()
                .any(|c| c.turn_id == turn_id && c.status == ChangeStatus::Pending);
            drop(guard);
            if !still_pending {
                maybe_auto_continue(&state, &app_handle, workspace_id, turn_id);
            }
        }
    }
    result
}

/// Accept/Reject 把某个 `Pending` 改动结清、发现这一轮（`turn_id`）已经没有其它
/// 待处理改动之后调用：后台异步检查 `CodingSession` 是不是正因为这一轮而"卡等
/// 确认"，是的话自动帮用户把对话续上（2026-09 用户反馈：AI 说"确认后我再执行
/// xxx"，用户点了应用却没有任何后续，必须自己再发一条消息才会继续）。
///
/// 整个过程放进 `tokio::spawn`、不 `await` 它——`coding_accept_change`/
/// `coding_reject_change` 必须立刻把写盘结果返回给前端，不能因为这里可能触发
/// 一次耗时的 AI 续跑请求就卡住"应用"按钮本身的响应（继续维持 `ChangeStore` 和
/// `CodingSession` 两把锁互不阻塞的既有设计，见 `ChangeStore` 顶部文档）。
fn maybe_auto_continue(
    state: &State<'_, AppState>,
    app_handle: &AppHandle,
    workspace_id: Uuid,
    turn_id: Uuid,
) {
    let coding_sessions = state.coding_sessions.clone();
    let coding_changes = state.coding_changes.clone();
    let ai_provider_manager = state.ai_provider_manager.clone();
    let ssh_pool = state.ssh_pool.clone();
    let agent_pool = state.agent_pool.clone();
    let audit_log = state.audit_log.clone();
    let command_confirms = state.command_confirms.clone();
    let permission_rules = state.permission_rules.clone();
    let question_confirms = state.question_confirms.clone();
    let mcp_manager = state.mcp_manager.clone();
    let coding_cancel_tokens = state.coding_cancel_tokens.clone();
    let app_handle = app_handle.clone();

    tokio::spawn(async move {
        let Some(session) = coding_sessions.read().await.get(&workspace_id).cloned() else {
            return;
        };
        let mut session_guard = session.lock().await;
        if !session_guard.resolve_awaiting_confirmation(turn_id) {
            // 不是当前正卡着的那一轮（比如这批改动很久之后才被处理，会话早就跑
            // 到更新的一轮甚至已经结束了）——不触发，避免打断/误接一段不相关
            // 的对话。
            return;
        }

        let summary = match coding_changes.read().await.get(&workspace_id).cloned() {
            Some(store) => {
                let guard = store.lock().await;
                summarize_turn_changes(guard.changes().iter().filter(|c| c.turn_id == turn_id))
            }
            None => "改动已处理".to_string(),
        };
        let continuation_text =
            format!("[系统自动继续] 你上一轮提议的文件改动已经全部处理完：{summary}。请据此继续完成任务。");
        let session_id = session_guard.id;

        // 这整个续跑请求完全是后端自己发起的，不是前端调用 `coding_send_message`
        // 触发的——前端的"发送中"状态（输入框禁用/停止按钮）纯靠那个 invoke 调用
        // 前后手动置位，感知不到这里在后台默默跑一轮。用一对独立事件把"开始/
        // 结束"通知过去，前端按和手动发送完全一致的方式处理（追加时间线气泡、
        // 置 sending、存历史），用户不会误以为点了应用之后什么都没发生，也不会
        // 在续跑进行中又手滑发一条容易和它打架的新消息。
        let _ = app_handle.emit(
            "coding:auto-continue-start",
            json!({ "sessionId": session_id, "note": format!("变更已确认（{summary}），AI 正在自动继续任务…") }),
        );

        let cancel_token = tokio_util::sync::CancellationToken::new();
        coding_cancel_tokens
            .lock()
            .unwrap()
            .insert(workspace_id, cancel_token.clone());
        let result = session_guard
            .send_message(
                &continuation_text,
                &[],
                &ai_provider_manager,
                &ssh_pool,
                &agent_pool,
                &audit_log,
                &command_confirms,
                &permission_rules,
                &question_confirms,
                &mcp_manager,
                &app_handle,
                &cancel_token,
            )
            .await;
        coding_cancel_tokens.lock().unwrap().remove(&workspace_id);

        let payload = match &result {
            Ok(reply) => json!({ "sessionId": session_id, "reply": reply, "error": Option::<String>::None }),
            Err(e) => json!({ "sessionId": session_id, "reply": Option::<String>::None, "error": e.to_string() }),
        };
        let _ = app_handle.emit("coding:auto-continue-done", payload);
    });
}

/// 拼一段"这一轮改动都怎么处理了"的人话摘要，喂给 `maybe_auto_continue` 触发的
/// 续跑请求——模型需要知道具体哪些文件被应用、哪些被拒绝了才能正确决定下一步
/// （不是随便一句"继续"就够，拒绝的文件可能意味着要换个方案）。
fn summarize_turn_changes<'a>(changes: impl Iterator<Item = &'a FileChange>) -> String {
    let mut applied = Vec::new();
    let mut rejected = Vec::new();
    for c in changes {
        match c.status {
            ChangeStatus::Applied => applied.push(c.path.as_str()),
            ChangeStatus::Rejected => rejected.push(c.path.as_str()),
            _ => {}
        }
    }
    let mut parts = Vec::new();
    if !applied.is_empty() {
        parts.push(format!("已应用 {} 个（{}）", applied.len(), applied.join("、")));
    }
    if !rejected.is_empty() {
        parts.push(format!("已拒绝 {} 个（{}）", rejected.len(), rejected.join("、")));
    }
    if parts.is_empty() {
        "没有改动被处理".to_string()
    } else {
        parts.join("；")
    }
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
    // history_id` 事后覆盖，会导致审计日志/事件广播用的还是构造时随手生成的
    // 临时 id，和前端实际展示的 session id 对不上，直接用最终 id 构造会话。
    let (mut session, change_store) = build_new_session(
        &state,
        workspace_id,
        detail.summary.provider_id,
        false,
        Some(history_id),
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
