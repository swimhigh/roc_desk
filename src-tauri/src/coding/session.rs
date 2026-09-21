use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::{AppHandle, Emitter};
use tokio::io::AsyncBufReadExt;
use tokio::sync::{Mutex, RwLock};
use uuid::Uuid;
use sha2::{Digest, Sha256};

use super::changes::ChangeStore;
use super::diff::DiffLine;
use super::git_ops;
use super::guard;
use super::permission::{Decision, PermissionEngine};
use super::skills::{self, SkillMeta};
use super::tools::{self, TodoItem, ToolCall};
use super::webfetch;
use super::{CommandConfirmRegistry, QuestionRegistry};
use crate::agent::AgentConnectionPool;
use crate::agent_llm;
use crate::ai::{search_web_results, AiProvider, AiProviderManager};
use crate::db::repo::audit_log_repo::AuditLogRepo;
use crate::db::repo::ai_evidence_repo::{AiEvidenceRepo, EvidenceEntry, MAX_EVIDENCE_BYTES};
use crate::db::repo::permission_rules_repo::PermissionRulesRepo;
use crate::error::AppError;
use crate::fsops::{search_stream, FileOps, SearchMode, SearchOptions};
use crate::mcp::McpServerManager;
use crate::ssh::SshConnectionPool;
use crate::symbols::{build_index, SymbolIndex};

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
    /// PDF——前端不解析内容，只把原始文件读成 base64 传过来（和 `Image` 一样
    /// "后端不碰用户本机文件系统"），真正的文本抽取在 `build_user_message_content`
    /// 里用 `pdf_extract` 做。2026-09 用户反馈：之前非图片附件一律当纯文本读
    /// （`FileReader.readAsText`），PDF 是二进制格式，读出来是乱码，模型完全
    /// 看不懂——需要专门的分支。
    Pdf {
        name: String,
        data_base64: String,
    },
}

/// AI 正在处理上一条消息时用户又发了一条——不等当前这一轮工具循环彻底结束，攒
/// 在 `AppState.coding_pending_injections`（按 workspace_id 一份，独立于
/// `CodingSession` 外层那把锁）里，`send_message` 的工具循环每轮迭代开头会读
/// 一次这张表、把攒到的都当作普通用户消息追加进 `messages`，供下一次模型请求
/// 看到。不能做成 `CodingSession` 自己的字段——`send_message` 从进入到返回一直
/// 独占持有会话外层的锁，塞在会话内部的字段在这轮结束前根本抢不到锁去写。
#[derive(Debug, Clone)]
pub struct PendingInjection {
    pub text: String,
    pub attachments: Vec<ChatAttachment>,
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
    pub(crate) evidence_repo: Arc<AiEvidenceRepo>,
    evidence_recall_query: Option<String>,
messages: Vec<serde_json::Value>,
    pub(crate) file_ops: Arc<dyn FileOps>,
    /// 当前正在处理的用户消息轮次 id——`send_message` 一开始就生成一个新的，
    /// 这一轮里 `stage_change` 产生的所有 `FileChange` 都打上同一个 `turn_id`，
    /// 供前端做"这一轮"的批量操作。
    current_turn_id: Uuid,
    /// `send_message` 收尾（模型不再调用工具、给出最终回复）时，如果这一轮还有
    /// `Pending` 状态的文件改动没被用户处理，就记下这一轮的 `turn_id`；
    /// `commands::coding` 的 accept/reject 在"这一轮改动已经全部处理完"时会检查
    /// 这个字段，命中就自动帮用户把对话续上（2026-09 用户反馈：AI 说"确认后我
    /// 再执行 xxx"，用户点了应用却没有任何后续，必须自己再发一条消息才会继续）。
    /// `None` 表示当前没有"卡在等确认"的轮次。
    awaiting_confirmation_turn: Option<Uuid>,
    /// `run_command_background` 启动的后台进程，key 是工具返回给模型的 job_id——
    /// 只在本地目标下可用。放在会话自己的字段里而不是像 `change_store` 那样单独
    /// 开一把 `AppState` 级别的锁，是因为只有 `execute_tool`（已经持有 `&mut
    /// self`）会读写它，不存在"要在 `send_message` 整轮持锁期间被别的命令并发
    /// 访问"的需求。会话被丢弃（关闭工作区/开新会话）时 `Drop` 实现会把还没
    /// 手动 `stop_background_process` 的进程一并杀掉，不会有开发服务器之类的
    /// 子进程在用户已经离开这个会话后还悄悄留在后台占端口。
    background_jobs: HashMap<Uuid, BackgroundJob>,
}

impl Drop for CodingSession {
    fn drop(&mut self) {
        for (_, mut job) in self.background_jobs.drain() {
            let _ = job.child.start_kill();
        }
    }
}

/// `run_command_background` 的一个在跑（或已结束但还没被 `stop_background_process`
/// 清理）的进程。`output` 是两个后台读取任务（stdout/stderr 各一个）持续追加的
/// 累计输出，用 `Arc<StdMutex<..>>` 而不是 `tokio::sync::Mutex`——追加操作本身
/// 不跨 `.await`，标准库的锁足够，不需要 tokio 版本的开销。
struct BackgroundJob {
    command: String,
    child: tokio::process::Child,
    output: Arc<StdMutex<String>>,
}

/// 单个后台进程累计输出的字符数上限——开发服务器/watch 进程可能一直不退出、
/// 一直往 stdout 写东西，不设上限的话这个 `String` 会无限增长。超过上限
/// （`2x` 之后才截断，不是每写一行就截断一次）就只保留最近这一段，配合
/// `is_char_boundary` 避免在多字节字符中间切断。
const MAX_BACKGROUND_OUTPUT_CHARS: usize = 20_000;

fn append_background_output(buf: &StdMutex<String>, text: &str) {
    let mut s = buf.lock().unwrap();
    s.push_str(text);
    if s.len() > MAX_BACKGROUND_OUTPUT_CHARS * 2 {
        let keep_from = s.len().saturating_sub(MAX_BACKGROUND_OUTPUT_CHARS);
        let mut idx = keep_from;
        while idx < s.len() && !s.is_char_boundary(idx) {
            idx += 1;
        }
        *s = s[idx..].to_string();
    }
}

// 2026-08-18 用户真实反馈：让编程助手"分析本项目源代码，对代码进行评审"这类开放式
// 大任务，几十轮工具调用的硬上限会把预算用完，被当成"疑似死循环"直接中止——这不是
// 真死循环，是这个仓库本身有几十个源文件，认真读一遍再给评审意见，工具调用次数本来
// 就会比"改一个已知文件的一行 bug"这种收敛型任务多得多。最初把上限从 8 调到 30，
// 后来又加了"最后 5 轮强制断供 tools 逼模型收尾"的兜底，但这个强制收尾的系统提示会
// 永久留在 `self.messages` 里（从不撤回）——2026-09 用户真实复现：这类大任务撞到
// 硬上限后，模型不仅这一轮被断供，后续新的一轮（工具预算其实已经重置、`tools` 字段
// 也正常发了）依然被历史里那条"接下来不再提供任何工具"的系统消息带偏，误以为整个
// 会话都不能再用工具，反过来告诉用户"当前会话已无法调用文件读写和执行工具"——这是
// 一句误导性的话，用户由此误以为是权限/配额问题去查权限设置，白费功夫。用户明确要求
// 干脆不做这层限制：工具循环不再有硬编码的轮次上限，只由"模型自己决定不再调用工具、
// 给出最终答案"或者用户主动点"停止"（`cancel_token`）来结束。代价是真遇到模型陷入
// 死循环、一直调工具不收敛的场景，不会再有自动兜底掐断——只能用户手动点停止；
// `self.messages` 不会丢失进度这一点不受影响。

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
/// 而不是指望一次 `read_file` 吃下整个大文件。这个上限本身（连同 `cap_tool_result`）
/// 已经挪到 `crate::agent_llm`，跟 `sql::agent` 共用一份实现。
/// 整个会话发给模型的上下文上限，按 token 估算（见 `estimate_tokens`）而不是原始字符数——
/// 2026-09 复盘：原来直接拿字符数当预算单位，对不同模型的真实上下文窗口没有代表性
/// （尤其中文场景：一个汉字在 UTF-8 里是 3 字节，用字符数算预算会系统性低估实际 token
/// 消耗）。真实复现过一次"HTTP 400 context_too_large"——用户接的 provider 实际能接受
/// 的窗口明显比这里假设的更小，60_000 是收紧后的保守默认值。
///
/// **只是没配置 `AiProvider::context_window_tokens` 时的兜底值**——2026-09 第二次
/// 复盘：把所有 Provider 都按这个保守默认值压缩，大窗口 Provider（比如实际支持
/// 128K/200K+ 上下文的模型）被同一套小阈值频繁触发压缩，一次 87 万 token 的探索性
/// 任务里被压缩了几十次，探索出来的文件路径/搜索结果几乎全被摘要抹掉，用户紧接着
/// 追问同一个任务时模型对着几句摘要没法定位到具体文件，只能整个重新探索一遍。现在
/// 优先用 Provider 设置里用户手填的真实窗口大小（`effective_context_budget`），没填
/// 才退回这个全局保守值。
const DEFAULT_CONTEXT_TOKENS_ESTIMATE: usize = 60_000;
/// 用户手填的 `context_window_tokens` 只留 80% 当预算，不是 100%——上下文里除了
/// 历史消息，还要给 system 提示词、工具 schema 定义、以及这一轮的响应本身留出
/// 空间，全部吃满真实窗口大小反而更容易触发 Provider 的硬性拒绝。
const CONTEXT_BUDGET_HEADROOM_NUM: usize = 8;
const CONTEXT_BUDGET_HEADROOM_DEN: usize = 10;

