use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU8};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex;
use uuid::Uuid;

use super::changes::ChangeStore;
use super::diff::DiffLine;
use super::guard;
use super::permission::{Decision, PermissionEngine};
use super::skills::{self, SkillMeta};
use super::tools::{self, TodoItem, ToolCall};
use super::webfetch;
use super::{CommandConfirmRegistry, QuestionRegistry};
use crate::agent::AgentConnectionPool;
use crate::ai::{search_web_results, AiProviderManager};
use crate::db::repo::audit_log_repo::AuditLogRepo;
use crate::db::repo::permission_rules_repo::PermissionRulesRepo;
use crate::error::AppError;
use crate::fsops::{search_stream, FileOps, SearchMode, SearchOptions};
use crate::mcp::McpServerManager;
use crate::ssh::SshConnectionPool;
use codex_engine::EngineEventSink;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingMode {
    Plan,
    Build,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum CodingTarget {
    Local,
    Remote {
        connection_id: Uuid,
        host_label: String,
    },
    /// 远程 Windows Agent 工作区（AGENT_DESIGN.md §四.4）：`run_command`/`search_files`
    /// 走 Agent 协议而不是 SSH `exec`/`grep`，命令语法也是 Windows 原生的
    /// （`cmd.exe /C` + 参数数组，不是"拼一行 POSIX shell 字符串"）。
    Agent {
        connection_id: Uuid,
        host_label: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeStatus {
    Pending,
    Applied,
    Rejected,
    Undone,
}

/// 用户随消息一起发的附件（DESIGN.md §3.8 输入优化/附件需求）：图片走 OpenAI
/// 兼容的 `image_url` 多模态 content parts，模型需要支持 vision 才能"看到"；
/// 文本类文件直接把内容拼进消息正文，不需要模型支持多模态就能读。前端负责把
/// 本地文件读成 base64/文本再传过来——后端不碰用户的本地文件系统，天然对齐
/// "远程工作区也能用附件"（附件来自用户本机，不是工作区所在的主机）。
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChatAttachment {
    Image {
        name: String,
        mime: String,
        data_base64: String,
    },
    File {
        name: String,
        content: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileChange {
    pub id: Uuid,
    pub path: String,
    pub old_content: String,
    pub new_content: String,
    pub diff: Vec<DiffLine>,
    pub status: ChangeStatus,
    #[serde(default)]
    pub expected_mtime: Option<i64>,
    /// 产生这条变更的用户消息轮次——同一轮对话里模型可能连续改好几个文件，
    /// 前端据此把它们分成一组，提供"这一轮全部应用/全部拒绝/整体撤销"的批量
    /// 操作（而不是必须一个个点），参考 Cursor/Windsurf 的 checkpoint 交互，但
    /// 不依赖 git（用户明确要求撤销机制不能绑定 git，见 `revert_turn`）。
    pub turn_id: Uuid,
}

/// AI 写盘（Accept/Undo/Redo/撤销整轮）落地后的同步信息——如果这个路径当前正在
/// 编辑器里开着，前端要用它刷新对应的 buffer，否则打开的 Tab 会和磁盘内容脱节
/// （之前的实现完全没有这一环：`accept_change` 写盘时 `expected_mtime` 传
/// `None`，绕开了正常保存路径的 mtime 冲突检测，也就意味着编辑器 buffer 的
/// mtime 完全不知道文件已经被外部改写过）。
#[derive(Debug, Clone, Serialize)]
pub struct FileSyncInfo {
    pub change_id: Uuid,
    pub path: String,
    pub content: String,
    pub mtime: i64,
}

/// AI 编程助手会话（DESIGN.md §3.8.3）：自动绑定某个已打开的工作区，一个进程内
/// 每个工作区最多一个活跃会话（CODE_DESIGN.md 里没有多会话并发的需求）。
///
/// 文件改动走"生成 Diff 立即可见 + 落盘延后到用户 Accept"的流程（对应
/// FileChangeCard.tsx 的 pending/applied/rejected 三态和"用户可逐条 Accept/Reject"
/// 的文案），而不是像骨架代码那样立即写盘再靠 Undo 补救——AI 编程助手的核心场景是
/// 触碰生产服务器，"改完再后悔"的代价比"多点一次确认"高得多。为了不让同一轮对话
/// 里后续的 read_file 看到"过期"的内容，`pending_content_for` 会优先返回未落盘的
/// 提议内容，让模型的推理和已经生成的 Diff 保持一致。
pub struct CodingSession {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub workspace_root: String,
    pub target: CodingTarget,
    pub mode: CodingMode,
    pub provider_id: Uuid,
    pub auto_allow_readonly: bool,
    /// 供 Codex exec-server 回调读取的共享镜像。Codex 回调不能锁回当前
    /// `CodingSession`，否则会和正在等待引擎事件的 `send_message` 死锁。
    pub(crate) codex_auto_allow_readonly: Arc<AtomicBool>,
    /// Codex exec-server 的模式镜像：0=Plan，1=Build。
    pub(crate) codex_mode: Arc<AtomicU8>,
    /// 文件改动（Diff/Accept/Undo/Redo）的独立状态容器，故意不是 `CodingSession`
    /// 的直接字段、而是一个单独加锁的 `Arc<Mutex<ChangeStore>>`——原因见
    /// `ChangeStore` 的文档注释：不能让"应用/拒绝某个文件改动"卡在等一个可能跑
    /// 一两分钟的 AI 对话轮次释放锁。`AppState.coding_changes` 持有同一个 Arc 的
    /// 另一份克隆，供 `commands::coding` 里的 accept/reject/undo 系列命令直接用，
    /// 不经过这个结构体、也就不需要拿 `CodingSession` 外层的锁。
    pub change_store: Arc<Mutex<ChangeStore>>,
    /// `todo_write` 工具维护的任务清单（对齐 OpenCode 语义：每次调用整体替换，
    /// 不是增量 patch），前端在对话区上方渲染成一条常驻清单。
    pub todos: Vec<TodoItem>,
    /// 当前工作区 `.rock_desk/skills/*/SKILL.md` 发现到的技能列表——只在
    /// `coding_start` 时扫描一次（和 `git_repo` 探测同样的时机/理由），不会
    /// 在会话生命周期内自动感知新增的技能目录。
    pub skills: Vec<SkillMeta>,
    /// 本次会话实际读到并注入系统提示词的项目记忆文件名（`AGENTS.md`/`CLAUDE.md`
    /// 中存在的那些），前端用来在工具栏渲染"已加载 XXX"徽标。
    pub project_memory_loaded: Vec<String>,
    messages: Vec<serde_json::Value>,
    /// `pub(crate)`：`coding::codex_exec_target` 的 `ExecTarget` 适配器读文件时
    /// 要复用同一个 `FileOps`（三态执行的本地/SSH/Agent 差异已经封在这里面），
    /// 不重新构造一份。
    pub(crate) file_ops: Arc<dyn FileOps>,
    /// 当前正在处理的用户消息轮次 id——`send_message` 一开始就生成一个新的，
    /// 这一轮里 `stage_change` 产生的所有 `FileChange` 都打上同一个 `turn_id`，
    /// 供前端做"这一轮"的批量操作。
    current_turn_id: Uuid,
    /// 只有 provider 命中 `routes_to_codex_engine`（目前限定 OpenAI 官方/Azure/
    /// Bedrock/Ollama 且目标是 `CodingTarget::Local`，见
    /// docs/CODEX_INTEGRATION_PLAN.md）且初始化成功时才是 `Some`——由
    /// `commands::coding::build_new_session` 在构造完会话后异步 attach 上去，
    /// `CodingSession::new` 本身保持同步、不做任何 codex-core 相关的初始化。
    codex_engine: Option<CodexEngineBundle>,
}

/// 深度嵌入 codex-core 的对话引擎 + 它专属的 exec-server（见
/// docs/CODEX_INTEGRATION_PLAN.md Phase 3）。`_exec_server` 只是为了在
/// `CodingSession` 存活期间保住这个值不被提前 drop——它的后台 WebSocket 监听
/// 任务已经在 `RocDeskExecServer::bind` 里用 `tokio::spawn` 起来了，drop 这个
/// 句柄本身不会停止那个任务，只是不这样保留会让代码读起来像"用完就可以扔"，
/// 容易误导后来的改动。
///
/// `engine` 是 `CodexCoreEngineHandle`（不是裸的 `CodexCoreEngine`）——运行时
/// 实测发现 codex-core 深层异步调用链（`ThreadManager::start_thread`/
/// `CodexThread::next_event`）在默认线程栈大小下会直接
/// `STATUS_STACK_OVERFLOW`（Windows 上是进程级致命错误，不是能捕获的
/// panic）。`CodexCoreEngineHandle` 把真正的调用挪到了一个专用的大栈线程上，
/// 这里通过 channel 通信，`CodingSession`/Tauri command 的正常异步上下文完全
/// 不用关心栈大小问题。
struct CodexEngineBundle {
    engine: codex_engine::CodexCoreEngineHandle,
    _exec_server: codex_engine::RocDeskExecServer,
    /// 当前轮次 id，和 `CodingSession::current_turn_id` 保持同步（`send_message`
    /// 每次开始新一轮时一起更新），供 `codex_exec_target::SessionExecTarget`
    /// 在收到 codex 的 `fs/writeFile` 回调时读取，不需要它反过来访问
    /// `CodingSession`（见 `codex_exec_target.rs` 顶部注释：那样会死锁）。
    turn_id: Arc<std::sync::Mutex<Uuid>>,
}

/// 把 `codex_engine::EngineEventSink` 的回调转成 roc_desk 现有的 Tauri 事件——
/// 事件名/形状故意和自研引擎（`send_message` 里散落的 `app_handle.emit` 调用）
/// 保持一致，前端 `codingStore.ts` 不需要为了 codex 路由改任何监听逻辑。
///
/// 拥有（而不是借用）`AppHandle`——`CodexCoreEngineHandle::run_turn` 要求
/// `Arc<dyn EngineEventSink>`（跨线程传给专用的大栈引擎线程），裸生命周期引用
/// 过不了 `'static` 约束；`AppHandle` 本身就是一个可以随便 `clone()` 的轻量句柄，
/// 拥有一份不是问题。
struct TauriEventSink {
    session_id: Uuid,
    app_handle: AppHandle,
}

impl EngineEventSink for TauriEventSink {
    fn on_assistant_delta(&self, text: &str) {
        let _ = self.app_handle.emit(
            "coding:assistant-note",
            json!({ "sessionId": self.session_id, "text": text, "kind": "model" }),
        );
    }

    fn on_reasoning_delta(&self, text: &str) {
        let _ = self.app_handle.emit(
            "coding:assistant-note",
            json!({ "sessionId": self.session_id, "text": text, "kind": "status" }),
        );
    }

    fn on_tool_progress(&self, label: &str) {
        let _ = self.app_handle.emit(
            "coding:tool-call-start",
            json!({ "sessionId": self.session_id, "tool": "codex", "detail": label }),
        );
    }

    fn on_tool_progress_end(&self) {
        let _ = self.app_handle.emit(
            "coding:tool-call-end",
            json!({ "sessionId": self.session_id, "tool": "codex" }),
        );
    }
}

// 2026-08-18 用户真实反馈：让编程助手"分析本项目源代码，对代码进行评审"这类
// 开放式大任务，8 轮工具调用就把预算用完了，被当成"疑似死循环"直接中止——这不是
// 真死循环，是这个仓库本身有几十个源文件，认真读一遍再给评审意见，工具调用次数
// 本来就会比"改一个已知文件的一行 bug"这种收敛型任务多得多。调到 30，给探索型任务
// 更合理的空间；即使还是用完了，`self.messages` 里的进度不会丢（下一条用户消息
// 会接着当前上下文继续），所以调大上限只是"减少不必要的中断"，不是移除保护本身。
const MAX_TOOL_ITERATIONS: usize = 30;
/// 最后这么多轮强制不再提供工具，逼模型收尾给结论（见 send_message 里的用法和注释）。
const FORCE_CONCLUDE_LAST_N: usize = 5;

/// 单条工具结果塞进 `self.messages` 前的字符数上限——`run_command`（截 4000 字符）
/// 和 `webfetch`（截 8000 字符）从一开始就有这层保护，`read_file`/`list_directory`
/// 之前完全没有：`self.messages` 不会随对话推进而裁剪，每一轮都整份重新序列化进
/// 请求体发给模型（见 `send_message` 里 `body["messages"] = self.messages`），
/// 读到一个几百 MB 的日志/压缩包/生成产物、或者一个几万个文件的目录，会在一次
/// 最多 30 轮的"分析整个项目"任务里被原样重复重发几十次。2026-09 用户报告"AI 程序
/// 运行着运行着进程自动退出了"，根因就是这个——Rust 默认分配器在内存分配失败时
/// 直接 `abort` 整个进程，不会走 panic hook，所以连一条崩溃日志都留不下，表现就是
/// 整个窗口悄无声息地消失。2 万字符足以覆盖绝大多数源文件的关键部分，
/// 真需要看更多内容时模型应该用 `search_files`/`glob` 定位更精确的范围，
/// 而不是指望一次 `read_file` 吃下整个大文件。
const MAX_TOOL_RESULT_CHARS: usize = 20_000;
/// 整个会话发给模型的上下文上限。每轮请求都会把它完整 JSON 序列化一次，因此只
/// 限制单条工具结果仍不够：多轮工具调用、历史恢复和图片附件都会让总量持续累积。
const MAX_CONTEXT_CHARS: usize = 1_000_000;
/// 在总量没有触顶时，仍只保留最近这些用户轮次，避免长时间会话缓慢挤占内存。
const MAX_CONTEXT_USER_TURNS: usize = 8;

fn cap_tool_result(text: String) -> String {
    if text.chars().count() <= MAX_TOOL_RESULT_CHARS {
        return text;
    }
    let truncated: String = text.chars().take(MAX_TOOL_RESULT_CHARS).collect();
    format!(
        "{truncated}\n\n[内容过长，已截断到前 {MAX_TOOL_RESULT_CHARS} 字符——如果需要看后续部分，\
         用 search_files/glob 定位更精确的范围，而不是整份读取]"
    )
}

impl CodingSession {
    /// 把上下文限制在一个可预期的内存与请求体大小内。删历史时以"用户消息"为
    /// 边界，因此一轮 assistant tool_calls 与紧随其后的 tool 结果始终一起保留，
    /// 不会留下 OpenAI 兼容接口无法接受的孤立 tool message；最前面的 system 提示
    /// 和项目约定永远不删。
    fn limit_context(&mut self) {
        let mut user_turns = self
            .messages
            .iter()
            .filter(|message| message["role"].as_str() == Some("user"))
            .count();
        loop {
            let size = self
                .messages
                .iter()
                .map(|message| message.to_string().len())
                .sum::<usize>();
            if size <= MAX_CONTEXT_CHARS && user_turns <= MAX_CONTEXT_USER_TURNS {
                break;
            }
            let Some(start) = self
                .messages
                .iter()
                .position(|message| message["role"].as_str() == Some("user"))
            else {
                break;
            };
            let Some(end) =
                self.messages
                    .iter()
                    .enumerate()
                    .skip(start + 1)
                    .find_map(|(index, message)| {
                        (message["role"].as_str() == Some("user")).then_some(index)
                    })
            else {
                break;
            };
            self.messages.drain(start..end);
            user_turns -= 1;
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: Uuid,
        workspace_id: Uuid,
        workspace_root: String,
        target: CodingTarget,
        provider_id: Uuid,
        file_ops: Arc<dyn FileOps>,
        change_store: Arc<Mutex<ChangeStore>>,
    ) -> Self {
        let target_desc = match &target {
            CodingTarget::Local => "本地工作区".to_string(),
            CodingTarget::Remote { host_label, .. } => format!("远程主机 {host_label}"),
            CodingTarget::Agent { host_label, .. } => format!("远程 Windows 主机 {host_label}"),
        };
        // 2026-08-18 真实复现：分析整个项目/做代码评审这类开放式大任务，模型会没完
        // 没了地交替 search_files/read_file，一直不给结论。后端有 FORCE_CONCLUDE_LAST_N
        // 硬兜底（见 send_message），但那是最后一道防线；这里在提示词里先明确给出
        // "工具调用总量有限、该收敛就收敛"的预期，减少真的撞到硬限制的次数。
        let system_prompt = format!(
            "你是集成在 roc_desk 桌面工具里的 AI 编程助手，当前绑定的工作区根目录是 `{workspace_root}`（{target_desc}）。\
             你可以用提供的工具读写文件、搜索代码、访问互联网、执行命令。涉及“今天/最新/新闻/外部事实”的问题必须先调用 web_search，\
             不要凭模型记忆回答。write_file/edit_file 产生的改动不会立即生效，\
             而是生成 Diff 交给用户确认，所以你可以放心连续提出多个改动，不需要等待每一步都被确认才能继续推理。\
             run_command 有安全限制：破坏性命令会被直接拦截，其余命令需要用户在弹窗里确认才会真正执行。\
             工具调用总次数是有限的（几十次量级），不是无限预算：面对\"分析整个项目\"这类开放式大任务时，\
             优先用 search_files/list_directory 快速定位最相关的一小批文件（不需要每个文件都读一遍），\
             读完这些就给出结论；不要为了追求\"看得更全\"而无休止地继续搜索/读取，觉得信息已经够回答用户的\
             问题时就直接总结，而不是再多看几个文件。",
        );
        Self {
            id,
            workspace_id,
            workspace_root,
            target,
            mode: CodingMode::Plan,
            provider_id,
            auto_allow_readonly: false,
            codex_auto_allow_readonly: Arc::new(AtomicBool::new(false)),
            codex_mode: Arc::new(AtomicU8::new(0)),
            change_store,
            todos: Vec::new(),
            skills: Vec::new(),
            project_memory_loaded: Vec::new(),
            messages: vec![json!({ "role": "system", "content": system_prompt })],
            file_ops,
            current_turn_id: Uuid::new_v4(),
            codex_engine: None,
        }
    }

    /// `commands::coding::build_new_session` 在判断这个会话的 provider 命中
    /// codex 路由后调用——`CodingSession::new` 本身保持同步，codex-core 的初始化
    /// （起 `RocDeskExecServer`/构造 `ThreadManager`）都是 async，只能在构造完
    /// 会话之后另外接上。
    pub(crate) fn attach_codex_engine(
        &mut self,
        engine: codex_engine::CodexCoreEngineHandle,
        exec_server: codex_engine::RocDeskExecServer,
        turn_id: Arc<std::sync::Mutex<Uuid>>,
    ) {
        self.codex_engine = Some(CodexEngineBundle {
            engine,
            _exec_server: exec_server,
            turn_id,
        });
    }

    /// 卸载当前挂着的 `CodexCoreEngine`（如果有）——`coding_set_provider` 切换
    /// provider 时用：旧引擎是绑定着切换前那个 provider 的 api_key/model/
    /// base_url 构造出来的，`provider_id` 字段本身只有自研引擎每轮请求时才会
    /// 重新读取，codex 路径不调用这个方法的话会继续悄悄用旧 provider 发请求。
    /// 调用方随后应该按新 provider 重新 `attach_codex_engine`；如果新 provider
    /// 挂不上（协议不兼容/没配 API Key），保持 `None` 就能干净地落回自研引擎，
    /// 不会残留一个绑定着错误 provider 的引擎。
    pub(crate) fn detach_codex_engine(&mut self) {
        self.codex_engine = None;
    }

    /// 读取工作区根目录下的 `AGENTS.md`/`CLAUDE.md`（两个都找就都注入，各自标注
    /// 来源，谁在前谁优先级更高不做主观判断，交给模型自己权衡），拼成一条 system
    /// 消息追加到初始系统提示词之后。cap 到 32KB——项目约定文件正常不会写这么长，
    /// 真写了这么长说明多半是用户不小心把别的内容也塞了进去，截断比整份塞给模型
    /// 更安全（避免一次性吃掉大量上下文预算）。找不到文件不是错误，静默跳过。
    /// 把 `fetch_project_memory` 抓到的内容灌回系统提示词——不做任何 I/O，纯同步，
    /// 可以放心在 `tokio::join!` 之后调用。拆成 fetch/apply 两半（而不是像之前
    /// 那样一个方法从头到尾都拿 `&mut self`）是因为 `build_new_session` 里项目
    /// 记忆/技能发现/git 仓库探测这三个远程探测本来是顺序 await 的，SSH/SFTP
    /// 单次握手的超时上限都是分钟级（见 `ssh/session.rs` 的 `EXEC_TIMEOUT`），
    /// 顺序跑最坏情况下要等三倍时间——远程工作区"点开始/新建会话/切历史都要等
    /// 很久"的真实反馈里，这是三个探测各自都在超时边界附近的复合效应。改成
    /// `tokio::join!` 并发跑，最坏情况的等待时间从"三者之和"降到"三者中最慢
    /// 的那个"，但并发要求这三个操作不能同时抢同一个 `&mut self`，所以拆成
    /// "先并发抓数据（只需要 `&FileOps`，不摸 `self`）、再依次同步应用"两步。
    pub(crate) fn apply_project_memory(&mut self, memory: Vec<(String, String)>) {
        for (filename, content) in memory {
            self.messages.push(json!({
                "role": "system",
                "content": format!("以下是项目 {filename} 中记录的约定，请在完成任务时遵守：\n\n{content}")
            }));
            self.project_memory_loaded.push(filename);
        }
    }

    /// 和 `apply_project_memory` 同理，把 `skills::discover_skills` 的结果灌回
    /// 技能列表 + 系统提示词。
    pub(crate) fn apply_skills(&mut self, skills: Vec<SkillMeta>) {
        self.skills = skills;
        if self.skills.is_empty() {
            return;
        }
        let list = self
            .skills
            .iter()
            .map(|s| format!("- {}: {}", s.name, s.description))
            .collect::<Vec<_>>()
            .join("\n");
        self.messages.push(json!({
            "role": "system",
            "content": format!("当前工作区定义了以下技能，需要时用 skill 工具按名称加载正文：\n{list}")
        }));
    }

    /// 发给 AI 的真实对话上下文快照——`coding_history_save` 落库时用，让"打开
    /// 历史会话"（`coding_history_resume`）能在原有上下文基础上真正继续对话，
    /// 而不只是回放一份文件改动记录（用户 2026-09 反馈）。
    pub fn messages_snapshot(&self) -> Vec<serde_json::Value> {
        self.messages.clone()
    }

    /// `coding_history_resume` 用持久化的对话上下文整体替换当前（刚构造、只有
    /// 系统提示词的）`messages`，恢复到历史会话中断时的状态。
    pub fn restore_messages(&mut self, messages: Vec<serde_json::Value>) {
        if !messages.is_empty() {
            self.messages = messages;
            self.limit_context();
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn send_message(
        &mut self,
        user_text: &str,
        attachments: &[ChatAttachment],
        providers: &AiProviderManager,
        ssh_pool: &SshConnectionPool,
        agent_pool: &AgentConnectionPool,
        audit: &AuditLogRepo,
        confirms: &CommandConfirmRegistry,
        permission_rules: &PermissionRulesRepo,
        question_confirms: &QuestionRegistry,
        mcp_manager: &McpServerManager,
        app_handle: &AppHandle,
        cancel_token: &tokio_util::sync::CancellationToken,
    ) -> Result<String, AppError> {
        self.current_turn_id = Uuid::new_v4();

        // codex 路由分支：绕开下面整套自研 HTTP+工具循环，交给
        // `CodexCoreEngine` 驱动。见 docs/CODEX_INTEGRATION_PLAN.md Phase 3——
        // 目前只在 `attach_codex_engine` 成功挂上时才会走这条分支（provider 命中
        // OpenAI 系 + `CodingTarget::Local`），其余情况原样走下面已经跑了很久的
        // 自研引擎，行为不变。
        if self.codex_engine.is_some() {
            let inputs = codex_user_inputs(user_text, attachments, self.mode);
            self.messages.push(
                json!({ "role": "user", "content": user_message_content(user_text, attachments) }),
            );
            self.limit_context();
            let sink: Arc<dyn EngineEventSink> = Arc::new(TauriEventSink {
                session_id: self.id,
                app_handle: app_handle.clone(),
            });
            let bundle = self.codex_engine.as_mut().expect("checked above");
            *bundle.turn_id.lock().unwrap() = self.current_turn_id;
            // "停止"按钮：`run_turn` 内部是 `oneshot::Receiver.await`（见
            // `codex-engine/src/engine.rs` 的 `CodexCoreEngineHandle::run_turn`），
            // 取消胜出时直接丢弃这个 future 是安全的——常驻的引擎线程稍后
            // `reply.send(result)` 会因为接收端已经没人听而静默失败（`let _ =`），
            // 不会 panic。同时调用 `interrupt_current_turn` 给 codex-core 自己的
            // session 循环发 `Op::Interrupt`，让它正常收敛掉这一轮（而不是被
            // roc_desk 这边强行抛弃，服务端状态却还在半途），下一轮对话开始时
            // 不会因为上一轮"卡在一半"而混乱。
            let text = tokio::select! {
                biased;
                result = bundle.engine.run_turn(inputs, sink) => {
                    result.map_err(|e| AppError::Internal(format!("codex engine: {e}")))?
                }
                _ = cancel_token.cancelled() => {
                    // `Op::Interrupt` 本身也是走 codex-core session 内部那条提交
                    // 队列的——如果 session 的处理循环正卡在别的地方（比如我们
                    // 自己的 exec-server 在等一个永远不会被回应的命令确认弹窗），
                    // 这次提交也可能跟着卡住。这里的初衷是"尽量让 codex-core 干净
                    // 收敛"，不能让它反过来把整个 `send_message`（进而是
                    // `coding_sessions` 那把锁）也搭进去卡死，所以加一个超时兜底：
                    // 超时就放弃干净中断，直接把控制权还给用户，好过用户连"停止"
                    // 本身都点不动。
                    let _ = tokio::time::timeout(
                        std::time::Duration::from_secs(5),
                        bundle.engine.interrupt_current_turn(),
                    )
                    .await;
                    return Err(AppError::Internal("已停止：用户取消了当前对话轮次".to_string()));
                }
            };
            self.messages
                .push(json!({ "role": "assistant", "content": &text }));
            return Ok(text);
        }

        self.messages.push(
            json!({ "role": "user", "content": user_message_content(user_text, attachments) }),
        );
        self.limit_context();
        // 规则改动（比如刚点了"允许并记住"）只需要下一条消息生效，不需要额外的
        // 缓存失效通知——这里每条用户消息都重新从数据库现取一份规则快照。
        let permission_engine = PermissionEngine::load(permission_rules)?;
        // Build 模式下把已启用 MCP 服务器的工具合并进模型可调用列表；单个服务器
        // 连接失败（子进程起不来/HTTP 握手失败）不影响其它服务器和内置工具，
        // 静默跳过——真正需要诊断的话，用户可以在 MCP 服务器管理里手动测试。
        // `mcp_tool_index` 把"暴露给模型的限定名"直接映射回 `(server_id, 原始工具名)`，
        // 调用时不需要反向解析 `mcp__<server>__<tool>` 这个拼接格式。
        let mut mcp_tool_defs: Vec<serde_json::Value> = Vec::new();
        let mut mcp_tool_index: HashMap<String, (Uuid, String)> = HashMap::new();
        if self.mode == CodingMode::Build {
            if let Ok(servers) = mcp_manager.list_enabled() {
                for server in servers {
                    let Ok(client) = mcp_manager.get_or_connect(server.id).await else {
                        continue;
                    };
                    for tool in &client.tools {
                        let qualified_name = format!(
                            "mcp__{}__{}",
                            sanitize_tool_name(&server.name),
                            sanitize_tool_name(&tool.name)
                        );
                        mcp_tool_defs.push(json!({
                            "type": "function",
                            "function": {
                                "name": qualified_name,
                                "description": format!("[MCP:{}] {}", server.name, tool.description),
                                "parameters": tool.input_schema,
                            }
                        }));
                        mcp_tool_index.insert(qualified_name, (server.id, tool.name.clone()));
                    }
                }
            }
        }

        // 不依赖模型是否主动输出说明：请求一开始就给 UI 一个即时、可读的状态。
        // 这是任务进度，不是模型的隐藏思维链。
        let _ = app_handle.emit(
            "coding:assistant-note",
            json!({
                "sessionId": self.id,
                "text": "我先理解任务并定位相关代码，接下来的检查和执行步骤会实时显示在这里。",
                "kind": "status"
            }),
        );

        let provider = providers.get(self.provider_id)?.ok_or_else(|| {
            AppError::NotFound(format!("ai provider not found: {}", self.provider_id))
        })?;
        let api_key = providers.resolve_api_key(&provider).await?;
        let client = reqwest::Client::new();

        for i in 0..MAX_TOOL_ITERATIONS {
            if cancel_token.is_cancelled() {
                return Err(AppError::Internal(
                    "已停止：用户取消了当前对话轮次".to_string(),
                ));
            }
            self.limit_context();
            let url = format!(
                "{}/chat/completions",
                provider.api_base.trim_end_matches('/')
            );

            // 2026-08-18 真实复现：让模型做"分析整个项目并评审"这类开放式大任务时，
            // 它会没完没了地交替 search_files/read_file，一直不给结论，直到把
            // MAX_TOOL_ITERATIONS 用完只剩一个报错——中间收集的所有信息全部浪费。
            // 到了最后几轮强制不再提供工具（不发 `tools` 字段，模型物理上叫不了任何
            // 工具），逼它基于已经读到的内容直接给结论，总比"报错、什么都没有"强。
            let force_conclude_start = MAX_TOOL_ITERATIONS.saturating_sub(FORCE_CONCLUDE_LAST_N);
            let force_conclude = i >= force_conclude_start;
            if i == force_conclude_start {
                self.messages.push(json!({
                    "role": "system",
                    "content": "你已经调用了很多次工具，收集到的信息应该已经足够。接下来不再提供任何工具，\
                                 请直接基于目前已经了解到的内容给出结论/总结，不要说\"我需要再看看\"这类话。"
                }));
            }

            let mut body = json!({ "model": provider.model, "messages": self.messages });
            if !force_conclude {
                let mut tools = tools_for_mode(self.mode);
                if let Some(arr) = tools.as_array_mut() {
                    arr.extend(mcp_tool_defs.iter().cloned());
                }
                body["tools"] = tools;
                body["tool_choice"] = json!("auto");
            }
            let mut req = client.post(&url).json(&body);
            if let Some(key) = &api_key {
                req = req.bearer_auth(key);
            }
            // "停止"按钮：单次请求本身就是这个循环里最容易卡很久的一步（网络慢/
            // 服务端限流排队），所以在这一步单独包一层取消——`req.send()` 败给
            // 取消信号时直接丢弃即可，`reqwest` 的请求 future 被 drop 时会中止
            // 底层连接，不会有资源泄漏。
            let resp = tokio::select! {
                biased;
                result = req.send() => result?,
                _ = cancel_token.cancelled() => {
                    return Err(AppError::Internal(
                        "已停止：用户取消了当前对话轮次".to_string(),
                    ));
                }
            };
            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(AppError::Connection(format!("HTTP {status}: {body}")));
            }
            let body: serde_json::Value = resp.json().await?;
            let message = body["choices"][0]["message"].clone();
            let tool_calls = message["tool_calls"]
                .as_array()
                .cloned()
                .unwrap_or_default();

            if tool_calls.is_empty() {
                let text = message["content"].as_str().unwrap_or("").to_string();
                self.messages
                    .push(json!({ "role": "assistant", "content": text }));
                return Ok(text);
            }

            // 模型在决定调工具的同时，很多时候会顺带写一句"我要看看 xxx 文件"之类的
            // 简短说明（`content` 和 `tool_calls` 在同一条 assistant 消息里同时出现，
            // 不是互斥的），之前这段文本被直接丢弃——只有工具调用本身作为一个匿名的
            // "tool: xxx"行短暂闪一下，模型到底在想什么完全不可见，这正是用户反馈的
            // "编程助手的思考过程没有展示出来"。这里把它当一条普通的 assistant 消息
            // 广播出去，前端渲染成时间线里的一条正常发言，不是等最终答案出来才一次性
            // 展示。
            if let Some(note) = message["content"].as_str() {
                if !note.trim().is_empty() {
                    let _ = app_handle.emit(
                        "coding:assistant-note",
                        json!({ "sessionId": self.id, "text": note, "kind": "model" }),
                    );
                }
            }

            // 很多兼容 OpenAI 的模型在工具调用轮只返回 tool_calls，没有 content。
            // 此时补一条基于公开工具参数生成的进度说明，避免界面长时间沉默。
            if message["content"]
                .as_str()
                .is_none_or(|text| text.trim().is_empty())
            {
                if let Some(call) = tool_calls.first() {
                    let fn_name = call["function"]["name"].as_str().unwrap_or_default();
                    let fn_args = call["function"]["arguments"].as_str().unwrap_or("{}");
                    let text = tool_progress_text(fn_name, fn_args, tool_calls.len());
                    let _ = app_handle.emit(
                        "coding:assistant-note",
                        json!({ "sessionId": self.id, "text": text, "kind": "status" }),
                    );
                }
            }

            self.messages.push(message);
            for call in &tool_calls {
                let call_id = call["id"].as_str().unwrap_or_default().to_string();
                let fn_name = call["function"]["name"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                let fn_args = call["function"]["arguments"].as_str().unwrap_or("{}");
                // 只显示工具名之前完全看不出模型在反复对同一个文件/同一个词调用，还是
                // 在正常地一个一个探索不同文件——这次真实复现的"看起来在循环"就是靠
                // 加上这个才能一眼确认（见 REQUIREMENTS.md §3.7 的记录）。
                let detail = tool_call_detail(&fn_name, fn_args);

                let _ = app_handle.emit(
                    "coding:tool-call-start",
                    json!({ "sessionId": self.id, "tool": fn_name, "detail": detail }),
                );

                let call_result = if let Some((server_id, tool_name)) = mcp_tool_index.get(&fn_name)
                {
                    let arguments: serde_json::Value =
                        serde_json::from_str(fn_args).unwrap_or_else(|_| json!({}));
                    Ok(ToolCall::Mcp {
                        server_id: *server_id,
                        tool_name: tool_name.clone(),
                        arguments,
                    })
                } else {
                    tools::parse_tool_call(&fn_name, fn_args)
                };
                let result_text = match call_result {
                    Ok(call) => self
                        .execute_tool(
                            call,
                            ssh_pool,
                            agent_pool,
                            audit,
                            confirms,
                            &permission_engine,
                            question_confirms,
                            mcp_manager,
                            app_handle,
                        )
                        .await
                        .unwrap_or_else(|e| format!("工具执行出错：{e}")),
                    Err(e) => format!("工具调用参数解析失败：{e}"),
                };

                let _ = app_handle.emit(
                    "coding:tool-call-end",
                    json!({ "sessionId": self.id, "tool": fn_name }),
                );

                self.messages.push(json!({
                    "role": "tool",
                    "tool_call_id": call_id,
                    "content": result_text,
                }));
            }
        }

        Err(AppError::Internal(format!(
            "这一轮已经调用了 {MAX_TOOL_ITERATIONS} 次工具还没给出最终结论，先停下来避免无限跑下去。\
             之前的进度都还在（对话上下文没丢），直接发\"继续\"就会接着刚才的内容往下做，不需要重新描述任务。"
        )))
    }

    #[allow(clippy::too_many_arguments)]
    async fn execute_tool(
        &mut self,
        call: ToolCall,
        ssh_pool: &SshConnectionPool,
        agent_pool: &AgentConnectionPool,
        audit: &AuditLogRepo,
        confirms: &CommandConfirmRegistry,
        permission_engine: &PermissionEngine,
        question_confirms: &QuestionRegistry,
        mcp_manager: &McpServerManager,
        app_handle: &AppHandle,
    ) -> Result<String, AppError> {
        match call {
            ToolCall::ReadFile { path } => {
                if let Some(content) = self.change_store.lock().await.pending_content_for(&path) {
                    return Ok(cap_tool_result(content));
                }
                // `read_file_for_editor`（不是不设上限的 `read_file`）：超过
                // `EDITOR_PREVIEW_MAX_BYTES` 只读前面一部分字节，不会像默认实现那样
                // 把一个几百 MB 的日志/压缩包/生成产物整份吸进内存——见下面
                // `cap_tool_result` 的注释，这是同一个内存失控问题的另一半修复
                // （字节层面 vs. 字符层面）。
                let content = self.file_ops.read_file_for_editor(&path).await?;
                Ok(cap_tool_result(content.text))
            }
            ToolCall::ListDirectory { path } => {
                let entries = self.file_ops.list_dir(&path).await?;
                Ok(cap_tool_result(
                    serde_json::to_string(&entries).unwrap_or_default(),
                ))
            }
            ToolCall::SearchFiles { pattern, path } => {
                self.search_files(&pattern, &path, ssh_pool, agent_pool)
                    .await
            }
            ToolCall::WebSearch { query } => {
                search_web_results(&reqwest::Client::new(), &query).await
            }
            ToolCall::WriteFile { path, content } => {
                self.stage_change(&path, content, ssh_pool, agent_pool, app_handle)
                    .await
            }
            ToolCall::EditFile {
                path,
                old_text,
                new_text,
            } => {
                let original = match self.change_store.lock().await.pending_content_for(&path) {
                    Some(c) => c,
                    None => self.file_ops.read_file(&path).await?.text,
                };
                if !original.contains(&old_text) {
                    return Err(AppError::Internal(format!(
                        "edit_file 失败：在 {path} 中没有找到匹配的 old_text，请先用 read_file 确认现有内容"
                    )));
                }
                let updated = original.replacen(&old_text, &new_text, 1);
                self.stage_change(&path, updated, ssh_pool, agent_pool, app_handle)
                    .await
            }
            ToolCall::RunCommand { command } => {
                self.run_command_gated(
                    &command,
                    ssh_pool,
                    agent_pool,
                    audit,
                    confirms,
                    permission_engine,
                    app_handle,
                )
                .await
            }
            ToolCall::Glob { pattern, path } => self.search_glob(&pattern, &path).await,
            ToolCall::WebFetch { url } => {
                self.webfetch_gated(&url, permission_engine, app_handle)
                    .await
            }
            ToolCall::TodoWrite { todos } => {
                self.todos = todos;
                let _ = app_handle.emit(
                    "coding:todo-update",
                    json!({ "sessionId": self.id, "todos": &self.todos }),
                );
                Ok(format!("任务清单已更新，共 {} 项", self.todos.len()))
            }
            ToolCall::Question { question, options } => {
                self.ask_user(&question, options, question_confirms, app_handle)
                    .await
            }
            ToolCall::Skill { name } => {
                skills::load_skill_body(self.file_ops.as_ref(), &self.skills, &name).await
            }
            ToolCall::Mcp {
                server_id,
                tool_name,
                arguments,
            } => {
                let full_auto = self
                    .change_store
                    .lock()
                    .await
                    .full_auto
                    .load(std::sync::atomic::Ordering::Relaxed);
                self.call_mcp_tool(
                    full_auto,
                    server_id,
                    &tool_name,
                    arguments,
                    permission_engine,
                    confirms,
                    mcp_manager,
                    app_handle,
                )
                .await
            }
        }
    }

    /// `glob` 工具：只比对文件名/路径，不读内容——直接复用 Explorer 全局搜索
    /// 用的 `fsops::search_stream`（`SearchMode::FileName`），本地/远程工作区
    /// 天然统一，不需要像旧的 `search_files_local` 那样另写一套只支持本地的
    /// 递归遍历。不需要流式进度（工具调用是"一次要一个完整结果"，不是给用户
    /// 实时看的搜索框），所以 `on_file` 直接收集进 `Vec`，`should_cancel` 恒为
    /// `false`。
    async fn search_glob(&self, pattern: &str, path: &str) -> Result<String, AppError> {
        let mut matches = Vec::new();
        let options = SearchOptions {
            case_sensitive: false,
            whole_word: false,
            use_regex: false,
        };
        search_stream(
            self.file_ops.as_ref(),
            path,
            pattern,
            &options,
            SearchMode::FileName,
            |result| matches.push(result.path),
            || false,
        )
        .await?;
        Ok(if matches.is_empty() {
            "没有匹配结果".to_string()
        } else {
            matches.join("\n")
        })
    }

    /// `webfetch` 工具：默认策略是"允许"（和已经不设防的 `web_search` 一致，
    /// SSRF 防护在 `coding/webfetch.rs::fetch_url` 里做），但用户可以在权限规则
    /// 里为 `webfetch` 加一条 `deny` 规则（比如公司网络策略不希望 AI 访问外部
    /// 网页），命中即拒绝，不会真的发出请求。
    async fn webfetch_gated(
        &self,
        url: &str,
        permission_engine: &PermissionEngine,
        _app_handle: &AppHandle,
    ) -> Result<String, AppError> {
        if permission_engine.decide("webfetch", url) == Some(Decision::Deny) {
            return Ok(format!("已按权限规则拒绝访问：{url}"));
        }
        webfetch::fetch_url(&reqwest::Client::new(), url).await
    }

    /// `question` 工具：和 `run_command` 的确认弹窗是同一个"阻塞等待前端响应"
    /// 模式，只是等的是一段文本回答而不是 bool。
    async fn ask_user(
        &self,
        question: &str,
        options: Vec<String>,
        question_confirms: &QuestionRegistry,
        app_handle: &AppHandle,
    ) -> Result<String, AppError> {
        let (request_id, rx) = question_confirms.register().await;
        let _ = app_handle.emit(
            "coding:question-request",
            json!({ "sessionId": self.id, "requestId": request_id, "question": question, "options": options }),
        );
        let answer = rx.await.unwrap_or_default();
        Ok(answer)
    }

    /// MCP 工具调用门禁：先看用户有没有为这个 `mcp:<server>:<tool>` 维度配过规则，
    /// 没有就落回"始终确认"——和 `run_command` 未命中白名单时的默认策略一致，
    /// 复用同一个 `CommandConfirmRegistry`（语义都是"允许执行一次带副作用的动作"，
    /// 弹窗文案由前端按 `kind` 字段区分，不需要后端再建一套新的确认流程）。
    #[allow(clippy::too_many_arguments)]
    async fn call_mcp_tool(
        &self,
        full_auto: bool,
        server_id: Uuid,
        tool_name: &str,
        arguments: serde_json::Value,
        permission_engine: &PermissionEngine,
        confirms: &CommandConfirmRegistry,
        mcp_manager: &McpServerManager,
        app_handle: &AppHandle,
    ) -> Result<String, AppError> {
        let servers = mcp_manager.list()?;
        let server = servers
            .iter()
            .find(|s| s.id == server_id)
            .ok_or_else(|| AppError::NotFound("MCP 服务器不存在".into()))?;
        // MCP 维度的规则用固定的 `tool = "mcp"` + 可通配的 `pattern`（比如
        // `filesystem:*` 放行某个服务器的全部工具、`filesystem:read_file` 只放行
        // 单个工具），和 `run_command` 的 `tool = "run_command"` + 通配命令文本是
        // 同一种设计，不是"每个工具单独存一行精确匹配"。
        let match_key = format!("{}:{}", server.name, tool_name);

        let allowed = match permission_engine.decide("mcp", &match_key) {
            Some(Decision::Allow) => true,
            Some(Decision::Deny) => false,
            _ if full_auto => true,
            _ => {
                let (request_id, rx) = confirms.register(self.id).await;
                let _ = app_handle.emit(
                    "coding:command-confirm-request",
                    json!({
                        "sessionId": self.id,
                        "requestId": request_id,
                        "command": format!("{}.{}({})", server.name, tool_name, arguments),
                        "host": null,
                        "kind": "mcp",
                        "matchKey": match_key,
                    }),
                );
                rx.await.unwrap_or(false)
            }
        };
        if !allowed {
            return Ok(format!(
                "用户拒绝执行 MCP 工具调用：{}.{}",
                server.name, tool_name
            ));
        }

        let client = mcp_manager.get_or_connect(server_id).await?;
        client.call_tool(tool_name, arguments).await
    }

    async fn search_files(
        &self,
        pattern: &str,
        path: &str,
        ssh_pool: &SshConnectionPool,
        agent_pool: &AgentConnectionPool,
    ) -> Result<String, AppError> {
        match &self.target {
            CodingTarget::Local => {
                let results = tools::search_files_local(std::path::Path::new(path), pattern, 50);
                Ok(if results.is_empty() {
                    "没有匹配结果".to_string()
                } else {
                    results.join("\n")
                })
            }
            CodingTarget::Remote { connection_id, .. } => {
                let session = ssh_pool.get_or_connect(*connection_id).await?;
                let quoted_pattern = crate::log::remote::shell_quote(pattern);
                let quoted_path = crate::log::remote::shell_quote(path);
                let cmd = format!(
                    "rg -n -F -- {quoted_pattern} {quoted_path} 2>/dev/null || grep -rn -F -- {quoted_pattern} {quoted_path} 2>/dev/null"
                );
                session.exec(&cmd).await
            }
            // Agent 侧的搜索在远程主机本机跑（`agent/src/handlers/search.rs`，用
            // std::fs 遍历，不是逐文件网络往返）——这正是 AGENT_DESIGN.md §一表格
            // 里"远程内容搜索慢"这条设计收益的落地位置。
            CodingTarget::Agent { connection_id, .. } => {
                let session = agent_pool.get_or_connect(*connection_id).await?;
                let options = roc_desk_protocol::SearchOptions {
                    case_sensitive: false,
                    whole_word: false,
                    use_regex: false,
                };
                let request = roc_desk_protocol::Request::SearchContent {
                    root: path.to_string(),
                    query: pattern.to_string(),
                    options,
                };
                match session.request(request).await? {
                    roc_desk_protocol::Response::Ok(
                        roc_desk_protocol::ResponseBody::SearchResults(results),
                    ) => {
                        if results.is_empty() {
                            return Ok("没有匹配结果".to_string());
                        }
                        let mut lines = Vec::new();
                        for file in results {
                            for m in file.matches {
                                lines.push(format!(
                                    "{}:{}:{}",
                                    file.path,
                                    m.line_number,
                                    m.line_text.trim()
                                ));
                            }
                        }
                        Ok(lines.join("\n"))
                    }
                    roc_desk_protocol::Response::Error { message, .. } => {
                        Err(AppError::Internal(message))
                    }
                    _ => Err(AppError::Internal("Agent 返回了意外的响应类型".into())),
                }
            }
        }
    }

    /// `pub(crate)`（而不是纯私有）：`codex::exec_target` 里对接 codex-core
    /// exec-server 协议的 `ExecTarget` 适配器需要复用这同一套 Pending 状态机
    /// 语义（见 docs/CODEX_INTEGRATION_PLAN.md §3.2），不重新实现一遍。
    pub(crate) async fn stage_change(
        &mut self,
        path: &str,
        new_content: String,
        ssh_pool: &SshConnectionPool,
        agent_pool: &AgentConnectionPool,
        app_handle: &AppHandle,
    ) -> Result<String, AppError> {
        let turn_id = self.current_turn_id;
        let (change, sync) = self
            .change_store
            .lock()
            .await
            .stage(path, new_content, turn_id, ssh_pool, agent_pool, app_handle)
            .await?;
        let id = change.id;
        let applied = sync.is_some();
        // `sync` 非空表示"完全授权模式"已经把这个改动直接落盘了——一并广播出去，
        // 前端据此刷新这个路径对应的、可能已经打开的编辑器 buffer（否则磁盘内容
        // 变了，编辑器里显示的还是旧内容）。
        let _ = app_handle.emit(
            "coding:file-change",
            json!({ "sessionId": self.id, "change": &change, "sync": sync }),
        );
        Ok(if applied {
            format!("已为 {path} 生成变更（id={id}）。完全授权模式已开启，已直接写入磁盘。")
        } else {
            format!("已为 {path} 生成变更（id={id}），已在界面展示 Diff，等待用户 Accept 后才会真正写入磁盘。")
        })
    }

    /// `pub(crate)`：理由同 `stage_change`，codex-core exec-server 适配器
    /// （`coding::codex_exec_target`）要复用同一套权限规则引擎/确认弹窗/三态
    /// 执行路由，不重新实现一遍——实际逻辑在自由函数 `run_command_gated_shared`
    /// 里（见下方），这里只是把 `&self` 上的几个字段拆出来传过去。拆成自由函数
    /// 是因为 `codex_exec_target::SessionExecTarget` 不能持有
    /// `Arc<Mutex<CodingSession>>` 来调用这个方法：codex-core 的 exec-server
    /// 回调发生在 `send_message` 正在 `&mut self` 持有这个会话、且还在等
    /// `thread.next_event()` 的时候，再去抢同一把锁会死锁。
    pub(crate) async fn run_command_gated(
        &mut self,
        command: &str,
        ssh_pool: &SshConnectionPool,
        agent_pool: &AgentConnectionPool,
        audit: &AuditLogRepo,
        confirms: &CommandConfirmRegistry,
        permission_engine: &PermissionEngine,
        app_handle: &AppHandle,
    ) -> Result<String, AppError> {
        let full_auto = self
            .change_store
            .lock()
            .await
            .full_auto
            .load(std::sync::atomic::Ordering::Relaxed);
        run_command_gated_shared(
            self.id,
            &self.target,
            &self.workspace_root,
            self.auto_allow_readonly,
            full_auto,
            command,
            ssh_pool,
            agent_pool,
            audit,
            confirms,
            permission_engine,
            app_handle,
        )
        .await
    }
}

pub(crate) struct CommandExecutionResult {
    pub output: String,
    pub exit_code: Option<i32>,
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_command_gated_shared(
    session_id: Uuid,
    target: &CodingTarget,
    workspace_root: &str,
    auto_allow_readonly: bool,
    full_auto: bool,
    command: &str,
    ssh_pool: &SshConnectionPool,
    agent_pool: &AgentConnectionPool,
    audit: &AuditLogRepo,
    confirms: &CommandConfirmRegistry,
    permission_engine: &PermissionEngine,
    app_handle: &AppHandle,
) -> Result<String, AppError> {
    Ok(run_command_gated_shared_with_status(
        session_id,
        target,
        workspace_root,
        auto_allow_readonly,
        full_auto,
        command,
        ssh_pool,
        agent_pool,
        audit,
        confirms,
        permission_engine,
        app_handle,
    )
    .await?
    .output)
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_command_gated_shared_with_status(
    session_id: Uuid,
    target: &CodingTarget,
    workspace_root: &str,
    auto_allow_readonly: bool,
    full_auto: bool,
    command: &str,
    ssh_pool: &SshConnectionPool,
    agent_pool: &AgentConnectionPool,
    audit: &AuditLogRepo,
    confirms: &CommandConfirmRegistry,
    permission_engine: &PermissionEngine,
    app_handle: &AppHandle,
) -> Result<CommandExecutionResult, AppError> {
    run_command_gated_shared_with_status_in_context(
        session_id,
        target,
        workspace_root,
        auto_allow_readonly,
        full_auto,
        workspace_root,
        &HashMap::new(),
        command,
        ssh_pool,
        agent_pool,
        audit,
        confirms,
        permission_engine,
        app_handle,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_command_gated_shared_with_status_in_context(
    session_id: Uuid,
    target: &CodingTarget,
    _workspace_root: &str,
    auto_allow_readonly: bool,
    full_auto: bool,
    cwd: &str,
    env: &HashMap<String, String>,
    command: &str,
    ssh_pool: &SshConnectionPool,
    agent_pool: &AgentConnectionPool,
    audit: &AuditLogRepo,
    confirms: &CommandConfirmRegistry,
    permission_engine: &PermissionEngine,
    app_handle: &AppHandle,
) -> Result<CommandExecutionResult, AppError> {
    let target_label = target_label_for(target);
    let is_windows_target = matches!(target, CodingTarget::Agent { .. });
    // 2026-09 临时诊断日志：远程 SSH 目标下 codex 路由的 run_command 疑似卡死，
    // 定位不清楚到底卡在黑名单检查/权限规则/等待确认弹窗/还是实际执行这几步
    // 里的哪一步——先打点，等下一次复现时直接从日志读出卡在哪。确认问题后
    // 应该删掉这几行，不是长期保留的诊断设施。
    tracing::info!(
        session_id = %session_id,
        target = %target_label,
        full_auto,
        auto_allow_readonly,
        command,
        "run_command_gated: 进入函数"
    );

    if guard::is_blacklisted(command, is_windows_target) {
        audit.record(session_id, &target_label, command, "blocked", None);
        let _ = app_handle.emit(
            "coding:command-blocked",
            json!({ "sessionId": session_id, "command": command }),
        );
        return Ok(CommandExecutionResult {
            output: format!("已拦截高危命令：{command}，如需执行请前往终端模块手动操作"),
            exit_code: Some(126),
        });
    }

    // 权限规则引擎是叠加在黑名单之上、白名单之外的一层（REQUIREMENTS.md §3.7）：
    // 命中 `deny` 直接拒绝；命中 `allow` 直接放行（跳过确认弹窗，但仍然写审计
    // 日志，`outcome` 标成 `auto-allow-rule` 以区分"用户点了一次仅本次允许"）；
    // 命中 `ask` 或者压根没有匹配规则，都落回原有的白名单/确认弹窗逻辑，不改变
    // 现有行为。
    match permission_engine.decide("run_command", command) {
        Some(Decision::Deny) => {
            audit.record(session_id, &target_label, command, "rejected", None);
            return Ok(CommandExecutionResult {
                output: format!("已按权限规则拒绝执行：{command}"),
                exit_code: Some(126),
            });
        }
        Some(Decision::Allow) => {
            tracing::info!(session_id = %session_id, command, "run_command_gated: 命中权限规则 Allow，即将实际执行");
            audit.record(session_id, &target_label, command, "auto-allow-rule", None);
            let output =
                run_target_command(target, cwd, env, command, ssh_pool, agent_pool).await?;
            tracing::info!(session_id = %session_id, command, "run_command_gated: Allow 分支实际执行完成");
            let summary: String = output.output.chars().take(2000).collect();
            audit.record(
                session_id,
                &target_label,
                command,
                "executed",
                Some(&summary),
            );
            return Ok(CommandExecutionResult {
                output: output.output.chars().take(4000).collect(),
                exit_code: output.exit_code,
            });
        }
        _ => {}
    }

    tracing::info!(session_id = %session_id, "run_command_gated: 通过黑名单/权限规则检查");
    let allowed = if full_auto || (auto_allow_readonly && guard::is_whitelisted(command)) {
        tracing::info!(session_id = %session_id, command, "run_command_gated: 命中 full_auto/白名单，跳过确认");
        true
    } else {
        tracing::info!(session_id = %session_id, "run_command_gated: 即将调用 confirms.register");
        let (request_id, rx) = confirms.register(session_id).await;
        tracing::info!(session_id = %session_id, request_id = %request_id, "run_command_gated: confirms.register 已返回");
        let is_remote = matches!(
            target,
            CodingTarget::Remote { .. } | CodingTarget::Agent { .. }
        );
        tracing::info!(
            session_id = %session_id,
            request_id = %request_id,
            command,
            "run_command_gated: 已注册确认请求，即将 emit 弹窗事件"
        );
        let _ = app_handle.emit(
            "coding:command-confirm-request",
            json!({
                "sessionId": session_id,
                "requestId": request_id,
                "command": command,
                "host": if is_remote { Some(target_label.clone()) } else { None },
                "kind": "command",
            }),
        );
        let result = rx.await.unwrap_or(false);
        tracing::info!(
            session_id = %session_id,
            request_id = %request_id,
            allowed = result,
            "run_command_gated: 收到确认结果"
        );
        result
    };

    if !allowed {
        audit.record(session_id, &target_label, command, "rejected", None);
        return Ok(CommandExecutionResult {
            output: format!("用户拒绝执行命令：{command}"),
            exit_code: Some(126),
        });
    }

    tracing::info!(session_id = %session_id, command, "run_command_gated: 已放行，开始实际执行");
    let output = run_target_command(target, cwd, env, command, ssh_pool, agent_pool).await?;
    tracing::info!(session_id = %session_id, command, "run_command_gated: 实际执行完成");
    let summary: String = output.output.chars().take(2000).collect();
    audit.record(
        session_id,
        &target_label,
        command,
        "executed",
        Some(&summary),
    );
    Ok(CommandExecutionResult {
        output: output.output.chars().take(4000).collect(),
        exit_code: output.exit_code,
    })
}

async fn run_target_command(
    target: &CodingTarget,
    cwd: &str,
    env: &HashMap<String, String>,
    command: &str,
    ssh_pool: &SshConnectionPool,
    agent_pool: &AgentConnectionPool,
) -> Result<CommandExecutionResult, AppError> {
    match target {
        CodingTarget::Local => {
            let output = run_local_command_output_with_env(command, cwd, env).await?;
            Ok(CommandExecutionResult {
                output: String::from_utf8_lossy(&[output.stdout, output.stderr].concat())
                    .to_string(),
                exit_code: output.status.code(),
            })
        }
        CodingTarget::Remote { connection_id, .. } => {
            let session = ssh_pool.get_or_connect(*connection_id).await?;
            // `exec()` 出错（尤其是 `EXEC_TIMEOUT` 超时）之后主动清掉这条缓存
            // 连接——`is_alive()` 检测不出网络层面的静默失联，不清掉的话下一次
            // `get_or_connect` 还会把同一条死连接交出去，每次都要重新等满
            // 120 秒超时（见 `SshConnectionPool::evict` 的文档注释）。
            let output = match session.exec(command).await {
                Ok(output) => output,
                Err(e) => {
                    ssh_pool.evict(*connection_id).await;
                    return Err(e);
                }
            };
            Ok(CommandExecutionResult {
                output,
                exit_code: Some(0),
            })
        }
        CodingTarget::Agent { connection_id, .. } => {
            let session = agent_pool.get_or_connect(*connection_id).await?;
            Ok(CommandExecutionResult {
                output: session.exec(command, cwd).await?,
                exit_code: Some(0),
            })
        }
    }
}

pub(crate) fn target_label_for(target: &CodingTarget) -> String {
    match target {
        CodingTarget::Local => "本地".to_string(),
        CodingTarget::Remote { host_label, .. } => host_label.clone(),
        CodingTarget::Agent { host_label, .. } => host_label.clone(),
    }
}

/// 只做 I/O、不摸 `CodingSession`——配合 `CodingSession::apply_project_memory`
/// 让 `build_new_session` 能用 `tokio::join!` 把这个探测和 git 仓库探测/技能
/// 发现并发跑，见 `apply_project_memory` 的文档。
pub(crate) async fn fetch_project_memory(file_ops: &dyn FileOps) -> Vec<(String, String)> {
    const MAX_BYTES: usize = 32 * 1024;
    let mut result = Vec::new();
    for filename in ["AGENTS.md", "CLAUDE.md"] {
        let Ok(content) = file_ops.read_file(filename).await else {
            continue;
        };
        let truncated: String = content.text.chars().take(MAX_BYTES).collect();
        result.push((filename.to_string(), truncated));
    }
    result
}

/// 从工具调用参数里挑一个最能说明"这次到底在操作什么"的字段，给前端时间线展示
/// （见上面调用处的注释）。解析失败或者没有对应字段就返回 None，不强求覆盖所有
/// 工具——展示不出细节比展示一个 "undefined" 更诚实。
fn tool_call_detail(fn_name: &str, fn_args: &str) -> Option<String> {
    let args: serde_json::Value = serde_json::from_str(fn_args).ok()?;
    match fn_name {
        "read_file" | "write_file" | "edit_file" | "list_directory" => {
            args["path"].as_str().map(str::to_string)
        }
        "web_search" => args["query"].as_str().map(str::to_string),
        "search_files" => {
            let pattern = args["pattern"].as_str().unwrap_or("?");
            let path = args["path"].as_str().unwrap_or("?");
            Some(format!("{pattern} in {path}"))
        }
        "run_command" => args["command"]
            .as_str()
            .map(|s| s.chars().take(80).collect()),
        _ => None,
    }
}

fn tool_progress_text(fn_name: &str, fn_args: &str, call_count: usize) -> String {
    let detail = tool_call_detail(fn_name, fn_args);
    let target = detail.as_deref().unwrap_or("相关内容");
    let action = match fn_name {
        "read_file" => format!("读取 `{target}`，确认当前实现"),
        "list_directory" => format!("查看 `{target}` 的目录结构"),
        "search_files" => format!("搜索 `{target}`，定位相关代码"),
        "web_search" => format!("访问互联网搜索 `{target}`"),
        "write_file" => format!("为 `{target}` 准备新文件变更"),
        "edit_file" => format!("为 `{target}` 准备代码修改"),
        "run_command" => format!("运行 `{target}`，检查实际结果"),
        _ => format!("执行 {fn_name}，继续处理任务"),
    };
    if call_count > 1 {
        format!("我准备并行执行 {call_count} 项检查，先{action}。")
    } else {
        format!("我正在{action}。")
    }
}

fn tools_for_mode(mode: CodingMode) -> serde_json::Value {
    let all = tools::tool_schema();
    match mode {
        CodingMode::Build => all,
        CodingMode::Plan => {
            // glob/todo_write/skill/question 是只读或纯 UI 交互（不碰文件系统/网络/
            // 命令执行），Plan 模式下同样开放，和 read_file 等分析类工具同一档。
            // webfetch 会发出真实网络请求、MCP 工具无法证明是只读的，两者继续
            // 只在 Build 模式暴露（MCP 工具本身是动态拼进 `tools` 数组的，不在这个
            // 静态白名单里，天然只在 `send_message` 判断 `mode == Build` 时才会出现）。
            const READ_ONLY: &[&str] = &[
                "read_file",
                "list_directory",
                "search_files",
                "web_search",
                "glob",
                "todo_write",
                "skill",
                "question",
            ];
            serde_json::Value::Array(
                all.as_array()
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|t| READ_ONLY.contains(&t["function"]["name"].as_str().unwrap_or("")))
                    .collect(),
            )
        }
    }
}

/// 把用户输入和附件拼成一条 user 消息的 `content`：没有附件时保持纯字符串
/// （和改动前完全一致，不打扰不用附件的既有场景/历史数据格式）；有附件时才
/// 切成 OpenAI 兼容的多模态 parts 数组——文本类附件直接拼进文字正文（模型不需要
/// 支持 vision 也能读），图片作为独立的 `image_url` part（需要模型支持 vision
/// 才"看得到"，不支持的模型会按各家实现忽略或报错，这里不做能力探测，交给用户
/// 自己判断当前 Provider 是否支持）。
fn codex_user_inputs(
    user_text: &str,
    attachments: &[ChatAttachment],
    mode: CodingMode,
) -> Vec<codex_engine::EngineInput> {
    let mode_instruction = match mode {
        CodingMode::Plan => {
            "\n\n当前为 Plan 模式：只分析和提出方案，不得创建目录、修改文件或执行命令。"
        }
        CodingMode::Build => "\n\n当前为 Build 模式：可按工具权限执行实现任务。",
    };
    let mut text = format!("{user_text}{mode_instruction}");
    for attachment in attachments {
        if let ChatAttachment::File { name, content } = attachment {
            text.push_str(&format!("\n\n--- 附件文件: {name} ---\n{content}"));
        }
    }
    let mut inputs = vec![codex_engine::EngineInput::Text(text)];
    for attachment in attachments {
        if let ChatAttachment::Image {
            mime, data_base64, ..
        } = attachment
        {
            inputs.push(codex_engine::EngineInput::ImageDataUrl(format!(
                "data:{mime};base64,{data_base64}"
            )));
        }
    }
    inputs
}

fn user_message_content(user_text: &str, attachments: &[ChatAttachment]) -> serde_json::Value {
    if attachments.is_empty() {
        return json!(user_text);
    }
    let mut text = user_text.to_string();
    for attachment in attachments {
        if let ChatAttachment::File { name, content } = attachment {
            text.push_str(&format!("\n\n--- 附件文件: {name} ---\n{content}"));
        }
    }
    let mut parts = vec![json!({ "type": "text", "text": text })];
    for attachment in attachments {
        if let ChatAttachment::Image {
            mime, data_base64, ..
        } = attachment
        {
            parts.push(json!({
                "type": "image_url",
                "image_url": { "url": format!("data:{mime};base64,{data_base64}") }
            }));
        }
    }
    serde_json::Value::Array(parts)
}

/// OpenAI function-calling 的工具名要求匹配 `^[a-zA-Z0-9_-]+$`，MCP 服务器/工具名
/// 是用户自己起的、可能带中文或空格——拼进 `mcp__<server>__<tool>` 之前先替换掉
/// 非法字符，避免一整条请求因为工具名不合法被 Provider 直接拒绝。
fn sanitize_tool_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

pub(super) async fn run_local_command(command: &str, cwd: &str) -> Result<String, AppError> {
    let output = run_local_command_output(command, cwd).await?;
    Ok(String::from_utf8_lossy(&output.stdout).to_string()
        + &String::from_utf8_lossy(&output.stderr))
}

pub(super) async fn run_local_command_output(
    command: &str,
    cwd: &str,
) -> Result<std::process::Output, AppError> {
    run_local_command_output_with_env(command, cwd, &HashMap::new()).await
}

/// 没有控制台的 GUI 进程（roc_desk.exe）起 `cmd.exe`/`powershell.exe` 这类
/// 控制台子进程，Windows 默认会一闪而过弹一个可见的控制台窗口——`rdp/mod.rs`/
/// `fsops/office_convert.rs` 起子进程时已经用这个标志位避免过，这里之前漏了。
/// 2026-09 用户反馈：本地目标下 AI 工具跑的命令会额外弹一个窗口执行，体感上
/// 像是"跑到 AI 工具外面去了"，其实就是这个控制台窗口闪现。
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub(super) async fn run_local_command_output_with_env(
    command: &str,
    cwd: &str,
    env: &HashMap<String, String>,
) -> Result<std::process::Output, AppError> {
    #[cfg(target_os = "windows")]
    let mut cmd = {
        // 用 `raw_arg` 而不是 `arg`——`command` 这个字符串（对 codex 路由来说，
        // 是 `codex_exec_target.rs::run_command` 已经用 `cmd_quote` 手动按
        // cmd.exe 语法转义、拼好的完整命令行）如果走普通的 `.arg()`，Rust 会
        // 把它当成"一个不透明的参数值"再转义一遍——它自己已经加好的引号会被
        // 当成需要保护的特殊字符处理，两层转义叠在一起，引号数量直接对不上。
        // 2026-09 用户实测复现：模型报 PowerShell "字符数缺少终止符" 解析错误，
        // 根因就是这次双重转义，不是编码问题（两个不同模型都被这条报错误导
        // 去查 GBK/UTF-8 编码，两个都猜错了方向）。`raw_arg` 原样把字符串接到
        // 命令行末尾，不做任何额外转义/加引号，`command` 已经是一条完整、
        // 转义好的命令行，正需要这种"照抄不动"的语义。
        let mut c = tokio::process::Command::new("cmd");
        c.raw_arg("/C").raw_arg(command);
        c.creation_flags(CREATE_NO_WINDOW);
        c
    };
    #[cfg(not(target_os = "windows"))]
    let mut cmd = {
        let mut c = tokio::process::Command::new("sh");
        c.arg("-c").arg(command);
        c
    };
    cmd.current_dir(cwd);
    cmd.envs(env);
    // `Command::output()` 只把 stdout/stderr 接成管道用来采集输出，stdin 默认
    // 原样继承父进程——roc_desk 是没有控制台的 GUI 进程，继承来的 stdin 句柄
    // 要么无效要么是个"读了就永远拿不到数据"的东西。子进程/命令解析链路里
    // 只要有任何一环尝试从 stdin 读一下（哪怕只是意外触发，比如命令行被
    // cmd.exe 解析成了带交互提示的形式），就会永远卡在那次读取上——`top`/
    // `ps`/`echo hi` 这类完全不需要输入的命令也一样卡死，正是这个特征。
    // 显式把 stdin 钉成 null，子进程读 stdin 直接拿到 EOF，不会再有这条路
    // 能把它卡住。
    cmd.stdin(std::process::Stdio::null());
    Ok(cmd.output().await?)
}
