use std::collections::HashMap;
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
use crate::ai::{search_web_results, AiProvider, AiProviderManager};
use crate::db::repo::audit_log_repo::AuditLogRepo;
use crate::db::repo::permission_rules_repo::PermissionRulesRepo;
use crate::error::AppError;
use crate::fsops::{search_stream, FileOps, SearchMode, SearchOptions};
use crate::mcp::McpServerManager;
use crate::ssh::SshConnectionPool;

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

/// 2026-09 用户反馈："GPT 模型经常报错（Selected model is at capacity / 503），
/// 希望有重试机制"——这几个状态码/关键词基本都是"服务端临时顶不住"，不是请求本身
/// 有问题，重试大概率能成。3 次（不算首次请求，总共最多打 4 次）配合指数退避，
/// 既给瞬时过载留出恢复时间，又不会让用户等太久。
const MAX_HTTP_RETRIES: u32 = 3;
/// 重试间隔的基数（秒），第 N 次重试等 `RETRY_BASE_DELAY_SECS * 2^(N-1)`——
/// 1s/2s/4s，指数退避，不是每次固定等一样久。
const RETRY_BASE_DELAY_SECS: u64 = 1;

/// HTTP 状态码层面判断"值得重试"：429（限流）、5xx（服务端错误/网关/过载）——
/// 4xx 里其余的（401 认证失败、400 参数错误）重试没有意义，问题不会自己消失。
fn is_retryable_http_status(status: reqwest::StatusCode) -> bool {
    status.as_u16() == 429 || status.is_server_error()
}

/// 有些 Provider 即使返回的 HTTP 状态码看着正常（甚至 200），也会在响应体文本里
/// 说"当前过载/请换个模型"这类话（2026-09 用户真实复现的原文就是"Selected model
/// is at capacity. Please try a different model."）——纯看状态码会漏掉这种情况，
/// 所以额外兜底扫一遍响应体文本里的关键词。只做英文关键词匹配，不做多语言穷举，
/// 覆盖不到的极端情况就走"重试次数用完/不可重试"这条路径，不是本质缺陷。
fn is_retryable_error_text(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    ["overloaded", "at capacity", "rate limit", "try again", "temporarily unavailable"]
        .iter()
        .any(|keyword| lower.contains(keyword))
}

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
/// 整个会话发给模型的上下文上限，按 token 估算（见 `estimate_tokens`）而不是原始字符数——
/// 2026-09 复盘：原来直接拿字符数当预算单位，对不同模型的真实上下文窗口没有代表性
/// （尤其中文场景：一个汉字在 UTF-8 里是 3 字节，用字符数算预算会系统性低估实际 token
/// 消耗）。真实复现过一次"HTTP 400 context_too_large"——用户接的 provider 实际能接受
/// 的窗口明显比这里假设的更小，60_000 是收紧后的全局默认值，宁可压缩得频繁一点，也不
/// 要让请求真的超限被 Provider 拒绝（拒绝了这一整轮就白跑，压缩只是多花一次轻量调用）。
/// 不是精确值——真要精确匹配每个 provider 的上下文窗口需要一份 per-provider 配置，
/// 这次先不做。
const MAX_CONTEXT_TOKENS_ESTIMATE: usize = 60_000;
/// 在总量没有触顶时，仍只保留最近这些用户轮次，避免长时间会话缓慢挤占内存。
const MAX_CONTEXT_USER_TURNS: usize = 8;
/// 标记 `self.messages[1]`（如果存在）是"早前对话摘要"消息，不是真实的历史内容——
/// `limit_context` 用这个前缀识别"要不要新建一条摘要消息，还是往已有的追加"。
const CONTEXT_SUMMARY_PREFIX: &str = "【早前对话摘要】";
/// 摘要消息自己的字符上限——长会话里如果不停追加摘要，摘要本身也会无限增长，超过这个
/// 阈值就丢弃最早的一截摘要片段（不再摘要一次摘要，避免过度设计）。
const MAX_CONTEXT_SUMMARY_CHARS: usize = 4_000;
/// 摘要请求本身的超时——摘要是锦上添花，不能变成新的卡死点，超时/失败就直接退回硬删。
const CONTEXT_SUMMARY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// 把字节数换算成一个粗略的 token 估算——3 字节 ≈ 1 token，向上取整。codex-core 自己
/// （`vendor/codex/codex-rs/core/src/context_manager/history.rs`）在没有接入真实分词器
/// 时用的是 4 字节/token，但那是面向英文场景的经验值；roc_desk 的对话大量是中文（系统
/// 提示词、用户输入、模型回复都是），一个汉字在 UTF-8 里是 3 字节，真实分词器通常给它
/// 1-2 个 token，用 4 字节/token 会系统性低估。这里的预算判断"宁可估多、不要估少"——
/// 估计偏高只会让压缩触发得早一点（多花一次摘要请求），估计偏低才会让真正超限的请求
/// 放过去被 Provider 拒绝（2026-09 真实复现过）。
fn estimate_tokens(byte_len: usize) -> usize {
    byte_len.div_ceil(3)
}