fn effective_context_budget(provider: &AiProvider) -> usize {
    match provider.context_window_tokens {
        Some(window) => (window as usize * CONTEXT_BUDGET_HEADROOM_NUM) / CONTEXT_BUDGET_HEADROOM_DEN,
        None => DEFAULT_CONTEXT_TOKENS_ESTIMATE,
    }
}
/// 在总量没有触顶时，仍只保留最近这些用户轮次，避免长时间会话缓慢挤占内存。
const MAX_CONTEXT_USER_TURNS: usize = 8;
/// 标记 `self.messages[1]`（如果存在）是"早前对话摘要"消息，不是真实的历史内容——
/// `limit_context` 用这个前缀识别"要不要新建一条摘要消息，还是往已有的追加"。
const CONTEXT_SUMMARY_PREFIX: &str = "【早前对话摘要】";
/// 摘要消息自己的字符上限——长会话里如果不停追加摘要，摘要本身也会无限增长，超过这个
/// 阈值就丢弃最早的一截摘要片段（不再摘要一次摘要，避免过度设计）。2026-09 从 4_000
/// 提到 16_000：4_000 字符的预算下，摘要提示词又要求"保留文件路径/结论"，模型写出来
/// 的内容密度很高，多轮压缩后早期摘要几乎全被挤掉，等于白摘要——加大到 16_000 后
/// 一次探索性任务里翻过的十几个文件路径和结论基本能留得住，不用被下一轮摘要挤没。
const MAX_CONTEXT_SUMMARY_CHARS: usize = 16_000;
/// 摘要请求本身的超时——摘要是锦上添花，不能变成新的卡死点，超时/失败就直接退回硬删。
const CONTEXT_SUMMARY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// 把即将从 `self.messages` 里删掉的一批轮次压成一段 2-4 句的摘要——发一次不带
/// `tools` 字段的轻量请求，不计入工具循环本身（这是 `limit_context` 自己触发的
/// 独立请求，不是工具循环的一部分）。任何失败（网络错误、超时、响应里
/// 没有可用的文本）都返回 `None`，调用方直接退回"这批轮次就是删掉、不留痕迹"的
/// 原有行为——摘要是锦上添花，不能变成新的卡死点。
async fn summarize_dropped_turns(
    dropped: &[serde_json::Value],
    client: &reqwest::Client,
    provider: &AiProvider,
    api_key: &Option<String>,
) -> Option<String> {
    let transcript: String = dropped
        .iter()
        .filter_map(|message| {
            let role = message["role"].as_str()?;
            let content = match &message["content"] {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Null => String::new(),
                other => other.to_string(),
            };
            let content: String = content.chars().take(800).collect();
            let tool_note = message["tool_calls"]
                .as_array()
                .filter(|calls| !calls.is_empty())
                .map(|calls| format!("（调用了 {} 个工具）", calls.len()))
                .unwrap_or_default();
            Some(format!("[{role}]{tool_note} {content}"))
        })
        .collect::<Vec<_>>()
        .join("\n");
    if transcript.trim().is_empty() {
        return None;
    }
    let url = format!(
        "{}/chat/completions",
        provider.api_base.trim_end_matches('/')
    );
    let body = json!({
        "model": provider.model,
        "messages": [
            {
                "role": "system",
                "content": "你是对话摘要助手。这段摘要是给正在处理同一个任务的编程助手接着往下\
                             用的背景资料，不是写给人看的概述，必须具体到文件路径和关键发现，\
                             不能只给模糊结论——后续对话要能直接引用这些路径和发现，不用重新\
                             搜索/读文件。按下面的格式输出中文内容，没有的部分直接省略，不要写\
                             占位词：\n\
                             已读文件：<路径1>：<这个文件里的关键发现，一两句>；<路径2>：...\n\
                             已确认结论：<明确的技术结论/决策，逐条列>\n\
                             下一步：<如果任务还没做完，下一步打算做什么>\n\
                             不要写客套话，不要逐句复述对话过程，只保留后续用得上的具体信息。"
            },
            { "role": "user", "content": transcript }
        ]
    });
    let mut req = client.post(&url).json(&body);
    if let Some(key) = api_key {
        req = req.bearer_auth(key);
    }
    let resp = match tokio::time::timeout(CONTEXT_SUMMARY_TIMEOUT, req.send()).await {
        Ok(Ok(resp)) => resp,
        _ => return None,
    };
    let body: serde_json::Value = match resp.json().await {
        Ok(body) => body,
        Err(_) => return None,
    };
    let text = body["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or_default()
        .trim()
        .to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

/// `summarize_dropped_turns` 失败（网络错误/超时/Provider 没返回可用文本）时的
/// 兜底摘要——之前失败就直接把这批消息静默丢掉、不留任何痕迹，2026-09 真实
/// 复现：一次持续过载的长任务里摘要请求反复超时，被压缩掉的内容完全没留下
/// 任何线索，用户下一轮说"按刚才的方案"时模型手里是真正的一片空白，只能让
/// 用户把内容整个重新贴一遍。这里不依赖模型调用，纯算法从被删的消息里摘出
/// "用户说了什么、碰过哪些文件"，保底也要留下这点线索，不能什么都不剩——
/// 摘要质量比不上 LLM 生成的那版，但"有损线索"总比"彻底消失"强。
fn deterministic_fallback_summary(dropped: &[serde_json::Value]) -> String {
    let mut user_texts = Vec::new();
    let mut file_paths = std::collections::BTreeSet::new();
    for message in dropped {
        if message["role"].as_str() == Some("user") {
            if let Some(text) = message["content"].as_str() {
                let trimmed: String = text.chars().take(200).collect();
                if !trimmed.trim().is_empty() {
                    user_texts.push(trimmed);
                }
            }
        }
        if let Some(calls) = message["tool_calls"].as_array() {
            for call in calls {
                if let Some(args) = call["function"]["arguments"].as_str() {
                    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(args) {
                        for key in ["path", "file_path", "directory"] {
                            if let Some(p) = parsed[key].as_str() {
                                file_paths.insert(p.to_string());
                            }
                        }
                    }
                }
            }
        }
    }
    let mut summary =
        String::from("（自动摘要生成失败，以下是算法兜底提取的线索，细节不如正常摘要完整）\n");
    if !user_texts.is_empty() {
        summary.push_str("用户消息：");
        summary.push_str(&user_texts.join("；"));
        summary.push('\n');
    }
    if !file_paths.is_empty() {
        summary.push_str("涉及文件：");
        summary.push_str(&file_paths.into_iter().collect::<Vec<_>>().join("、"));
        summary.push('\n');
    }
    summary
}

impl CodingSession {
    fn evidence_version(mtime: i64, bytes: u64) -> String {
        format!("mtime={mtime};size={bytes}")
    }

    fn evidence_hash(text: &str) -> String {
        let mut h = Sha256::new();
        h.update(text.as_bytes());
        format!("{:x}", h.finalize())
    }

    async fn persist_file_evidence(&self, path: &str, text: &str, mtime: i64, bytes: u64) -> Uuid {
        let content: String = text.chars().take(MAX_EVIDENCE_BYTES).collect();
        let version = Self::evidence_version(mtime, bytes);
        let hash = Self::evidence_hash(&content);
        let entry = EvidenceEntry {
            id: Uuid::new_v4(),
            workspace_id: self.workspace_id,
            target_key: format!("{}", self.target_key()),
            kind: "file_snapshot".into(),
            query_hash: hash.clone(),
            path_or_url: path.into(),
            version_token: version,
            content_hash: hash,
            payload_json: serde_json::json!({"path": path}).to_string(),
            summary: format!("文件快照：{path}"),
            content,
            expires_at: None,
        };
        let id = entry.id;
        let _ = self.evidence_repo.upsert(&entry);
        id
    }

    fn target_key(&self) -> String {
        match &self.target {
            CodingTarget::Local => "local".into(),
            CodingTarget::Remote { connection_id, .. } => format!("ssh:{connection_id}"),
            CodingTarget::Agent { connection_id, .. } => format!("agent:{connection_id}"),
        }
    }
    /// 把上下文限制在一个可预期的内存/token 预算内。优先删历史时以"用户消息"为
    /// 边界整轮删（一轮 assistant tool_calls 与紧随其后的 tool 结果始终一起保留，
    /// 不会留下 OpenAI 兼容接口无法接受的孤立 tool message；最前面的 system 提示
    /// 和项目约定永远不删）。
    ///
    /// 2026-09 真实复现："继续几轮会话后模型又报 context_too_large"——根因是这套
    /// 整轮删除的策略只能删"已经有下一条用户消息、已经跑完的完整轮次"，对
    /// **当前正在跑的这一轮**完全无能为力：如果这一轮本身就带着好几十次工具调用、
    /// 每次都读了不小的文件/搜索结果，光是这一轮自己积累的 token 就能冲破预算，
    /// 而循环里找不到"下一条用户消息"作为 `end`，直接 `break` 放弃，请求原样带着
    /// 超预算的内容发出去，Provider 直接拒绝。codex-core 的压缩（`compact.rs`）是
    /// 按 token 数触发、不区分"轮次边界"的，这里补上同样的思路：找不到完整轮次可删
    /// 时，退而求其次，在**当前这一轮内部**找最旧的一组"assistant 工具调用 + 对应
    /// tool 结果"整体删掉（`trim_oldest_exchange_in_current_round`），只保留这一轮
    /// 最新的一组不动——模型至少还看得到最近一次工具调用的结果，旧的换成摘要。
    async fn limit_context(&mut self, client: &reqwest::Client, provider: &AiProvider, api_key: &Option<String>) {
        self.inject_evidence_recall();
        let budget = effective_context_budget(provider);
        let mut user_turns = self
            .messages
            .iter()
            .filter(|message| message["role"].as_str() == Some("user"))
            .count();
        loop {
            let size_tokens = agent_llm::estimate_tokens(
                self.messages
                    .iter()
                    .map(|message| message.to_string().len())
                    .sum::<usize>(),
            );
            if size_tokens <= budget && user_turns <= MAX_CONTEXT_USER_TURNS {
                break;
            }
            let Some(start) = self
                .messages
                .iter()
                .position(|message| message["role"].as_str() == Some("user"))
            else {
                break;
            };
            let end = self
                .messages
                .iter()
                .enumerate()
                .skip(start + 1)
                .find_map(|(index, message)| {
                    (message["role"].as_str() == Some("user")).then_some(index)
                });
            match end {
                Some(end) => {
                    let dropped: Vec<serde_json::Value> = self.messages.drain(start..end).collect();
                    user_turns -= 1;
                    let summary = match summarize_dropped_turns(&dropped, client, provider, api_key).await {
                        Some(summary) => summary,
                        None => deterministic_fallback_summary(&dropped),
                    };
                    self.append_context_summary(&summary);
                }
                None => {
                    if !self
                        .trim_oldest_exchange_in_current_round(start, client, provider, api_key)
                        .await
                    {
                        // 当前这一轮里已经没有"更旧、可以安全删掉"的工具交换了
                        // （只剩最新一组，不能动）——没有更多能做的，放弃继续裁剪，
                        // 避免死循环；请求可能仍然超预算，但已经尽力压缩过。
                        break;
                    }
                }
            }
        }
    }

    /// 在当前这一轮（`round_start` 是这一轮用户消息的下标）内部，找最旧的一组
    /// "assistant 工具调用 + 紧随其后的 tool 结果"整体删掉，换成一段摘要——是
    /// `limit_context` 在"没有完整轮次可删"时的退路（见上面文档）。刻意保留这一轮
    /// **最新**的一组交换不动：模型至少要能看到刚发生的这次工具调用结果，不能因为
    /// 压缩把手头正在用的信息也删掉。只剩一组交换（就是最新这组）时没有更旧的可删，
    /// 返回 `false` 告诉调用方"这里已经没办法再压缩了"。
    async fn trim_oldest_exchange_in_current_round(
        &mut self,
        round_start: usize,
        client: &reqwest::Client,
        provider: &AiProvider,
        api_key: &Option<String>,
    ) -> bool {
        let mut exchanges: Vec<(usize, usize)> = Vec::new();
        let mut i = round_start + 1;
        while i < self.messages.len() {
            if self.messages[i]["role"].as_str() == Some("assistant") {
                let unit_start = i;
                let mut j = i + 1;
                while j < self.messages.len() && self.messages[j]["role"].as_str() == Some("tool")
                {
                    j += 1;
                }
                exchanges.push((unit_start, j));
                i = j;
            } else {
                i += 1;
            }
        }
        if exchanges.len() <= 1 {
            return false;
        }
        let (unit_start, unit_end) = exchanges[0];
        let dropped: Vec<serde_json::Value> = self.messages.drain(unit_start..unit_end).collect();
        let summary = match summarize_dropped_turns(&dropped, client, provider, api_key).await {
            Some(summary) => summary,
            None => deterministic_fallback_summary(&dropped),
        };
        self.append_context_summary(&summary);
        true
    }

    fn inject_evidence_recall(&mut self) {
        let Some(query) = self.messages.iter().rev().find(|m| m["role"].as_str() == Some("user")).and_then(|m| m["content"].as_str()).map(str::trim).filter(|q| !q.is_empty()) else { return; };
        if self.evidence_recall_query.as_deref() == Some(query) { return; }
        self.evidence_recall_query = Some(query.to_string());
        let terms = query.split_whitespace().take(8).collect::<Vec<_>>().join(" OR ");
        let Ok(rows) = self.evidence_repo.search_fts(self.workspace_id, &self.target_key(), &terms, 8) else { return; };
        if rows.is_empty() { return; }
        let body = rows.into_iter().map(|(id, path, summary)| format!("- evidence_id={id} source={path} {summary}")).collect::<Vec<_>>().join("\n");
        self.messages.push(json!({"role":"system","content":format!("[历史证据召回，仅供定位，需用 read_evidence 获取正文]\n{body}")}));
    }
    /// 把一段新摘要文本并入 `self.messages` 里专门的摘要消息——用 `CONTEXT_SUMMARY_PREFIX`
    /// 这个前缀识别"哪条是摘要消息"，而不是假设固定下标：项目记忆（AGENTS.md/CLAUDE.md）
    /// 也是插在最前面的 system 消息，数量随项目而变，摘要消息必须插在"所有前置 system
    /// 消息之后、第一条非 system 消息之前"才不会打乱这个顺序。摘要自己超过字符上限时
    /// 从最早的部分开始截，保留最近的内容——新摘要通常比旧摘要更贴近当前话题。
    fn append_context_summary(&mut self, new_piece: &str) {
        if let Some(existing) = self.messages.iter_mut().find(|message| {
            message["role"].as_str() == Some("system")
                && message["content"]
                    .as_str()
                    .is_some_and(|c| c.starts_with(CONTEXT_SUMMARY_PREFIX))
        }) {
            let mut body = existing["content"]
                .as_str()
                .unwrap_or_default()
                .strip_prefix(CONTEXT_SUMMARY_PREFIX)
                .unwrap_or_default()
                .trim_start_matches('\n')
                .to_string();
            body.push('\n');
            body.push_str(new_piece);
            if body.chars().count() > MAX_CONTEXT_SUMMARY_CHARS {
                let skip = body.chars().count() - MAX_CONTEXT_SUMMARY_CHARS;
                body = format!(
                    "（更早的摘要已省略）\n{}",
                    body.chars().skip(skip).collect::<String>()
                );
            }
            *existing = json!({
                "role": "system",
                "content": format!("{CONTEXT_SUMMARY_PREFIX}\n{body}")
            });
        } else {
            let insert_at = self
                .messages
                .iter()
                .position(|message| message["role"].as_str() != Some("system"))
                .unwrap_or(self.messages.len());
            self.messages.insert(
                insert_at,
                json!({
                    "role": "system",
                    "content": format!("{CONTEXT_SUMMARY_PREFIX}\n{new_piece}")
                }),
            );
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
        evidence_repo: Arc<AiEvidenceRepo>,
    ) -> Self {
        // 2026-09 复盘：不明确告诉模型"你在什么系统上、用什么 shell"，它会凭训练数据的
        // 默认假设去猜命令语法（最常见的是无论目标是什么系统都先猜 Linux/bash），猜错了
        // 要么命令直接执行失败、要么在远程/受限 shell 下产生更隐蔽的语法错误，模型自己
        // 还得再花几轮工具调用才能反应过来。这里的平台/shell 映射跟
        // `run_local_ai_command_output`/`log::remote::shell_quote`/`cmd_quote`
        // 现有的转义假设保持一致，不是新发明的映射关系——2026-09 曾经出现过
        // 这里说是 PowerShell、`run_command` 实际默认却走 cmd.exe 的不一致
        // （模型写 PowerShell 专属语法被 cmd.exe 拒收），已经把默认解释器改成
        // 真的对应 PowerShell，见 `run_local_ai_command_output` 的文档。
        let target_desc = match &target {
            CodingTarget::Local => "本地工作区，Windows，命令行执行环境是 PowerShell".to_string(),
            CodingTarget::Remote { host_label, .. } => {
                format!("远程主机 {host_label}，Linux，命令行执行环境是 bash")
            }
            CodingTarget::Agent { host_label, .. } => {
                format!("远程 Windows 主机 {host_label}，命令行执行环境是 cmd.exe")
            }
        };
        // 2026-08-18 真实复现：分析整个项目/做代码评审这类开放式大任务，模型会没完
        // 没了地交替 search_files/read_file，一直不给结论。后端曾经有过硬编码的轮次
        // 上限兜底（见 send_message 顶部的历史记录），2026-09 用户明确要求去掉这层
        // 限制，工具循环现在没有硬上限，全靠模型自己判断"该收敛就收敛"——提示词里
        // 仍然给出这个预期，只是不再说"总次数有限"（那句话配合已移除的硬上限说得通，
        // 单独留着会误导模型以为还有一个不存在的配额，反而更容易在长任务里过早收尾）。
        let system_prompt = format!(
            "你是集成在 roc_desk 桌面工具里的 AI 编程助手，当前绑定的工作区根目录是 `{workspace_root}`（{target_desc}）。\
             run_command 执行的命令必须匹配上面说明的 shell 语法，不要凭空假设是另一种系统/shell。\
             你可以用提供的工具读写文件、搜索代码、访问互联网、执行命令。涉及“今天/最新/新闻/外部事实”的问题必须先调用 web_search，\
             不要凭模型记忆回答。write_file/edit_file 产生的改动不会立即生效，\
             而是生成 Diff 交给用户确认，所以你可以放心连续提出多个改动，不需要等待每一步都被确认才能继续推理。\
             run_command 有安全限制：破坏性命令会被直接拦截，其余命令需要用户在弹窗里确认才会真正执行。\
             面对\"分析整个项目\"这类开放式大任务时，优先用 search_files/list_directory 快速定位\
             最相关的一小批文件（不需要每个文件都读一遍），读完这些就给出结论；不要为了追求\"看得更全\"\
             而无休止地继续搜索/读取，觉得信息已经够回答用户的问题时就直接总结，而不是再多看几个文件。\
             \n\n读文件内容优先用 read_file 工具，不要用 run_command 里 sed/cat/head 这类命令去手动\
             分段读——read_file 会自动按合理长度截断并在截断处提示，不需要你自己为了\"怕超长\"而每次\
             只读几十行、切成一大堆零碎调用（这样反而更浪费工具调用次数）；单次工具结果本身有长度保护，\
             可以放心一次性多读一些内容（比如几百行），不用过度保守。\
             \n\n用户的话如果有明显歧义、可能对应两种差别很大的意图（比如一句简短的\"继续\"\"报错了\"\
             \"确认\"，既可能是在接着上一个没答完的问题往下走，也可能是在描述一件跟上文完全无关的新情况），\
             不要凭猜测直接选一种理解就展开长篇回答——调用 question 工具，把你想到的几种理解列成\
             options 让用户选一下，等用户选完再按确定下来的理解继续，比自己猜错了、答非所问、用户还要\
             再纠正一轮更省事。只有在意图已经足够清楚、只是细节需要你自己判断的情况下，才不需要用这个\
             工具反复确认——不要把它用成什么都要问一遍的过度谨慎。\
             \n\n如果歧义的原因是你手头缺上下文——比如用户说\"按刚才的方案\"\"继续处理\"\"你说的那个\
             办法\"，但当前对话历史里根本找不到对应的具体内容（长对话被自动摘要压缩后会发生这种情况）——\
             不要立刻用 question 工具让用户把内容重新讲一遍。先主动用手头已有的工具自己找线索：用\
             git_status/git_diff/run_command 跑 git log -n 10 看最近改了什么，用 read_file/\
             list_directory/search_files 看用户提到的文件、函数现在是什么状态。多数时候当前代码库的\
             实际情况就足够你推断出大致是要做什么、直接继续把任务做完，不需要用户重新说一遍。只有这样\
             查过之后仍然拿不准关键决策（比如有两种同样合理但结果差异很大的做法）时，才用 question 工具\
             问一个具体、带着你已经查到的线索的问题（例如\"我看到 X 文件现在是 Y 状态，接下来是按 A 方式\
             改还是 B 方式改？\"），而不是空泛地让用户把整个方案重新讲一遍。\
             \n\n除了前面提到的这些，还有几个更专用的工具，适用时优先用它们而不是绕远路用 run_command 拼命令行：\
             查看仓库状态/改动用 git_status/git_diff（只读，不需要确认），确定要提交时用 git_commit（需要\
             明确给出要提交的路径列表）；同一个文件要改好几处时用 multi_edit 一次性提交，不要为了改一个文件\
             连续调好几次 edit_file；按函数名/类名找定义位置优先用 find_definition，比用 search_files 猜关键词\
             更精确（找不到再退回 search_files）；需要启动一个不会自己退出的进程（开发服务器、watch 进程）\
             时用 run_command_background，不要用 run_command——那会一直等它退出、把这一轮卡住，配合\
             read_background_output 查看输出、stop_background_process 结束它；遇到\"在一大堆文件里找到某个\
             具体结论\"这种会消耗大量探索性工具调用、但你自己只需要一个结论的子任务，可以用 task 委派给一个\
             独立上下文的子代理去做，避免探索过程占满你自己的上下文——但子任务之间没有共享的探索上下文，\
             委派时要把背景信息写全。",
        );
        Self {
            id,
            workspace_id,
            workspace_root,
            target,
            mode: CodingMode::Plan,
            provider_id,
            change_store,
            todos: Vec::new(),
            skills: Vec::new(),
            project_memory_loaded: Vec::new(),
            evidence_repo,
            evidence_recall_query: None,
            messages: vec![json!({ "role": "system", "content": system_prompt })],
            file_ops,
            current_turn_id: Uuid::new_v4(),
            awaiting_confirmation_turn: None,
            background_jobs: HashMap::new(),
        }
    }

    /// `commands::coding` 的 accept/reject 在确认"这一轮改动已经没有 Pending 的了"
    /// 之后调用：`turn_id` 匹配当前正卡着的那一轮才真正清掉标记、返回 `true`
    /// （告诉调用方可以触发自动续跑了）；不匹配（比如这是更早一轮遗留、用户很久
    /// 之后才处理的改动，此时会话可能已经在跑更新的一轮）就什么都不做、返回
    /// `false`——避免过期的确认误触发一次不相关的自动续跑。
    pub fn resolve_awaiting_confirmation(&mut self, turn_id: Uuid) -> bool {
        if self.awaiting_confirmation_turn == Some(turn_id) {
            self.awaiting_confirmation_turn = None;
            true
        } else {
            false
        }
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
            // 不在这里裁剪——`limit_context` 现在是 async 的（触顶时会发一次摘要
            // 请求），而这里没有现成的 provider/api_key 可用。恢复历史后的第一条
            // 新消息会走 `send_message` 里的循环，进去就会调用一次 `limit_context`，
            // 到时候一并裁剪即可，不需要在恢复这一步重复做。
        }
    }

    /// 把这个会话在 `AppState.coding_pending_injections` 里攒到的插话消息（如果
    /// 有）取出并清空、按顺序追加进 `messages`。用 `workspace_id` 做 key（和
    /// `coding_cancel_tokens` 同一个键空间），不是 `self.id`——`inject_message`
    /// 命令拿到的也是 workspace_id，会话内部 id 前端根本不需要知道。
    async fn drain_pending_injections(
        &mut self,
        pending_injections: &Arc<StdMutex<HashMap<Uuid, Vec<PendingInjection>>>>,
        client: &reqwest::Client,
        provider: &AiProvider,
        api_key: &Option<String>,
        app_handle: &AppHandle,
    ) {
        let injected = {
            let mut map = pending_injections.lock().unwrap();
            map.remove(&self.workspace_id).unwrap_or_default()
        };
        for inj in injected {
            let content =
                build_user_message_content(&inj.text, &inj.attachments, client, provider, api_key, app_handle, self.id)
                    .await;
            self.messages.push(json!({ "role": "user", "content": content }));
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
        pending_injections: &Arc<StdMutex<HashMap<Uuid, Vec<PendingInjection>>>>,
        symbol_indexes: &Arc<RwLock<HashMap<Uuid, SymbolIndex>>>,
    ) -> Result<String, AppError> {
        // provider/api_key/client 提到这里最先解析——`build_user_message_content`
        // （附件超预算时的分窗口提取）和下面的 `drain_pending_injections` 都需要
        // 用它们发请求，原来这三行是在压完用户消息之后才解析的，2026-09 加上
        // "附件自动分窗口"这个需求之后必须挪到前面。
        let provider = providers.get(self.provider_id)?.ok_or_else(|| {
            AppError::NotFound(format!("ai provider not found: {}", self.provider_id))
        })?;
        let api_key = providers.resolve_api_key(&provider).await?;
        let client = reqwest::Client::new();

        // 先捞一遍上一轮结束后才插进来、还没来得及塞进 `messages` 的消息（如果
        // 有），再压进这次用户主动发的 `user_text`——保证时间顺序：先来的排前面。
        self.drain_pending_injections(pending_injections, &client, &provider, &api_key, app_handle)
            .await;
        self.current_turn_id = Uuid::new_v4();

        let content =
            build_user_message_content(user_text, attachments, &client, &provider, &api_key, app_handle, self.id)
                .await;
        self.messages.push(json!({ "role": "user", "content": content }));
        // 不在这里裁剪——下面工具循环（`loop { ... }`）第一轮一进去就会调用
        // `limit_context`，那时候才需要，摘要请求同样用得上上面已经解析好的
        // provider/api_key/client。
        // 权限规则不在这里一次性取快照——早期版本只在这里 `PermissionEngine::load`
        // 一次、整轮工具调用循环共用同一份快照，导致用户在"确认执行命令"弹窗还开着
        // 的时候跑去权限规则管理里新增/改一条规则，当前这一轮后面的工具调用完全看
        // 不到这条新规则，还是照样弹确认（2026-09 用户反馈"我已经放开只读直接过了，
        // 它还是弹"）。现在改成每个工具调用决策前才现取一份最新规则（下面
        // `for call in &tool_calls` 循环里），本地 SQLite 读一次的开销可以忽略，
        // 换来的是规则改动能在同一轮对话里立刻生效，不用等到下一条消息。
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

        // 这一整轮（一条用户消息到最终给出结论）可能要跑好几次工具循环迭代，
        // 每次迭代都是一次独立的 API 请求——累加起来才是用户真正关心的"这轮
        // 对话一共花了多少 token"，单次请求的消耗只在过程中当进度参考。
        let mut turn_usage = agent_llm::TurnUsage::default();
        let session_id = self.id;

        // 2026-09 用户明确要求去掉硬编码的轮次上限（见本文件顶部 `MAX_TOOL_ITERATIONS`
        // 旧常量位置的历史记录）：这里不再是 `for i in 0..N`，而是一个没有内建终止
        // 条件的 `loop`——工具循环只在两种情况下结束：模型自己不再调用任何工具（下面
        // `tool_calls.is_empty()` 分支里 `return Ok(text)`），或者用户点了"停止"
        // （`cancel_token` 被取消，下面立刻 `return Err`）。不会再出现"最后几轮强制
        // 断供 tools、还留一条永久性的系统消息误导后续轮次"的情况，代价是真遇到模型
        // 陷入死循环、一直调工具不收敛的场景，没有自动兜底，只能用户手动停止。
        loop {
            if cancel_token.is_cancelled() {
                return Err(AppError::Internal(
                    "已停止：用户取消了当前对话轮次".to_string(),
                ));
            }
            // 每轮工具调用之间的检查点——用户在这一轮进行中插的话（`inject_message`
            // 命令，不经过这个会话外层的锁）攒到这里才被真正塞进对话上下文，供
            // 下一次模型请求看到；不是打断正在跑的这次请求，是"下一轮生效"。
            self.drain_pending_injections(pending_injections, &client, &provider, &api_key, app_handle)
                .await;
            self.limit_context(&client, &provider, &api_key).await;

            let mut tools = tools_for_mode(self.mode);
            if let Some(arr) = tools.as_array_mut() {
                arr.extend(mcp_tool_defs.iter().cloned());
            }

            // 一次带工具调用的 LLM 请求（两种 wire 协议归一化、429/5xx 重试、
            // usage 抽取）跟`coding` 场景完全无关的部分都挪到了 `agent_llm`，
            // 跟 `sql::agent` 共用同一份实现——见该模块文档。
            let round = match agent_llm::call_llm_once(
                &client,
                &provider,
                &api_key,
                &self.messages,
                Some(&tools),
                app_handle,
                session_id,
                "coding",
                cancel_token,
            )
            .await
            {
                Ok(round) => round,
                Err(agent_llm::LlmCallError::Cancelled) => {
                    return Err(AppError::Internal(
                        "已停止：用户取消了当前对话轮次".to_string(),
                    ));
                }
                Err(agent_llm::LlmCallError::RequestFailed(detail)) => {
                    self.messages.push(agent_llm::request_failed_message(&detail));
                    return Err(AppError::Connection(detail));
                }
                Err(agent_llm::LlmCallError::ParseFailed(e)) => {
                    self.messages.push(agent_llm::parse_failed_message(&e));
                    return Err(AppError::Internal(format!("解析响应失败: {e}")));
                }
            };
            turn_usage.add(round.usage);
            let message = round.message;
            let tool_calls = round.tool_calls;

            if tool_calls.is_empty() {
                let text = message["content"].as_str().unwrap_or("").to_string();
                self.messages
                    .push(json!({ "role": "assistant", "content": text }));
                turn_usage.emit_summary(app_handle, session_id, "coding");
                // 这一轮到此为止（模型不再调用工具）——如果这一轮里 `stage_change`
                // 生成的改动还有没被用户处理的（`full_auto` 关闭时默认状态），记下
                // 这一轮的 id，供之后 accept/reject 判断"是不是这一轮的改动都处理完
                // 了、该自动帮用户把对话续上"（见 `awaiting_confirmation_turn` 字段
                // 文档、`resolve_awaiting_confirmation`）。没有遗留 Pending 改动就
                // 清空，避免残留上一次判断错误留下的标记。
                let has_pending_this_turn = {
                    let store = self.change_store.lock().await;
                    store.changes().iter().any(|c| {
                        c.turn_id == self.current_turn_id && c.status == ChangeStatus::Pending
                    })
                };
                self.awaiting_confirmation_turn =
                    has_pending_this_turn.then_some(self.current_turn_id);
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
                // 2026-09 真实复现：远程目标下 `search_files` 直接把 `rg`/`grep`
                // 的原始输出整段返回（`Local` 分支自己限了 50 条结果，`Remote`/
                // `Agent` 分支当时漏了同样的限制），一次搜到大量匹配时单条工具
                // 结果能到两千多万字符，Responses API 直接报
                // "input[i].output: string too long" 拒绝整个请求——`cap_tool_result`
                // 之前只在 `read_file`/`list_directory` 这两个调用点手动套了一层，
                // 不是每个工具分支都记得套。这里挪到 `execute_tool` 唯一的结果
                // 汇聚点统一兜底，不管以后新增哪个工具、哪个分支忘记自己限制长度，
                // 单条工具结果都不可能超过 `MAX_TOOL_RESULT_CHARS`。
                // 每个工具调用决策前现取一份最新规则快照，而不是整轮共用一份加载于
                // 循环之前的旧快照——见本函数顶部的说明，这样规则改动能在同一轮
                // 对话里立刻生效。
                let permission_engine = PermissionEngine::load(permission_rules)?;
                let result_text = agent_llm::cap_tool_result(match call_result {
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
                            &client,
                            &provider,
                            &api_key,
                            symbol_indexes,
                            cancel_token,
                            app_handle,
                        )
                        .await
                        .unwrap_or_else(|e| format!("工具执行出错：{e}")),
                    Err(e) => format!("工具调用参数解析失败：{e}"),
                });

                // 带上 `result_text`：之前这个事件只报"哪个工具跑完了"，实际拿到的
                // 结果只存进 `self.messages` 发给模型，前端完全看不到——用户反馈
                // "想点一下时间线里已完成的命令，看看它到底执行出了什么"，时间线
                // 需要这份数据才能在用户点开时展示出来。
                let _ = app_handle.emit(
                    "coding:tool-call-end",
                    json!({ "sessionId": self.id, "tool": fn_name, "output": result_text }),
                );

                self.messages.push(json!({
                    "role": "tool",
                    "tool_call_id": call_id,
                    "content": result_text,
                }));
            }
        }
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
        // 下面这三个只有 `ToolCall::Task` 分支会用到（子代理要发起自己的 LLM
        // 请求）——之所以还是加进这个统一的签名而不是单独开一条路径，是因为
        // `task` 工具的执行逻辑（`run_subagent_task`）需要递归调用回这同一个
        // `execute_tool` 来跑子代理自己的工具调用，两边共用一份签名更简单。
        client: &reqwest::Client,
        provider: &AiProvider,
        api_key: &Option<String>,
        symbol_indexes: &Arc<RwLock<HashMap<Uuid, SymbolIndex>>>,
        cancel_token: &tokio_util::sync::CancellationToken,
        app_handle: &AppHandle,
    ) -> Result<String, AppError> {
        match call {
            ToolCall::ReadFile { path } => {
                if let Some(content) = self.change_store.lock().await.pending_content_for(&path) {
                    return Ok(agent_llm::cap_tool_result(content));
                }
                // 先以文件大小做廉价的弱校验，命中后仍由版本 token 标记为可复用快照。
                if let Ok(size) = self.file_ops.file_size(&path).await {
                    if let Ok(Some(entry)) = self.evidence_repo.get_latest_path(
                        self.workspace_id, &self.target_key(), &path, size,
                    ) {
                        return Ok(agent_llm::cap_tool_result(entry.content));
                    }
                }
                // `read_file_for_editor`（不是不设上限的 `read_file`）：超过
                // `EDITOR_PREVIEW_MAX_BYTES` 只读前面一部分字节，不会像默认实现那样
                // 把一个几百 MB 的日志/压缩包/生成产物整份吸进内存——见下面
                // `cap_tool_result` 的注释，这是同一个内存失控问题的另一半修复
                // （字节层面 vs. 字符层面）。
                let content = self.file_ops.read_file_for_editor(&path).await?;
                let evidence_id = self.persist_file_evidence(&path, &content.text, content.mtime, content.total_size).await;
                Ok(format!("{}\n[evidence_id={evidence_id}]", agent_llm::cap_tool_result(content.text)))
            }
            ToolCall::ReadEvidence { id, start_line, end_line } => {
                let entry = self.evidence_repo.get_by_id(id)?.ok_or_else(|| AppError::NotFound("证据不存在或已清理".into()))?;
                let lines: Vec<&str> = entry.content.lines().collect();
                let start = start_line.unwrap_or(1).max(1);
                let end = end_line.unwrap_or(lines.len()).min(lines.len().max(start));
                let body = if lines.is_empty() { String::new() } else { lines[(start - 1).min(lines.len())..end].join("\n") };
                Ok(format!("[evidence_id={id} source={} version={}]\n{}", entry.path_or_url, entry.version_token, agent_llm::cap_tool_result(body)))
            }
            ToolCall::ListDirectory { path } => {
                let entries = self.file_ops.list_dir(&path).await?;
                Ok(agent_llm::cap_tool_result(
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
            ToolCall::MultiEdit { path, edits } => {
                let mut content = match self.change_store.lock().await.pending_content_for(&path) {
                    Some(c) => c,
                    None => self.file_ops.read_file(&path).await?.text,
                };
                for (i, edit) in edits.iter().enumerate() {
                    if !content.contains(&edit.old_text) {
                        return Err(AppError::Internal(format!(
                            "multi_edit 失败：第 {} 处替换的 old_text 在 {path} 中没有找到匹配（前面 {} 处\
                             已经在内存里预演成功，但这次调用整体不会生效，不会留下部分修改），请先用 \
                             read_file 确认最新内容后重试",
                            i + 1,
                            i
                        )));
                    }
                    content = content.replacen(&edit.old_text, &edit.new_text, 1);
                }
                self.stage_change(&path, content, ssh_pool, agent_pool, app_handle)
                    .await
            }
            ToolCall::GitStatus { path } => Ok(agent_llm::cap_tool_result(
                git_ops::status(&self.target, &self.workspace_root, path.as_deref(), ssh_pool, agent_pool)
                    .await?,
            )),
            ToolCall::GitDiff { path } => Ok(agent_llm::cap_tool_result(
                git_ops::diff(&self.target, &self.workspace_root, path.as_deref(), ssh_pool, agent_pool)
                    .await?,
            )),
            ToolCall::GitCommit { message, paths } => {
                if paths.is_empty() {
                    return Err(AppError::Internal(
                        "git_commit 失败：paths 不能为空，请明确给出要提交的文件/目录路径（可以先用 \
                         git_status 确认）"
                            .to_string(),
                    ));
                }
                let (auto_allow_readonly, full_auto) = {
                    let store = self.change_store.lock().await;
                    (
                        store.auto_allow_readonly.load(std::sync::atomic::Ordering::Relaxed),
                        store.full_auto.load(std::sync::atomic::Ordering::Relaxed),
                    )
                };
                let synthetic_command = format!("git commit -m {message:?} -- {}", paths.join(" "));
                match gate_command(
                    self.id,
                    &self.target,
                    auto_allow_readonly,
                    full_auto,
                    &synthetic_command,
                    audit,
                    confirms,
                    permission_engine,
                    app_handle,
                )
                .await
                {
                    GateOutcome::Blocked(msg) => Ok(msg),
                    GateOutcome::Allowed => Ok(agent_llm::cap_tool_result(
                        git_ops::commit_paths(
                            &self.target,
                            &self.workspace_root,
                            &paths,
                            &message,
                            ssh_pool,
                            agent_pool,
                        )
                        .await?,
                    )),
                }
            }
            ToolCall::RunCommandBackground { command } => {
                self.run_command_background_gated(&command, audit, confirms, permission_engine, app_handle)
                    .await
            }
            ToolCall::ReadBackgroundOutput { job_id } => self.read_background_output(job_id),
            ToolCall::StopBackgroundProcess { job_id } => self.stop_background_process(job_id),
            ToolCall::FindDefinition { symbol } => self.find_definition(&symbol, symbol_indexes).await,
            ToolCall::Task { description, prompt } => {
                Box::pin(self.run_subagent_task(
                    &description,
                    &prompt,
                    client,
                    provider,
                    api_key,
                    ssh_pool,
                    agent_pool,
                    audit,
                    confirms,
                    permission_engine,
                    question_confirms,
                    mcp_manager,
                    symbol_indexes,
                    cancel_token,
                    app_handle,
                ))
                .await
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
        let mut h = Sha256::new();
        h.update(pattern.as_bytes()); h.update([0]); h.update(path.as_bytes());
        let query_hash = format!("{:x}", h.finalize());
        if let Some(entry) = self.evidence_repo.get_exact(self.workspace_id, &self.target_key(), "content_search", &query_hash, path, "search-v1")? {
            return Ok(format!("{}\n[evidence_id={} cache_hit=true]", entry.content, entry.id));
        }
        let result = self.search_files_uncached(pattern, path, ssh_pool, agent_pool).await?;
        let content: String = result.chars().take(MAX_EVIDENCE_BYTES).collect();
        let id = Uuid::new_v4();
        let mut ch = Sha256::new(); ch.update(content.as_bytes());
        let hash = format!("{:x}", ch.finalize());
        let entry = EvidenceEntry { id, workspace_id: self.workspace_id, target_key: self.target_key(), kind: "content_search".into(), query_hash, path_or_url: path.into(), version_token: "search-v1".into(), content_hash: hash, payload_json: serde_json::json!({"pattern": pattern, "path": path}).to_string(), summary: format!("搜索 {pattern} in {path}"), content, expires_at: None };
        let _ = self.evidence_repo.upsert(&entry);
        Ok(format!("{}\n[evidence_id={} cache_hit=false]", result, id))
    }

    async fn search_files_uncached(
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
                // 2026-09 真实复现：这条命令原来没有任何行数上限，一次搜到大量
                // 匹配时单条工具结果能到两千多万字符，直接把 Responses API 的单
                // 字段长度上限（10MB）冲爆，整个请求被拒绝——`Local` 分支自己
                // 限了 50 条结果，这里当时漏了同样的限制。`head -n 200` 让远端
                // 自己截断，比"整段传回来本地再截"省一次几十 MB 的 SSH 往返。
                let cmd = format!(
                    "rg -n -F -- {quoted_pattern} {quoted_path} 2>/dev/null | head -n 200 || grep -rn -F -- {quoted_pattern} {quoted_path} 2>/dev/null | head -n 200"
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
                        // 跟 `Local`/`Remote` 分支同一个理由（见上面 `Remote` 分支的
                        // 注释）：Agent 协议本身没有 limit 参数，返回多少条这里就收
                        // 多少条，客户端这层至少不把它拼成一个无上限的大字符串——
                        // 200 条封顶，跟 `Remote` 分支的 `head -n 200` 对齐。
                        let mut lines = Vec::new();
                        'outer: for file in results {
                            for m in file.matches {
                                if lines.len() >= 200 {
                                    break 'outer;
                                }
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
        let _ = self.evidence_repo.invalidate_path(self.workspace_id, &self.target_key(), path);
        // `sync` 非空表示"自动应用"（`auto_apply_changes`，默认开启，或用户额外
        // 开了"完全授权模式" `full_auto`）已经把这个改动直接落盘了——一并广播
        // 出去，前端据此刷新这个路径对应的、可能已经打开的编辑器 buffer（否则
        // 磁盘内容变了，编辑器里显示的还是旧内容）。
        let _ = app_handle.emit(
            "coding:file-change",
            json!({ "sessionId": self.id, "change": &change, "sync": sync }),
        );
        Ok(if applied {
            format!("已为 {path} 生成变更（id={id}），已直接写入磁盘，用户可在界面上点\"撤销\"。")
        } else {
            format!("已为 {path} 生成变更（id={id}），已在界面展示 Diff，等待用户 Accept 后才会真正写入磁盘。")
        })
    }

    /// 实际逻辑在自由函数 `run_command_gated_shared` 里（见下方），这里只是把
    /// `&self` 上的几个字段拆出来传过去。
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
        let (auto_allow_readonly, full_auto) = {
            let store = self.change_store.lock().await;
            (
                store.auto_allow_readonly.load(std::sync::atomic::Ordering::Relaxed),
                store.full_auto.load(std::sync::atomic::Ordering::Relaxed),
            )
        };
        run_command_gated_shared(
            self.id,
            &self.target,
            &self.workspace_root,
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
        .await
    }

    /// `run_command_background` 工具——只支持 `CodingTarget::Local`：Remote/Agent
    /// 目标下的"后台进程"意味着要在一条 SSH/Agent 连接上一直保持某种状态、跨
    /// 多次工具调用复用，现有连接池是"每次 exec 用完就还回去"的短连接模式，硬
    /// 要支持这个场景需要单独一套远程会话状态管理，收益（远程开发服务器场景）
    /// 暂时不足以承担这份复杂度，先只做本地。
    pub(crate) async fn run_command_background_gated(
        &mut self,
        command: &str,
        audit: &AuditLogRepo,
        confirms: &CommandConfirmRegistry,
        permission_engine: &PermissionEngine,
        app_handle: &AppHandle,
    ) -> Result<String, AppError> {
        if !matches!(self.target, CodingTarget::Local) {
            return Ok(
                "后台执行目前只支持本地工作区。远程/Agent 目标请改用 run_command，在命令本身里用 \
                 shell 自带的后台手段（比如 Linux 下 `nohup ... &`，Windows 下 `Start-Process`）。"
                    .to_string(),
            );
        }
        let (auto_allow_readonly, full_auto) = {
            let store = self.change_store.lock().await;
            (
                store.auto_allow_readonly.load(std::sync::atomic::Ordering::Relaxed),
                store.full_auto.load(std::sync::atomic::Ordering::Relaxed),
            )
        };
        match gate_command(
            self.id,
            &self.target,
            auto_allow_readonly,
            full_auto,
            command,
            audit,
            confirms,
            permission_engine,
            app_handle,
        )
        .await
        {
            GateOutcome::Blocked(message) => return Ok(message),
            GateOutcome::Allowed => {}
        }

        #[cfg(target_os = "windows")]
        let mut cmd = windows_command_for(command, true);
        #[cfg(not(target_os = "windows"))]
        let mut cmd = unix_command_for(command);
        cmd.current_dir(&self.workspace_root);
        cmd.stdin(std::process::Stdio::null());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        // 这里故意不设 `kill_on_drop`——这个 `Child` 会被塞进 `self.background_jobs`
        // 长期持有，它该被杀掉的时机是"用户调用 stop_background_process"或者
        // "整个会话结束"（`Drop for CodingSession` 统一兜底，见结构体字段文档），
        // 不是"这次 execute_tool 调用返回"，不需要 tokio 在这里提前介入。
        let mut child = cmd
            .spawn()
            .map_err(|e| AppError::Internal(format!("启动后台进程失败：{e}")))?;
        let pid = child.id();
        let output = Arc::new(StdMutex::new(String::new()));
        if let Some(stdout) = child.stdout.take() {
            let buf = output.clone();
            tokio::spawn(async move {
                let mut lines = tokio::io::BufReader::new(stdout).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    append_background_output(&buf, &format!("{line}\n"));
                }
            });
        }
        if let Some(stderr) = child.stderr.take() {
            let buf = output.clone();
            tokio::spawn(async move {
                let mut lines = tokio::io::BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    append_background_output(&buf, &format!("[stderr] {line}\n"));
                }
            });
        }
        let job_id = Uuid::new_v4();
        self.background_jobs.insert(
            job_id,
            BackgroundJob {
                command: command.to_string(),
                child,
                output,
            },
        );
        Ok(format!(
            "已在后台启动{}，job_id={job_id}。用 read_background_output 查看目前的输出，用 \
             stop_background_process 结束它。",
            pid.map(|p| format!("（pid={p}）")).unwrap_or_default()
        ))
    }

    /// `read_background_output` 工具——`try_wait` 是非阻塞的查询，不会等它退出。
    pub(crate) fn read_background_output(&mut self, job_id: Uuid) -> Result<String, AppError> {
        let job = self.background_jobs.get_mut(&job_id).ok_or_else(|| {
            AppError::Internal(format!(
                "找不到 job_id={job_id}，可能已经用 stop_background_process 结束并清理过了，或者 id 有误"
            ))
        })?;
        let status = match job.child.try_wait() {
            Ok(Some(status)) => format!("已结束，退出码 {:?}", status.code()),
            Ok(None) => "仍在运行".to_string(),
            Err(e) => format!("查询进程状态失败：{e}"),
        };
        let output = agent_llm::cap_tool_result(job.output.lock().unwrap().clone());
        let output = if output.trim().is_empty() {
            "（还没有任何输出）".to_string()
        } else {
            output
        };
        Ok(format!("命令：{}\n状态：{status}\n累计输出：\n{output}", job.command))
    }

    /// `stop_background_process` 工具。
    pub(crate) fn stop_background_process(&mut self, job_id: Uuid) -> Result<String, AppError> {
        let mut job = self.background_jobs.remove(&job_id).ok_or_else(|| {
            AppError::Internal(format!("找不到 job_id={job_id}，可能已经结束并清理过了，或者 id 有误"))
        })?;
        let _ = job.child.start_kill();
        Ok(format!("已终止 job_id={job_id}（命令：{}）", job.command))
    }

    /// `find_definition` 工具——索引还没建过（这个工作区在这次会话里第一次用到
    /// 这个工具）就现场扫一遍整个工作区建索引，构建结果顺带存回
    /// `symbol_indexes`，供同一个工作区后面的调用（这个会话或者其它命令，比如
    /// 编辑器里的"转到定义"）复用，不用每次都重新扫一遍。
    pub(crate) async fn find_definition(
        &self,
        symbol: &str,
        symbol_indexes: &Arc<RwLock<HashMap<Uuid, SymbolIndex>>>,
    ) -> Result<String, AppError> {
        {
            let indexes = symbol_indexes.read().await;
            if let Some(index) = indexes.get(&self.workspace_id) {
                return Ok(format_symbol_lookup(symbol, index.lookup(symbol)));
            }
        }
        let index = build_index(self.file_ops.as_ref(), &self.workspace_root).await?;
        let result = format_symbol_lookup(symbol, index.lookup(symbol));
        symbol_indexes.write().await.insert(self.workspace_id, index);
        Ok(result)
    }

    /// `task` 工具——委派一个子任务给一个临时的、独立消息历史的子代理去跑一个
    /// 有界的工具循环，只把最终这段文字总结交还给主循环（子代理的探索过程不会
    /// 混进 `self.messages`，这正是这个工具存在的意义：避免大量探索性工具调用
    /// 占满主对话的上下文）。副作用（写文件产生的 Diff、执行的命令）是真实的、
    /// 和主循环共享同一个 `self`——子代理不是完全沙盒隔离，只是"对话历史"这一件
    /// 事是独立的一份临时 `messages`，用完即弃。
    #[allow(clippy::too_many_arguments)]
    async fn run_subagent_task(
        &mut self,
        description: &str,
        prompt: &str,
        client: &reqwest::Client,
        provider: &AiProvider,
        api_key: &Option<String>,
        ssh_pool: &SshConnectionPool,
        agent_pool: &AgentConnectionPool,
        audit: &AuditLogRepo,
        confirms: &CommandConfirmRegistry,
        permission_engine: &PermissionEngine,
        question_confirms: &QuestionRegistry,
        mcp_manager: &McpServerManager,
        symbol_indexes: &Arc<RwLock<HashMap<Uuid, SymbolIndex>>>,
        cancel_token: &tokio_util::sync::CancellationToken,
        app_handle: &AppHandle,
    ) -> Result<String, AppError> {
        const MAX_SUBAGENT_ITERATIONS: usize = 15;
        let mut messages = vec![
            json!({ "role": "system", "content": format!(
                "你是被主任务临时委派的子代理，工作区根目录是 `{}`。只需要完成下面这一个具体子任务，\
                 不需要和用户交互、不需要维护任务清单，完成后用一段简洁但信息完整的文字总结你做了什么、\
                 找到了什么、结论是什么——这段总结会被直接交回给委派你的主任务，它看不到你这边的过程\
                 细节，只看得到这段总结文字，务必把关键信息（具体文件路径、结论、必要的后续建议）写全，\
                 不要只说\"已完成\"。",
                self.workspace_root
            ) }),
            json!({ "role": "user", "content": prompt }),
        ];
        let _ = app_handle.emit(
            "coding:assistant-note",
            json!({ "sessionId": self.id, "text": format!("委派子任务：{description}"), "kind": "status" }),
        );
        // 子代理能用的工具集和主循环按同一个模式（Plan/Build）过滤，只是再减去
        // task 自己（防止无限递归委派）和 question/todo_write（那两个是面向
        // 用户的顶层交互通道，子代理没有直接对话用户的路径）。
        let mut tools = tools_for_mode(self.mode);
        if let Some(arr) = tools.as_array_mut() {
            arr.retain(|t| {
                !matches!(
                    t["function"]["name"].as_str().unwrap_or(""),
                    "task" | "question" | "todo_write"
                )
            });
        }
        for _ in 0..MAX_SUBAGENT_ITERATIONS {
            // 子代理复用主循环传下来的同一个 `cancel_token`——用户点"停止"时，
            // 主循环那次检查要等这次 `execute_tool`（也就是整个子任务）返回才能
            // 生效，所以子任务自己的循环也要在每轮迭代开头做同样的检查，不然
            // 委派一个子任务期间点"停止"会完全没有反应，直到最多 15 轮子任务
            // 自然跑完。
            if cancel_token.is_cancelled() {
                return Err(AppError::Internal("已停止：用户取消了当前对话轮次".to_string()));
            }
            let round = match agent_llm::call_llm_once(
                client,
                provider,
                api_key,
                &messages,
                Some(&tools),
                app_handle,
                self.id,
                "coding",
                cancel_token,
            )
            .await
            {
                Ok(r) => r,
                // 和主循环用同一句文案（见 `send_message` 里的用法）——前端按这句
                // 固定文本识别"用户主动停止"，不当成真正的错误展示。
                Err(agent_llm::LlmCallError::Cancelled) => {
                    return Err(AppError::Internal("已停止：用户取消了当前对话轮次".to_string()))
                }
                Err(agent_llm::LlmCallError::RequestFailed(detail)) => {
                    return Err(AppError::Connection(detail))
                }
                Err(agent_llm::LlmCallError::ParseFailed(e)) => {
                    return Err(AppError::Internal(format!("子任务解析响应失败: {e}")))
                }
            };
            let message = round.message;
            let tool_calls = round.tool_calls;
            if tool_calls.is_empty() {
                let text = message["content"].as_str().unwrap_or("").to_string();
                return Ok(format!("[子任务「{description}」完成]\n{text}"));
            }
            messages.push(message);
            for call in &tool_calls {
                let call_id = call["id"].as_str().unwrap_or_default().to_string();
                let fn_name = call["function"]["name"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                let fn_args = call["function"]["arguments"].as_str().unwrap_or("{}");
                let call_result = tools::parse_tool_call(&fn_name, fn_args);
                let result_text = agent_llm::cap_tool_result(match call_result {
                    Ok(c) => Box::pin(self.execute_tool(
                        c,
                        ssh_pool,
                        agent_pool,
                        audit,
                        confirms,
                        permission_engine,
                        question_confirms,
                        mcp_manager,
                        client,
                        provider,
                        api_key,
                        symbol_indexes,
                        cancel_token,
                        app_handle,
                    ))
                    .await
                    .unwrap_or_else(|e| format!("工具执行出错：{e}")),
                    Err(e) => format!("工具调用参数解析失败：{e}"),
                });
                messages.push(json!({ "role": "tool", "tool_call_id": call_id, "content": result_text }));
            }
        }
        Ok(format!(
            "[子任务「{description}」在 {MAX_SUBAGENT_ITERATIONS} 轮内没有给出最终结论，已提前收尾；\
             如果任务确实很大，建议拆成更小的子任务分别委派]"
        ))
    }
}

fn format_symbol_lookup(symbol: &str, locations: Vec<crate::symbols::SymbolLocation>) -> String {
    if locations.is_empty() {
        format!(
            "没有找到符号 `{symbol}` 的定义——索引是基于正则的轻量扫描，不理解语言语义，多行签名/\
             宏生成的定义/罕见写法可能漏掉；符号名如果没写错，改用 search_files 按关键词搜索。"
        )
    } else {
        let lines: Vec<String> = locations
            .iter()
            .map(|loc| format!("{}:{} ({})", loc.path, loc.line, loc.kind))
            .collect();
        format!("找到 {} 处候选定义：\n{}", locations.len(), lines.join("\n"))
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
/// `gate_command` 的判定结果——`Blocked` 携带的文本直接就是要交回给模型的工具
/// 结果（黑名单拦截/权限规则拒绝/用户点了拒绝，三种情况文案不同，调用方不需要
/// 关心具体是哪一种，原样透出即可）。
pub(crate) enum GateOutcome {
    Blocked(String),
    Allowed,
}

/// 把"这条命令能不能执行"的判定逻辑（黑名单 → 权限规则 → 白名单自动放行/
/// 弹窗确认）从"判定完了真的去跑它"里拆出来——`run_command`（跑完等结果）和
/// `run_command_background`（起了就不等）需要完全相同的判定逻辑，但后续动作
/// 不一样，不应该把"要不要执行"和"怎么执行"耦合在同一个函数里被迫复制粘贴
/// 一遍判定代码。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn gate_command(
    session_id: Uuid,
    target: &CodingTarget,
    auto_allow_readonly: bool,
    full_auto: bool,
    command: &str,
    audit: &AuditLogRepo,
    confirms: &CommandConfirmRegistry,
    permission_engine: &PermissionEngine,
    app_handle: &AppHandle,
) -> GateOutcome {
    let target_label = target_label_for(target);
    let is_windows_target = matches!(target, CodingTarget::Agent { .. });

    if guard::is_blacklisted(command, is_windows_target) {
        audit.record(session_id, &target_label, command, "blocked", None);
        let _ = app_handle.emit(
            "coding:command-blocked",
            json!({ "sessionId": session_id, "command": command }),
        );
        return GateOutcome::Blocked(format!(
            "已拦截高危命令：{command}，如需执行请前往终端模块手动操作"
        ));
    }

    // 权限规则引擎是叠加在黑名单之上、白名单之外的一层（REQUIREMENTS.md §3.7）：
    // 命中 `deny` 直接拒绝；命中 `allow` 直接放行（跳过确认弹窗，但仍然写审计
    // 日志，`outcome` 标成 `auto-allow-rule` 以区分"用户点了一次仅本次允许"）；
    // 命中 `ask` 或者压根没有匹配规则，都落回原有的白名单/确认弹窗逻辑，不改变
    // 现有行为。
    match permission_engine.decide("run_command", command) {
        Some(Decision::Deny) => {
            audit.record(session_id, &target_label, command, "rejected", None);
            return GateOutcome::Blocked(format!("已按权限规则拒绝执行：{command}"));
        }
        Some(Decision::Allow) => {
            audit.record(session_id, &target_label, command, "auto-allow-rule", None);
            return GateOutcome::Allowed;
        }
        _ => {}
    }

    let allowed = if full_auto || (auto_allow_readonly && guard::is_whitelisted(command)) {
        true
    } else {
        let (request_id, rx) = confirms.register(session_id).await;
        let is_remote = matches!(
            target,
            CodingTarget::Remote { .. } | CodingTarget::Agent { .. }
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
        rx.await.unwrap_or(false)
    };

    if !allowed {
        audit.record(session_id, &target_label, command, "rejected", None);
        return GateOutcome::Blocked(format!("用户拒绝执行命令：{command}"));
    }

    GateOutcome::Allowed
}

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
    match gate_command(
        session_id,
        target,
        auto_allow_readonly,
        full_auto,
        command,
        audit,
        confirms,
        permission_engine,
        app_handle,
    )
    .await
    {
        GateOutcome::Blocked(message) => {
            return Ok(CommandExecutionResult {
                output: message,
                exit_code: Some(126),
            });
        }
        GateOutcome::Allowed => {}
    }

    let target_label = target_label_for(target);
    let output = run_target_command(target, cwd, env, command, ssh_pool, agent_pool).await?;
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
            let output = run_local_ai_command_output(command, cwd, env).await?;
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
        "run_command" | "run_command_background" => args["command"]
            .as_str()
            .map(|s| s.chars().take(80).collect()),
        "multi_edit" => args["path"].as_str().map(str::to_string),
        "git_status" | "git_diff" => Some(
            args["path"]
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| "整个仓库".to_string()),
        ),
        "git_commit" => args["message"].as_str().map(|s| s.chars().take(80).collect()),
        "read_background_output" | "stop_background_process" => {
            args["job_id"].as_str().map(str::to_string)
        }
        "find_definition" => args["symbol"].as_str().map(str::to_string),
        "task" => args["description"].as_str().map(str::to_string),
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
        "multi_edit" => format!("为 `{target}` 准备多处代码修改"),
        "git_status" => format!("查看 {target} 的 Git 状态"),
        "git_diff" => format!("查看 {target} 未暂存的改动"),
        "git_commit" => format!("提交改动：{target}"),
        "run_command_background" => format!("后台启动 `{target}`"),
        "find_definition" => format!("查找 `{target}` 的定义"),
        "task" => format!("委派子任务：{target}"),
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
            // git_status/git_diff 只读不改动仓库，find_definition 只读符号索引，
            // 同一档。task 委派子任务时内部会再调用一次 `tools_for_mode(self.mode)`
            // 决定子代理自己能用的工具集，Plan 模式下子代理自然也只会拿到这份
            // 只读工具列表，不会绕过 Plan 模式"不碰文件系统"的边界。
            const READ_ONLY: &[&str] = &[
                "read_file",
                "list_directory",
                "search_files",
                "web_search",
                "glob",
                "todo_write",
                "skill",
                "question",
                "git_status",
                "git_diff",
                "find_definition",
                "task",
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
///
/// 2026-09 用户明确要求"就算超预算，也应该循环处理，不是报错"：附件（PDF/
/// 文本文件）全量拼进去的估算 token 数如果明显超出这个 Provider 的预算
/// （`agent_llm::context_budget`——和 `call_llm_once` 发请求前那道硬性拦截用的
/// 是同一个数字，不能各算各的），就不再直接塞全文，改成分窗口、逐窗口用一次
/// 轻量 LLM 调用提取"和这次问题相关的内容"（`condense_attachment_text`），拼起来
/// 代替原文——是"自动处理"而不是"报错让用户自己去缩小/拆分"。附件明显在预算
/// 内时还是走原来的全文直塞（更快、更完整，没必要为了"可能超预算"多打一堆用
/// 不上的请求）。这也是为什么这个函数从原来的同步函数改成了 `async fn`——分窗口
/// 提取本身要发 HTTP 请求。
pub(crate) async fn build_user_message_content(
    user_text: &str,
    attachments: &[ChatAttachment],
    client: &reqwest::Client,
    provider: &AiProvider,
    api_key: &Option<String>,
    app_handle: &AppHandle,
    session_id: Uuid,
) -> serde_json::Value {
    if attachments.is_empty() {
        return json!(user_text);
    }

    struct ResolvedAttachment {
        label: String,
        text: String,
    }
    let mut resolved: Vec<ResolvedAttachment> = Vec::new();
    for attachment in attachments {
        match attachment {
            ChatAttachment::File { name, content } => {
                resolved.push(ResolvedAttachment { label: name.clone(), text: content.clone() });
            }
            ChatAttachment::Pdf { name, data_base64 } => {
                let text = extract_pdf_text_raw(data_base64)
                    .unwrap_or_else(|e| format!("[PDF 「{name}」解析失败：{e}——可能是加密/损坏/格式不受支持的 PDF]"));
                resolved.push(ResolvedAttachment { label: name.clone(), text });
            }
            ChatAttachment::Image { .. } => {}
        }
    }

    // 预算算法和 `agent_llm::context_budget` 保持一致（同一个数字）；只给附件
    // 本身留一半预算——剩下的要留给 system 提示词/工具 schema/这句话本身/
    // 模型的回答空间，全部吃满反而更容易在别的地方再触发一次同一个预算保护。
    let budget = agent_llm::context_budget(provider);
    let attachment_budget = budget / 2;
    let total_estimate: usize = resolved.iter().map(|r| agent_llm::estimate_tokens(r.text.len())).sum();
    let fair_share = attachment_budget / resolved.len().max(1);

    let mut text = user_text.to_string();
    if total_estimate <= attachment_budget {
        for r in &resolved {
            text.push_str(&format!("\n\n--- 附件文件: {} ---\n{}", r.label, r.text));
        }
    } else {
        for r in &resolved {
            let contribution = if agent_llm::estimate_tokens(r.text.len()) > fair_share {
                condense_attachment_text(client, provider, api_key, user_text, &r.label, &r.text, app_handle, session_id)
                    .await
            } else {
                r.text.clone()
            };
            text.push_str(&format!("\n\n--- 附件文件: {} ---\n{}", r.label, contribution));
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

/// 原始提取文本超过这个字符数就先硬截断再分窗口——防止一份异常巨大的 PDF
/// （几十万字，比如整本扫描书籍的 OCR 文字层）被切成成百上千个窗口、打出
/// 成百上千次 LLM 请求，那不是"自动分窗口处理"想要的效果，是另一种失控。
/// 200 万字符（约 66 万 token 估算）留了足够大的余量给正常的大文档。
const MAX_PDF_RAW_CHARS: usize = 2_000_000;

/// 解码 base64 + 用 `pdf_extract` 抽取纯文本，不做任何截断/预算判断——那是
/// `build_user_message_content` 的职责，这里只负责"这份 PDF 里到底写了什么字"。
/// `pdf_extract::extract_text_from_mem` 是同步、CPU 密集的调用（正则式的 PDF
/// 内容流解析，不是网络 I/O），没有用 `spawn_blocking` 挪到阻塞线程池——对
/// 单机单用户的桌面应用、一次几页到几十页的 PDF 来说这次阻塞可以接受，真遇到
/// 大到明显卡顿的 PDF 属于另一个问题（考虑给 `pdf_extract` 相关调用整体挪到
/// 阻塞线程池），不是这次"分窗口"要解决的。
fn extract_pdf_text_raw(data_base64: &str) -> Result<String, String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data_base64)
        .map_err(|e| format!("附件解码失败：{e}"))?;
    let text = pdf_extract::extract_text_from_mem(&bytes).map_err(|e| e.to_string())?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("没有可提取的文本内容——可能是纯扫描图片版，没有文字层".to_string());
    }
    if trimmed.chars().count() > MAX_PDF_RAW_CHARS {
        let truncated: String = trimmed.chars().take(MAX_PDF_RAW_CHARS).collect();
        return Ok(format!("{truncated}\n...[原文档异常巨大，已先截断到前 {MAX_PDF_RAW_CHARS} 字符，超出部分没有参与后续处理]"));
    }
    Ok(trimmed.to_string())
}

/// 单个"窗口"的目标 token 数——不是硬上限，是"这一个窗口大概能安全用掉多少
/// 预算"的粗略目标：选一个比较保守的固定值（不是按 provider 实际预算动态算），
/// 因为分窗口这一步本身就是为了避免"卡线"，选择"够安全"比"刚好卡线"更重要。
const ATTACHMENT_WINDOW_TARGET_TOKENS: usize = 6_000;
/// 单个窗口提取请求的超时——和 `CONTEXT_SUMMARY_TIMEOUT`（历史轮次摘要）同一个
/// 量级、同一个"锦上添花不能卡死主流程"的原则：超时/失败就把这个窗口原文
/// （截断一部分）直接保留，不阻塞整个流程。
const ATTACHMENT_WINDOW_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// 按行边界把一段长文本切成若干个目标大小在 `target_tokens` 附近的窗口——
/// 尽量不在一行中间切断（PDF 抽取出来的文本经常一行就是一段话/一行表格，
/// 从中间切开会让单个窗口看起来语义不完整）。目标字符数用
/// `agent_llm::estimate_tokens` 反过来算的近似值（3 字节 ≈ 1 token），不是
/// 精确的分词器切分，这里只需要"大致均匀"，不需要精确到字。
fn split_into_windows(text: &str, target_tokens: usize) -> Vec<String> {
    let target_chars = target_tokens.saturating_mul(3).max(1);
    if text.chars().count() <= target_chars {
        return vec![text.to_string()];
    }
    let mut windows = Vec::new();
    let mut current = String::new();
    let mut current_chars = 0usize;
    for line in text.split_inclusive('\n') {
        let line_chars = line.chars().count();
        if current_chars + line_chars > target_chars && !current.is_empty() {
            windows.push(std::mem::take(&mut current));
            current_chars = 0;
        }
        current.push_str(line);
        current_chars += line_chars;
    }
    if !current.is_empty() {
        windows.push(current);
    }
    windows
}

/// 附件内容太大、直接塞进消息正文会让请求超出 Provider 的上下文预算时，自动
/// 按窗口拆分、逐窗口用一次轻量 LLM 调用提取"和用户这次问题相关的内容"，再把
/// 所有窗口的提取结果拼起来代替原始全文。故意不用 `agent_llm::call_llm_once`
/// （那一套完整的双协议归一化/429 重试机制）——和 `summarize_dropped_turns`
/// （历史轮次摘要）同一个理由：这是主流程之外的辅助步骤，摘要/提取本身失败
/// 不能变成新的卡死点，宁可退化成"保留原文的一部分"也不要因为这一步反复重试
/// 拖住整个请求。
///
/// 每个窗口互相看不到彼此的内容——窗口之间没有"记忆"，这是简化设计的代价：
/// 如果答案需要综合多个窗口的信息才能得出（比如"数一下全文一共出现了几次
/// X"），拆分之后可能丢失跨窗口的关联。真正需要完整看一遍全文做统计类任务的
/// 场景，这个机制帮不上忙，需要用户换个更聚焦的问题，或者直接把相关章节复制
/// 粘贴进对话而不是整份文档当附件。
#[allow(clippy::too_many_arguments)]
async fn condense_attachment_text(
    client: &reqwest::Client,
    provider: &AiProvider,
    api_key: &Option<String>,
    user_text: &str,
    attachment_name: &str,
    full_text: &str,
    app_handle: &AppHandle,
    session_id: Uuid,
) -> String {
    let windows = split_into_windows(full_text, ATTACHMENT_WINDOW_TARGET_TOKENS);
    if windows.len() <= 1 {
        return full_text.to_string();
    }
    let _ = app_handle.emit(
        "coding:assistant-note",
        json!({
            "sessionId": session_id,
            "text": format!(
                "附件「{attachment_name}」内容较多（约 {} 字），超出了当前 Provider 的上下文预算，\
                 已自动拆成 {} 个窗口分别提取与你的问题相关的部分…",
                full_text.chars().count(),
                windows.len()
            ),
            "kind": "status"
        }),
    );
    let window_count = windows.len();
    let mut parts: Vec<String> = Vec::with_capacity(window_count);
    for (i, window) in windows.iter().enumerate() {
        match summarize_attachment_window(client, provider, api_key, user_text, attachment_name, i + 1, window_count, window)
            .await
        {
            Some(extracted) if extracted.contains("此部分与问题无关") => {}
            Some(extracted) => parts.push(format!("[第 {}/{window_count} 部分]\n{extracted}", i + 1)),
            None => parts.push(format!(
                "[第 {}/{window_count} 部分：自动提取超时/失败，保留原文前 2000 字符]\n{}",
                i + 1,
                window.chars().take(2000).collect::<String>()
            )),
        }
    }
    if parts.is_empty() {
        return format!(
            "（附件《{attachment_name}》内容较多，已自动分成 {window_count} 个窗口检查，但没有找到和\
             当前问题明显相关的内容——如果确定文档里有相关信息，换一个更具体的问题描述再试，或者直接\
             告诉我大概在文档的哪个部分）"
        );
    }
    format!(
        "（原文档较长，超出了当前上下文预算，已自动拆成 {window_count} 个窗口、分别提取与你的问题\
         「{user_text}」相关的内容，以下是各窗口提取结果的合并，不是原文全文）\n\n{}",
        parts.join("\n\n")
    )
}

/// `condense_attachment_text` 的单个窗口——不走 `agent_llm::call_llm_once`
/// 的理由见调用方文档；这里假设 Provider 是 chat/completions 协议，和
/// `summarize_dropped_turns` 同样的简化（这个 codebase 里"锦上添花"的辅助
/// LLM 调用目前都是这个简化，暂不支持 Responses-only 协议的 Provider 走这条
/// 路径——那种 Provider 会退化成"保留原文片段"而不是提取失败崩溃）。
#[allow(clippy::too_many_arguments)]
async fn summarize_attachment_window(
    client: &reqwest::Client,
    provider: &AiProvider,
    api_key: &Option<String>,
    user_text: &str,
    attachment_name: &str,
    window_index: usize,
    window_count: usize,
    window_text: &str,
) -> Option<String> {
    let url = format!("{}/chat/completions", provider.api_base.trim_end_matches('/'));
    let body = json!({
        "model": provider.model,
        "messages": [
            {
                "role": "system",
                "content": format!(
                    "你在帮用户从一份过长的附件文档《{attachment_name}》里挑出和他的问题相关的内容——\
                     这份文档太大，已经被自动切成 {window_count} 个窗口分别处理，你现在看到的是第 \
                     {window_index}/{window_count} 部分，看不到其它部分。用户的问题是：『{user_text}』。\
                     请从这部分内容里提取和这个问题直接相关的信息（具体的数据、表名/字段名/接口定义/\
                     结论等，能保留原文措辞就保留，不要过度概括丢细节），无关的内容直接跳过不用提。\
                     如果这部分内容整体上和问题没有关系，只回复\"（此部分与问题无关）\"这一句，不要\
                     硬凑内容。直接输出提取结果，不要复述这段说明、不要说\"好的\"\"以下是\"这类开场白。"
                )
            },
            { "role": "user", "content": window_text }
        ]
    });
    let mut req = client.post(&url).json(&body);
    if let Some(key) = api_key {
        req = req.bearer_auth(key);
    }
    let resp = match tokio::time::timeout(ATTACHMENT_WINDOW_TIMEOUT, req.send()).await {
        Ok(Ok(resp)) => resp,
        _ => return None,
    };
    let body: serde_json::Value = match resp.json().await {
        Ok(body) => body,
        Err(_) => return None,
    };
    body["choices"][0]["message"]["content"]
        .as_str()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
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

/// `command` 的第一个空白分隔 token 如果是 `powershell.exe`/`pwsh.exe`（不管带不带
/// 完整路径），说明它已经是"可执行文件 + 自带完整参数"的形式，返回
/// `(可执行文件, 剩余参数原文)` 供直接 spawn，绕开 `cmd.exe /C` 对内嵌换行符的截断
/// 问题（见下面 `run_local_command_output_with_env` 的注释）；否则返回 `None`，
/// 调用方走 `cmd.exe /C` 那条路——不假设 exe 路径本身会被引号包住这种更复杂的情况。
#[cfg(target_os = "windows")]
fn split_direct_shell_invocation(command: &str) -> Option<(&str, &str)> {
    let trimmed = command.trim_start();
    let end = trimmed.find(char::is_whitespace).unwrap_or(trimmed.len());
    let exe = &trimmed[..end];
    let exe_lower = exe.trim_matches('"').to_ascii_lowercase();
    if exe_lower.ends_with("powershell.exe") || exe_lower.ends_with("pwsh.exe") {
        Some((exe, trimmed[end..].trim_start()))
    } else {
        None
    }
}

/// `command` 的第一个空白分隔 token 如果是 `cmd`/`cmd.exe`，说明调用方显式要用
/// cmd.exe 当解释器——和上面 `split_direct_shell_invocation` 识别
/// `powershell.exe`/`pwsh.exe` 是同一个思路，供 `run_local_ai_command_output`
/// 判断"要不要按默认的 PowerShell 兜底，还是尊重这条命令自己指定的解释器"。
#[cfg(target_os = "windows")]
fn is_explicit_cmd_invocation(command: &str) -> bool {
    let trimmed = command.trim_start();
    let end = trimmed.find(char::is_whitespace).unwrap_or(trimmed.len());
    let exe = trimmed[..end].trim_matches('"').to_ascii_lowercase();
    exe == "cmd" || exe.ends_with("cmd.exe")
}

#[cfg(target_os = "windows")]
fn windows_command_for(command: &str, default_to_powershell: bool) -> tokio::process::Command {
    // 用 `raw_arg` 而不是 `arg`——`command` 这个字符串如果已经是调用方按目标
    // shell 语法手动转义、拼好的完整命令行，走普通的 `.arg()` 的话 Rust 会把它
    // 当成"一个不透明的参数值"再转义一遍——它自己已经加好的引号会被当成需要
    // 保护的特殊字符处理，两层转义叠在一起，引号数量直接对不上。2026-09 用户
    // 实测复现：模型报 PowerShell "字符数缺少终止符" 解析错误，根因就是这次双重
    // 转义，不是编码问题（两个不同模型都被这条报错误导去查 GBK/UTF-8 编码，
    // 两个都猜错了方向）。`raw_arg` 原样把字符串接到命令行末尾，不做任何额外
    // 转义/加引号，`command` 已经是一条完整、转义好的命令行，正需要这种"照抄
    // 不动"的语义。
    //
    // 2026-09 第二轮诊断：`cmd.exe /C` 修好双重转义之后，命令仍然"执行
    // 成功（exit 0）但输出完全是空的"——用 PowerShell 直接复现
    // `cmd /c 'powershell -Command "a
    // cmd.exe /C 不能可靠处理嵌入换行，直接使用 PowerShell 进程。
    // 保留命令文本，避免额外 shell 转义。
    // 其余行为与原有命令执行逻辑一致。
    //
    // 吞。改用 `ProcessStartInfo` 直接起 `powershell.exe`（不经 cmd.exe）复现，
    // 同一个带换行的参数原样保留、输出正常拿到。如果 `command` 已经是"完整
    // 可执行文件路径 + 自带参数"的形式（`split_direct_shell_invocation` 识别），
    // 就直接起这个可执行文件，走 Windows 标准命令行解析（`CommandLineToArgvW`，
    // PowerShell 自己也是这套），双引号内的换行能原样保留，不会被截断。
    if let Some((exe, rest)) = split_direct_shell_invocation(command) {
        let mut c = tokio::process::Command::new(exe);
        if !rest.is_empty() {
            c.raw_arg(rest);
        }
        c.creation_flags(CREATE_NO_WINDOW);
        c
    } else if default_to_powershell && !is_explicit_cmd_invocation(command) {
        // `run_command` 工具专用分支（见 `run_local_ai_command_output` 的文档）：
        // 命令没有显式指定解释器时，按系统提示词的承诺默认当 PowerShell 脚本
        // 执行，而不是 cmd.exe。用普通 `.arg()`（不是 `raw_arg`）——这里的
        // `command` 是模型写的一整段原始 PowerShell 脚本文本，不是已经按 shell
        // 语法转义好的命令行片段，需要 Rust 标准库的自动转义把它安全地包成
        // `-Command` 的单个参数值；`.arg()` 的转义算法和 Win32
        // `CommandLineToArgvW` 配套设计，能保证 PowerShell 收到的内容和原始
        // 文本一字不差，不会被重复转义（和上面 `raw_arg` 那段注释描述的"已经
        // 转义好的命令行"是完全不同的场景，不冲突）。
        let mut c = tokio::process::Command::new("powershell.exe");
        c.arg("-NoProfile").arg("-Command").arg(command);
        c.creation_flags(CREATE_NO_WINDOW);
        c
    } else {
        // 任意 shell 命令字符串（比如 `git status`、显式 `cmd.exe /c dir`），
        // 或者 `default_to_powershell` 为 false 的调用方（`git_ops.rs`，命令
        // 按 POSIX 规则手动转义拼好，见 `run_local_command_output_with_env`
        // 的文档），仍然用 cmd.exe 当解释器，保持原来的包法。
        let mut c = tokio::process::Command::new("cmd");
        c.raw_arg("/C").raw_arg(command);
        c.creation_flags(CREATE_NO_WINDOW);
        c
    }
}

#[cfg(not(target_os = "windows"))]
fn unix_command_for(command: &str) -> tokio::process::Command {
    let mut c = tokio::process::Command::new("sh");
    c.arg("-c").arg(command);
    c
}

/// 本地命令统一的执行超时——`run_command`/`git_ops.rs` 内部用到的 git 命令都过
/// 这条路径。2026-09 之前完全没有超时保护：命令一旦意外卡住（比如命令行被解析
/// 成了带交互提示的形式、或者其实是一个不会自己退出的常驻进程），会让当前这
/// 一轮工具调用一直挂着，只能用户手动点"停止"整个对话轮次，白白损失这一轮已经
/// 收集到的其它进度。10 分钟覆盖绝大多数编译/安装依赖/跑测试套件的场景；真正
/// 需要长期挂起的进程（开发服务器、watch 模式）应该用 `run_command_background`
/// 而不是这条路径。
const LOCAL_COMMAND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

async fn run_prepared_command(
    mut cmd: tokio::process::Command,
    cwd: &str,
    env: &HashMap<String, String>,
) -> Result<std::process::Output, AppError> {
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
    // 超时后需要连带终止子进程，不能只是放弃等待——`kill_on_drop(true)` 让
    // tokio 在下面这个 future 因为超时被取消、内部持有的 `Child` 随之被丢弃时
    // 自动发送终止信号，不需要自己再拿一份 `Child` 句柄手动处理 kill。
    cmd.kill_on_drop(true);
    match tokio::time::timeout(LOCAL_COMMAND_TIMEOUT, cmd.output()).await {
        Ok(result) => Ok(result?),
        Err(_) => Err(AppError::Internal(format!(
            "命令执行超过 {} 秒未结束，已强制终止。如果这是一个需要长期挂起的进程（比如开发服务器、\
             watch 模式），改用 run_command_background 工具而不是 run_command。",
            LOCAL_COMMAND_TIMEOUT.as_secs()
        ))),
    }
}

/// `git_ops.rs` 走的这条路径——命令是按 POSIX 规则手动转义拼好的
/// （`crate::log::remote::shell_quote`），默认解释器保持 cmd.exe 不变，不跟
/// `run_local_ai_command_output` 一起换成 PowerShell（那样会破坏这层转义假设）。
pub(super) async fn run_local_command_output_with_env(
    command: &str,
    cwd: &str,
    env: &HashMap<String, String>,
) -> Result<std::process::Output, AppError> {
    #[cfg(target_os = "windows")]
    let cmd = windows_command_for(command, false);
    #[cfg(not(target_os = "windows"))]
    let cmd = unix_command_for(command);
    run_prepared_command(cmd, cwd, env).await
}

/// `run_command` 工具专用入口——和上面 `run_local_command_output_with_env`
/// 唯一的区别是：命令没有显式指定解释器时，默认按 PowerShell 执行，而不是
/// cmd.exe。系统提示词（本文件 498/511-512 行）明确告诉模型"本地命令行执行
/// 环境是 PowerShell"，但原来的实际默认解释器其实是 cmd.exe——提示词的承诺
/// 和实际执行环境不一致（2026-09 真实复现：模型写 `Set-Location '...';
/// python ...` 这类 PowerShell 专属语法，落进 cmd.exe 直接报"'Set-Location'
/// 不是内部或外部命令"，连续失败两轮才靠自己加 `powershell -Command` 前缀
/// 试出来，白白浪费好几轮工具调用/token）。这里改成默认真的按提示词说的走
/// PowerShell，模型不再需要自己猜测/补前缀；命令显式点名 `powershell.exe`/
/// `pwsh.exe`/`cmd`/`cmd.exe` 时仍然尊重模型自己的选择。
pub(super) async fn run_local_ai_command_output(
    command: &str,
    cwd: &str,
    env: &HashMap<String, String>,
) -> Result<std::process::Output, AppError> {
    #[cfg(target_os = "windows")]
    let cmd = windows_command_for(command, true);
    #[cfg(not(target_os = "windows"))]
    let cmd = unix_command_for(command);
    run_prepared_command(cmd, cwd, env).await
}