/// 把即将从 `self.messages` 里删掉的一批轮次压成一段 2-4 句的摘要——发一次不带
/// `tools` 字段的轻量请求，不走 `MAX_TOOL_ITERATIONS` 预算（这是 `limit_context`
/// 自己触发的独立请求，不是工具循环的一部分）。任何失败（网络错误、超时、响应里
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
                "content": "你是对话摘要助手。用 2-4 句中文总结下面这段编程助手对话做了什么、\
                             涉及哪些文件/结论，给后续对话保留必要背景。不要客套，不要逐句复述，\
                             只给结论性摘要。"
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
        let mut user_turns = self
            .messages
            .iter()
            .filter(|message| message["role"].as_str() == Some("user"))
            .count();
        loop {
            let size_tokens = estimate_tokens(
                self.messages
                    .iter()
                    .map(|message| message.to_string().len())
                    .sum::<usize>(),
            );
            if size_tokens <= MAX_CONTEXT_TOKENS_ESTIMATE && user_turns <= MAX_CONTEXT_USER_TURNS {
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
                    if let Some(summary) =
                        summarize_dropped_turns(&dropped, client, provider, api_key).await
                    {
                        self.append_context_summary(&summary);
                    }
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
        if let Some(summary) = summarize_dropped_turns(&dropped, client, provider, api_key).await {
            self.append_context_summary(&summary);
        }
        true
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
        // 没了地交替 search_files/read_file，一直不给结论。后端有 FORCE_CONCLUDE_LAST_N
        // 硬兜底（见 send_message），但那是最后一道防线；这里在提示词里先明确给出
        // "工具调用总量有限、该收敛就收敛"的预期，减少真的撞到硬限制的次数。
        let system_prompt = format!(
            "你是集成在 roc_desk 桌面工具里的 AI 编程助手，当前绑定的工作区根目录是 `{workspace_root}`（{target_desc}）。\
             run_command 执行的命令必须匹配上面说明的 shell 语法，不要凭空假设是另一种系统/shell。\
             你可以用提供的工具读写文件、搜索代码、访问互联网、执行命令。涉及“今天/最新/新闻/外部事实”的问题必须先调用 web_search，\
             不要凭模型记忆回答。write_file/edit_file 产生的改动不会立即生效，\
             而是生成 Diff 交给用户确认，所以你可以放心连续提出多个改动，不需要等待每一步都被确认才能继续推理。\
             run_command 有安全限制：破坏性命令会被直接拦截，其余命令需要用户在弹窗里确认才会真正执行。\
             工具调用总次数是有限的（几十次量级），不是无限预算：面对\"分析整个项目\"这类开放式大任务时，\
             优先用 search_files/list_directory 快速定位最相关的一小批文件（不需要每个文件都读一遍），\
             读完这些就给出结论；不要为了追求\"看得更全\"而无休止地继续搜索/读取，觉得信息已经够回答用户的\
             问题时就直接总结，而不是再多看几个文件。\
             \n\n读文件内容优先用 read_file 工具，不要用 run_command 里 sed/cat/head 这类命令去手动\
             分段读——read_file 会自动按合理长度截断并在截断处提示，不需要你自己为了\"怕超长\"而每次\
             只读几十行、切成一大堆零碎调用（这样反而更浪费工具调用次数）；单次工具结果本身有长度保护，\
             可以放心一次性多读一些内容（比如几百行），不用过度保守。\
             \n\n用户的话如果有明显歧义、可能对应两种差别很大的意图（比如一句简短的\"继续\"\"报错了\"\
             \"确认\"，既可能是在接着上一个没答完的问题往下走，也可能是在描述一件跟上文完全无关的新情况），\
             不要凭猜测直接选一种理解就展开长篇回答——调用 question 工具，把你想到的几种理解列成\
             options 让用户选一下，等用户选完再按确定下来的理解继续，比自己猜错了、答非所问、用户还要\
             再纠正一轮更省事。只有在意图已经足够清楚、只是细节需要你自己判断的情况下，才不需要用这个\
             工具反复确认——不要把它用成什么都要问一遍的过度谨慎。",
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
            messages: vec![json!({ "role": "system", "content": system_prompt })],
            file_ops,
            current_turn_id: Uuid::new_v4(),
            awaiting_confirmation_turn: None,
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

        self.messages.push(
            json!({ "role": "user", "content": user_message_content(user_text, attachments) }),
        );
        // 不在这里裁剪——下面工具循环的第一轮（`for i in 0..MAX_TOOL_ITERATIONS`）一
        // 进去就会调用 `limit_context`，那时候 provider/api_key/client 才解析出来，
        // 摘要请求需要用到它们。
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
        // 这一整轮（一条用户消息到最终给出结论）可能要跑好几次工具循环迭代，
        // 每次迭代都是一次独立的 API 请求——累加起来才是用户真正关心的"这轮
        // 对话一共花了多少 token"，单次请求的消耗只在过程中当进度参考。
        let mut turn_prompt_tokens: i64 = 0;
        let mut turn_completion_tokens: i64 = 0;
        let mut turn_total_tokens: i64 = 0;
        let session_id = self.id;
        // 每个"这一轮结束"的出口（正常给出结论 / 工具调用次数耗尽）都要发一次
        // 汇总，闭包只是为了不在好几个 return 点前重复写同一段 emit 代码；
        // `total == 0` 时不发（provider 没回传 usage，或者这一轮压根没请求成功
        // 过），避免时间线里堆一堆没有信息量的"0 tokens"提示。
        let emit_turn_usage_summary = |prompt: i64, completion: i64, total: i64| {
            if total > 0 {
                let _ = app_handle.emit(
                    "coding:token-usage-summary",
                    json!({
                        "sessionId": session_id,
                        "promptTokens": prompt,
                        "completionTokens": completion,
                        "totalTokens": total,
                    }),
                );
            }
        };

        for i in 0..MAX_TOOL_ITERATIONS {
            if cancel_token.is_cancelled() {
                return Err(AppError::Internal(
                    "已停止：用户取消了当前对话轮次".to_string(),
                ));
            }
            self.limit_context(&client, &provider, &api_key).await;
            let is_responses = provider.wire_api == "responses";
            let url = if is_responses {
                format!("{}/responses", provider.api_base.trim_end_matches('/'))
            } else {
                format!("{}/chat/completions", provider.api_base.trim_end_matches('/'))
            };

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

            let mut tools = tools_for_mode(self.mode);
            if let Some(arr) = tools.as_array_mut() {
                arr.extend(mcp_tool_defs.iter().cloned());
            }
            // Responses API 的 `input`/`instructions` 跟 chat/completions 的
            // `messages` 是完全不同的请求形状（见 `messages_to_responses_input` 的
            // 文档注释），但 `self.messages` 内部存储/`limit_context`/摘要机制/
            // 历史持久化全都只认 chat/completions 这一种形状——转换只发生在这里，
            // 发完请求之后响应也会被归一化回同一种形状（见下面 `message` 的构造），
            // 其余代码完全不需要关心当前 provider 是哪种协议。
            let mut body = if is_responses {
                let (instructions, input) = messages_to_responses_input(&self.messages);
                let mut body = json!({
                    "model": provider.model,
                    "instructions": instructions,
                    "input": input,
                    "stream": false,
                });
                if !force_conclude {
                    body["tools"] = chat_tools_to_responses_tools(&tools);
                    body["tool_choice"] = json!("auto");
                }
                body
            } else {
                let mut body = json!({ "model": provider.model, "messages": self.messages });
                if !force_conclude {
                    body["tools"] = tools;
                    body["tool_choice"] = json!("auto");
                }
                body
            };
            // 对齐 Codex `config.toml` 的 `model_reasoning_effort`——gpt-5/o 系列
            // 这类推理模型从客户端调节内部推理力度，两种协议的传参位置不一样：
            // Responses API 是嵌套的 `reasoning.effort`，chat/completions 兼容层
            // 是顶层 `reasoning_effort`（不是所有 OpenAI 兼容中转都认，用户自己
            // 决定要不要在 Provider 里配这个）。留空（`None`）就完全不带这个
            // 字段，维持之前"交给服务端默认值"的行为。
            if let Some(effort) = &provider.reasoning_effort {
                if is_responses {
                    body["reasoning"] = json!({ "effort": effort });
                } else {
                    body["reasoning_effort"] = json!(effort);
                }
            }
            // "停止"按钮：单次请求本身就是这个循环里最容易卡很久的一步（网络慢/
            // 服务端限流排队），所以在这一步单独包一层取消——`req.send()` 败给
            // 取消信号时直接丢弃即可，`reqwest` 的请求 future 被 drop 时会中止
            // 底层连接，不会有资源泄漏。
            // 2026-09 真实复现：请求失败（HTTP 错误/网络错误/响应解析失败）之前
            // 直接 `return Err(...)`，`self.messages` 里什么痕迹都不留——用户看到
            // 报错后回一句"报错了，请继续回答上个问题"，下一轮模型看到的对话历史
            // 是"提了问题→查了一堆东西→用户突然说报错了"，完全没有信号能判断
            // "报错"指的是"你上次没答完"还是"我自己的程序运行出错了"，往往会理解
            // 成后者，答非所问。这里在每个失败分支返回之前，先往历史里补一条
            // system 消息把"刚才这次请求失败了"这件事讲清楚，下一轮就有明确依据。
            //
            // 2026-09 用户反馈："GPT 模型经常报错（Selected model is at capacity /
            // 503），希望有重试机制"——之前是完全不重试的，一次瞬时过载就直接判
            // 这一轮失败。503/502/504/429 这几个状态码、以及响应体里带
            // "overloaded"/"at capacity"/"rate limit" 这类字样，基本都是"服务端
            // 临时顶不住，过会儿再试大概率能成"，不是请求本身有问题，值得自动重试；
            // 其余 4xx（比如 401 认证失败、400 参数错误）重试没有意义，直接失败。
            // 重试之间用指数退避（1s/2s/4s）等一下，不是立刻重打，给服务端一点喘息
            // 空间；等待过程中同样响应取消。
            let mut retry_count = 0u32;
            let (resp, body_text) = loop {
                let mut req = client.post(&url).json(&body);
                if let Some(key) = &api_key {
                    req = req.bearer_auth(key);
                }
                let send_result = tokio::select! {
                    biased;
                    result = req.send() => result,
                    _ = cancel_token.cancelled() => {
                        return Err(AppError::Internal(
                            "已停止：用户取消了当前对话轮次".to_string(),
                        ));
                    }
                };
                let (status, body_text) = match send_result {
                    Ok(resp) if resp.status().is_success() => break (Some(resp), String::new()),
                    Ok(resp) => {
                        let status = resp.status();
                        (Some(status), resp.text().await.unwrap_or_default())
                    }
                    Err(e) => (None, e.to_string()),
                };
                let retryable = match status {
                    Some(status) => is_retryable_http_status(status),
                    None => true, // 网络层面的错误（连不上/超时）也值得重试
                } || is_retryable_error_text(&body_text);
                if retryable && retry_count < MAX_HTTP_RETRIES {
                    retry_count += 1;
                    let _ = app_handle.emit(
                        "coding:assistant-note",
                        json!({
                            "sessionId": self.id,
                            "text": format!(
                                "接口响应异常（{}），{} 秒后自动重试第 {}/{} 次……",
                                status.map(|s| s.to_string()).unwrap_or_else(|| body_text.clone()),
                                RETRY_BASE_DELAY_SECS * (1u64 << (retry_count - 1)),
                                retry_count,
                                MAX_HTTP_RETRIES
                            ),
                            "kind": "status"
                        }),
                    );
                    let delay = std::time::Duration::from_secs(
                        RETRY_BASE_DELAY_SECS * (1u64 << (retry_count - 1)),
                    );
                    tokio::select! {
                        _ = tokio::time::sleep(delay) => {}
                        _ = cancel_token.cancelled() => {
                            return Err(AppError::Internal(
                                "已停止：用户取消了当前对话轮次".to_string(),
                            ));
                        }
                    }
                    continue;
                }
                break (None, format!("{}: {body_text}", status.map(|s| s.to_string()).unwrap_or_else(|| "连接失败".to_string())));
            };
            let Some(resp) = resp else {
                self.messages.push(json!({
                    "role": "system",
                    "content": format!(
                        "[系统提示] 上一次请求失败（{body_text}），重试 {retry_count} 次后仍未成功，\
                         没有得到正常回复，上面那个问题还没有被回答。如果用户接下来说\"继续\"\
                         \"报错了请继续回答\"之类的话，是指继续回答上面被打断的那个问题，不是\
                         在描述一个新的、跟这次请求无关的错误。"
                    )
                }));
                return Err(AppError::Connection(body_text));
            };
            body = match resp.json().await {
                Ok(b) => b,
                Err(e) => {
                    self.messages.push(json!({
                        "role": "system",
                        "content": format!(
                            "[系统提示] 上一次请求返回的内容解析失败（{e}），没有得到正常回复，\
                             上面那个问题还没有被回答。如果用户接下来说\"继续\"之类的话，是指\
                             继续回答上面被打断的那个问题。"
                        )
                    }));
                    return Err(AppError::Internal(format!("解析响应失败: {e}")));
                }
            };
            if let Some((prompt, completion, total)) = extract_token_usage(&body, &provider.wire_api)
            {
                turn_prompt_tokens += prompt;
                turn_completion_tokens += completion;
                turn_total_tokens += total;
                let _ = app_handle.emit(
                    "coding:token-usage",
                    json!({
                        "sessionId": self.id,
                        "promptTokens": prompt,
                        "completionTokens": completion,
                        "totalTokens": total,
                    }),
                );
            }
            let message = if is_responses {
                let (text, tool_calls) = parse_responses_output(&body);
                json!({ "role": "assistant", "content": text, "tool_calls": tool_calls })
            } else {
                body["choices"][0]["message"].clone()
            };
            let tool_calls = message["tool_calls"]
                .as_array()
                .cloned()
                .unwrap_or_default();

            if tool_calls.is_empty() {
                let text = message["content"].as_str().unwrap_or("").to_string();
                self.messages
                    .push(json!({ "role": "assistant", "content": text }));
                emit_turn_usage_summary(turn_prompt_tokens, turn_completion_tokens, turn_total_tokens);
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
                let result_text = cap_tool_result(match call_result {
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

        emit_turn_usage_summary(turn_prompt_tokens, turn_completion_tokens, turn_total_tokens);
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
            audit.record(session_id, &target_label, command, "auto-allow-rule", None);
            let output =
                run_target_command(target, cwd, env, command, ssh_pool, agent_pool).await?;
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
        return Ok(CommandExecutionResult {
            output: format!("用户拒绝执行命令：{command}"),
            exit_code: Some(126),
        });
    }

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

/// 把 chat/completions 的工具定义 `[{type:"function",function:{name,description,
/// parameters}}]` 拍平成 Responses API 要求的形状
/// `[{type:"function",name,description,parameters,strict:false}]`——两边字段名
/// 一样，区别只是 `function` 这层嵌套被拆掉，多一个 `strict` 字段（本次实现固定
/// 传 `false`，不做严格 JSON Schema 校验）。
fn chat_tools_to_responses_tools(tools: &serde_json::Value) -> serde_json::Value {
    let Some(arr) = tools.as_array() else {
        return json!([]);
    };
    json!(
        arr.iter()
            .map(|tool| {
                let f = &tool["function"];
                json!({
                    "type": "function",
                    "name": f["name"],
                    "description": f["description"],
                    "parameters": f["parameters"],
                    "strict": false,
                })
            })
            .collect::<Vec<_>>()
    )
}

/// 把内部统一存储的 `self.messages`（chat/completions 形状）转换成 Responses API
/// 的 `(instructions, input)`——`system` 角色的内容抽出来拼成 `instructions`；
/// `user`/`assistant` 文本消息转成 `message` item；`assistant` 消息里的
/// `tool_calls` 数组每个转成一个独立的 `function_call` item（Responses API 把它
/// 们当平级 item，不像 chat/completions 嵌在一条 assistant 消息里）；`tool` 消息
/// 转成 `function_call_output` item，`call_id` 复用已有的 `tool_call_id`。
fn messages_to_responses_input(
    messages: &[serde_json::Value],
) -> (String, Vec<serde_json::Value>) {
    let mut instructions = String::new();
    let mut input = Vec::new();
    for message in messages {
        match message["role"].as_str() {
            Some("system") => {
                if let Some(text) = message["content"].as_str() {
                    if !instructions.is_empty() {
                        instructions.push_str("\n\n");
                    }
                    instructions.push_str(text);
                }
            }
            Some("user") => {
                input.push(json!({
                    "type": "message",
                    "role": "user",
                    "content": content_to_responses_input_parts(&message["content"]),
                }));
            }
            Some("assistant") => {
                if let Some(text) = message["content"].as_str() {
                    if !text.trim().is_empty() {
                        input.push(json!({
                            "type": "message",
                            "role": "assistant",
                            "content": [{ "type": "output_text", "text": text }],
                        }));
                    }
                }
                if let Some(calls) = message["tool_calls"].as_array() {
                    for call in calls {
                        input.push(json!({
                            "type": "function_call",
                            "call_id": call["id"].as_str().unwrap_or_default(),
                            "name": call["function"]["name"].as_str().unwrap_or_default(),
                            "arguments": call["function"]["arguments"].as_str().unwrap_or("{}"),
                        }));
                    }
                }
            }
            Some("tool") => {
                input.push(json!({
                    "type": "function_call_output",
                    "call_id": message["tool_call_id"].as_str().unwrap_or_default(),
                    "output": message["content"].as_str().unwrap_or_default(),
                }));
            }
            _ => {}
        }
    }
    (instructions, input)
}

/// `user_message_content` 生成的要么是纯字符串、要么是 OpenAI 风格的多模态 parts
/// 数组（`{type:"text",text}`/`{type:"image_url",image_url:{url}}`）——转成
/// Responses API 对应的 `input_text`/`input_image` item。
fn content_to_responses_input_parts(content: &serde_json::Value) -> Vec<serde_json::Value> {
    match content {
        serde_json::Value::String(text) => vec![json!({ "type": "input_text", "text": text })],
        serde_json::Value::Array(parts) => parts
            .iter()
            .filter_map(|part| match part["type"].as_str() {
                Some("text") => part["text"]
                    .as_str()
                    .map(|text| json!({ "type": "input_text", "text": text })),
                Some("image_url") => part["image_url"]["url"]
                    .as_str()
                    .map(|url| json!({ "type": "input_image", "image_url": url })),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// 解析 Responses API 的非流式响应体（`{"output":[...], "usage":{...}}`），把
/// `output` 数组归一化成跟 chat/completions 原生 `message` 完全一样的形状——
/// `type=="message"` 的 item 里 `content[].type=="output_text"` 的文本拼起来当
/// 助手回复；`type=="function_call"` 的 item 转成 chat/completions 风格的
/// `tool_calls[i]`（`{id,type:"function",function:{name,arguments}}`）。转换后
/// `self.messages`/`limit_context`/摘要机制完全不需要关心是哪种 wire 协议。
fn parse_responses_output(body: &serde_json::Value) -> (String, Vec<serde_json::Value>) {
    let mut text = String::new();
    let mut tool_calls = Vec::new();
    if let Some(items) = body["output"].as_array() {
        for item in items {
            match item["type"].as_str() {
                Some("message") => {
                    if let Some(parts) = item["content"].as_array() {
                        for part in parts {
                            if part["type"].as_str() == Some("output_text") {
                                if let Some(t) = part["text"].as_str() {
                                    text.push_str(t);
                                }
                            }
                        }
                    }
                }
                Some("function_call") => {
                    tool_calls.push(json!({
                        "id": item["call_id"].as_str().unwrap_or_default(),
                        "type": "function",
                        "function": {
                            "name": item["name"].as_str().unwrap_or_default(),
                            "arguments": item["arguments"].as_str().unwrap_or("{}"),
                        }
                    }));
                }
                _ => {}
            }
        }
    }
    (text, tool_calls)
}

/// 两种协议的 `usage` 字段名不一样（chat/completions 是
/// `prompt_tokens`/`completion_tokens`，Responses API 是
/// `input_tokens`/`output_tokens`），统一抽成同一个形状供前端展示。拿不到就返回
/// `None`——有些 provider 压根不回传 usage，静默跳过，不影响主流程。
fn extract_token_usage(body: &serde_json::Value, wire_api: &str) -> Option<(i64, i64, i64)> {
    let usage = &body["usage"];
    if usage.is_null() {
        return None;
    }
    let (prompt, completion) = if wire_api == "responses" {
        (
            usage["input_tokens"].as_i64()?,
            usage["output_tokens"].as_i64()?,
        )
    } else {
        (
            usage["prompt_tokens"].as_i64()?,
            usage["completion_tokens"].as_i64()?,
        )
    };
    let total = usage["total_tokens"].as_i64().unwrap_or(prompt + completion);
    Some((prompt, completion, total))
}

/// 把用户输入和附件拼成一条 user 消息的 `content`：没有附件时保持纯字符串
/// （和改动前完全一致，不打扰不用附件的既有场景/历史数据格式）；有附件时才
/// 切成 OpenAI 兼容的多模态 parts 数组——文本类附件直接拼进文字正文（模型不需要
/// 支持 vision 也能读），图片作为独立的 `image_url` part（需要模型支持 vision
/// 才"看得到"，不支持的模型会按各家实现忽略或报错，这里不做能力探测，交给用户
/// 自己判断当前 Provider 是否支持）。
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
    // `cmd /c 'powershell -Command "a`nb"'`（`` `n `` 是真实换行符）确认：
    // cmd.exe 的 `/C` 解析器按物理行处理语句边界，双引号包不住内嵌的换行——
    // 换行后面的内容直接被吞掉，不报错、不进 stderr，只是"消失"。CRLF 也一样
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
    Ok(cmd.output().await?)
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
