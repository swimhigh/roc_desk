use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

use super::diff::{generate_diff, DiffLine};
use super::git_ops;
use super::guard;
use super::permission::{Decision, PermissionEngine};
use super::skills::{self, SkillMeta};
use super::tools::{self, TodoItem, ToolCall};
use super::webfetch;
use super::{CommandConfirmRegistry, QuestionRegistry};
use crate::ai::{search_web_results, AiProviderManager};
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
    Remote { connection_id: Uuid, host_label: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeStatus {
    Pending,
    Applied,
    Rejected,
    Undone,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileChange {
    pub id: Uuid,
    pub path: String,
    pub old_content: String,
    pub new_content: String,
    pub diff: Vec<DiffLine>,
    pub status: ChangeStatus,
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
    /// 工作区根目录是否在一个 Git 仓库里——`coding_start` 命令构造完 session 后
    /// 探测一次填进来（探测本身要跑一条命令，`new()` 保持同步，不在这里做）。
    /// 不是仓库时前端应该把"自动提交"开关直接 disable 掉。
    pub git_repo: bool,
    /// 用户是否要求"每次 Accept 一条变更就自动 git add + commit"（DESIGN.md §3.8.2）。
    pub auto_git_commit: bool,
    pub changes: Vec<FileChange>,
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
    undo_stack: Vec<FileChange>,
    messages: Vec<serde_json::Value>,
    file_ops: Arc<dyn FileOps>,
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

impl CodingSession {
    pub fn new(
        workspace_id: Uuid,
        workspace_root: String,
        target: CodingTarget,
        provider_id: Uuid,
        file_ops: Arc<dyn FileOps>,
    ) -> Self {
        let target_desc = match &target {
            CodingTarget::Local => "本地工作区".to_string(),
            CodingTarget::Remote { host_label, .. } => format!("远程主机 {host_label}"),
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
            id: Uuid::new_v4(),
            workspace_id,
            workspace_root,
            target,
            mode: CodingMode::Plan,
            provider_id,
            auto_allow_readonly: false,
            git_repo: false,
            auto_git_commit: false,
            changes: Vec::new(),
            todos: Vec::new(),
            skills: Vec::new(),
            project_memory_loaded: Vec::new(),
            undo_stack: Vec::new(),
            messages: vec![json!({ "role": "system", "content": system_prompt })],
            file_ops,
        }
    }

    /// 读取工作区根目录下的 `AGENTS.md`/`CLAUDE.md`（两个都找就都注入，各自标注
    /// 来源，谁在前谁优先级更高不做主观判断，交给模型自己权衡），拼成一条 system
    /// 消息追加到初始系统提示词之后。cap 到 32KB——项目约定文件正常不会写这么长，
    /// 真写了这么长说明多半是用户不小心把别的内容也塞了进去，截断比整份塞给模型
    /// 更安全（避免一次性吃掉大量上下文预算）。找不到文件不是错误，静默跳过。
    pub async fn load_project_memory(&mut self) {
        const MAX_BYTES: usize = 32 * 1024;
        for filename in ["AGENTS.md", "CLAUDE.md"] {
            let Ok(content) = self.file_ops.read_file(filename).await else { continue };
            let truncated: String = content.text.chars().take(MAX_BYTES).collect();
            self.messages.push(json!({
                "role": "system",
                "content": format!("以下是项目 {filename} 中记录的约定，请在完成任务时遵守：\n\n{truncated}")
            }));
            self.project_memory_loaded.push(filename.to_string());
        }
    }

    /// 扫描 `.rock_desk/skills/*/SKILL.md` 并把"可用技能：name — description"
    /// 列表追加进系统提示词，模型据此决定要不要通过 `skill` 工具加载某个技能的
    /// 详细正文。没有技能目录时不追加任何内容（`skills` 为空数组，静默跳过）。
    pub async fn load_skills(&mut self) {
        self.skills = skills::discover_skills(self.file_ops.as_ref(), &self.workspace_root).await;
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

    fn target_label(&self) -> String {
        match &self.target {
            CodingTarget::Local => "本地".to_string(),
            CodingTarget::Remote { host_label, .. } => host_label.clone(),
        }
    }

    /// 会话里已经存在的、尚未撤销的最新提议内容（见结构体文档）。
    fn pending_content_for(&self, path: &str) -> Option<String> {
        self.changes
            .iter()
            .rev()
            .find(|c| c.path == path && c.status != ChangeStatus::Rejected && c.status != ChangeStatus::Undone)
            .map(|c| c.new_content.clone())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn send_message(
        &mut self,
        user_text: &str,
        providers: &AiProviderManager,
        ssh_pool: &SshConnectionPool,
        audit: &AuditLogRepo,
        confirms: &CommandConfirmRegistry,
        permission_rules: &PermissionRulesRepo,
        question_confirms: &QuestionRegistry,
        mcp_manager: &McpServerManager,
        app_handle: &AppHandle,
    ) -> Result<String, AppError> {
        self.messages.push(json!({ "role": "user", "content": user_text }));
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
                    let Ok(client) = mcp_manager.get_or_connect(server.id).await else { continue };
                    for tool in &client.tools {
                        let qualified_name = format!("mcp__{}__{}", sanitize_tool_name(&server.name), sanitize_tool_name(&tool.name));
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

        let provider = providers
            .get(self.provider_id)?
            .ok_or_else(|| AppError::NotFound(format!("ai provider not found: {}", self.provider_id)))?;
        let api_key = providers.resolve_api_key(&provider).await?;
        let client = reqwest::Client::new();

        for i in 0..MAX_TOOL_ITERATIONS {
            let url = format!("{}/chat/completions", provider.api_base.trim_end_matches('/'));

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
            let resp = req.send().await?;
            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(AppError::Connection(format!("HTTP {status}: {body}")));
            }
            let body: serde_json::Value = resp.json().await?;
            let message = body["choices"][0]["message"].clone();
            let tool_calls = message["tool_calls"].as_array().cloned().unwrap_or_default();

            if tool_calls.is_empty() {
                let text = message["content"].as_str().unwrap_or("").to_string();
                self.messages.push(json!({ "role": "assistant", "content": text }));
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
            if message["content"].as_str().is_none_or(|text| text.trim().is_empty()) {
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
                let fn_name = call["function"]["name"].as_str().unwrap_or_default().to_string();
                let fn_args = call["function"]["arguments"].as_str().unwrap_or("{}");
                // 只显示工具名之前完全看不出模型在反复对同一个文件/同一个词调用，还是
                // 在正常地一个一个探索不同文件——这次真实复现的"看起来在循环"就是靠
                // 加上这个才能一眼确认（见 REQUIREMENTS.md §3.7 的记录）。
                let detail = tool_call_detail(&fn_name, fn_args);

                let _ = app_handle.emit(
                    "coding:tool-call-start",
                    json!({ "sessionId": self.id, "tool": fn_name, "detail": detail }),
                );

                let call_result = if let Some((server_id, tool_name)) = mcp_tool_index.get(&fn_name) {
                    let arguments: serde_json::Value = serde_json::from_str(fn_args).unwrap_or_else(|_| json!({}));
                    Ok(ToolCall::Mcp { server_id: *server_id, tool_name: tool_name.clone(), arguments })
                } else {
                    tools::parse_tool_call(&fn_name, fn_args)
                };
                let result_text = match call_result {
                    Ok(call) => self
                        .execute_tool(call, ssh_pool, audit, confirms, &permission_engine, question_confirms, mcp_manager, app_handle)
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
        audit: &AuditLogRepo,
        confirms: &CommandConfirmRegistry,
        permission_engine: &PermissionEngine,
        question_confirms: &QuestionRegistry,
        mcp_manager: &McpServerManager,
        app_handle: &AppHandle,
    ) -> Result<String, AppError> {
        match call {
            ToolCall::ReadFile { path } => {
                if let Some(content) = self.pending_content_for(&path) {
                    return Ok(content);
                }
                Ok(self.file_ops.read_file(&path).await?.text)
            }
            ToolCall::ListDirectory { path } => {
                let entries = self.file_ops.list_dir(&path).await?;
                Ok(serde_json::to_string(&entries).unwrap_or_default())
            }
            ToolCall::SearchFiles { pattern, path } => self.search_files(&pattern, &path, ssh_pool).await,
            ToolCall::WebSearch { query } => {
                search_web_results(&reqwest::Client::new(), &query).await
            }
            ToolCall::WriteFile { path, content } => self.stage_change(&path, content, app_handle).await,
            ToolCall::EditFile { path, old_text, new_text } => {
                let original = match self.pending_content_for(&path) {
                    Some(c) => c,
                    None => self.file_ops.read_file(&path).await?.text,
                };
                if !original.contains(&old_text) {
                    return Err(AppError::Internal(format!(
                        "edit_file 失败：在 {path} 中没有找到匹配的 old_text，请先用 read_file 确认现有内容"
                    )));
                }
                let updated = original.replacen(&old_text, &new_text, 1);
                self.stage_change(&path, updated, app_handle).await
            }
            ToolCall::RunCommand { command } => {
                self.run_command_gated(&command, ssh_pool, audit, confirms, permission_engine, app_handle).await
            }
            ToolCall::Glob { pattern, path } => self.search_glob(&pattern, &path).await,
            ToolCall::WebFetch { url } => self.webfetch_gated(&url, permission_engine, app_handle).await,
            ToolCall::TodoWrite { todos } => {
                self.todos = todos;
                let _ = app_handle.emit("coding:todo-update", json!({ "sessionId": self.id, "todos": &self.todos }));
                Ok(format!("任务清单已更新，共 {} 项", self.todos.len()))
            }
            ToolCall::Question { question, options } => {
                self.ask_user(&question, options, question_confirms, app_handle).await
            }
            ToolCall::Skill { name } => skills::load_skill_body(self.file_ops.as_ref(), &self.skills, &name).await,
            ToolCall::Mcp { server_id, tool_name, arguments } => {
                self.call_mcp_tool(server_id, &tool_name, arguments, permission_engine, confirms, mcp_manager, app_handle).await
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
        let options = SearchOptions { case_sensitive: false, whole_word: false, use_regex: false };
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
        Ok(if matches.is_empty() { "没有匹配结果".to_string() } else { matches.join("\n") })
    }

    /// `webfetch` 工具：默认策略是"允许"（和已经不设防的 `web_search` 一致，
    /// SSRF 防护在 `coding/webfetch.rs::fetch_url` 里做），但用户可以在权限规则
    /// 里为 `webfetch` 加一条 `deny` 规则（比如公司网络策略不希望 AI 访问外部
    /// 网页），命中即拒绝，不会真的发出请求。
    async fn webfetch_gated(&self, url: &str, permission_engine: &PermissionEngine, _app_handle: &AppHandle) -> Result<String, AppError> {
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
        server_id: Uuid,
        tool_name: &str,
        arguments: serde_json::Value,
        permission_engine: &PermissionEngine,
        confirms: &CommandConfirmRegistry,
        mcp_manager: &McpServerManager,
        app_handle: &AppHandle,
    ) -> Result<String, AppError> {
        let servers = mcp_manager.list()?;
        let server = servers.iter().find(|s| s.id == server_id).ok_or_else(|| AppError::NotFound("MCP 服务器不存在".into()))?;
        // MCP 维度的规则用固定的 `tool = "mcp"` + 可通配的 `pattern`（比如
        // `filesystem:*` 放行某个服务器的全部工具、`filesystem:read_file` 只放行
        // 单个工具），和 `run_command` 的 `tool = "run_command"` + 通配命令文本是
        // 同一种设计，不是"每个工具单独存一行精确匹配"。
        let match_key = format!("{}:{}", server.name, tool_name);

        let allowed = match permission_engine.decide("mcp", &match_key) {
            Some(Decision::Allow) => true,
            Some(Decision::Deny) => false,
            _ => {
                let (request_id, rx) = confirms.register().await;
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
            return Ok(format!("用户拒绝执行 MCP 工具调用：{}.{}", server.name, tool_name));
        }

        let client = mcp_manager.get_or_connect(server_id).await?;
        client.call_tool(tool_name, arguments).await
    }

    async fn search_files(&self, pattern: &str, path: &str, ssh_pool: &SshConnectionPool) -> Result<String, AppError> {
        match &self.target {
            CodingTarget::Local => {
                let results = tools::search_files_local(std::path::Path::new(path), pattern, 50);
                Ok(if results.is_empty() { "没有匹配结果".to_string() } else { results.join("\n") })
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
        }
    }

    async fn stage_change(&mut self, path: &str, new_content: String, app_handle: &AppHandle) -> Result<String, AppError> {
        let old_content = match self.pending_content_for(path) {
            Some(c) => c,
            None => self.file_ops.read_file(path).await.map(|f| f.text).unwrap_or_default(),
        };
        let diff = generate_diff(&old_content, &new_content);
        let id = Uuid::new_v4();
        let change = FileChange {
            id,
            path: path.to_string(),
            old_content,
            new_content,
            diff,
            status: ChangeStatus::Pending,
        };
        let _ = app_handle.emit("coding:file-change", json!({ "sessionId": self.id, "change": &change }));
        self.changes.push(change);
        Ok(format!("已为 {path} 生成变更（id={id}），已在界面展示 Diff，等待用户 Accept 后才会真正写入磁盘。"))
    }

    async fn run_command_gated(
        &mut self,
        command: &str,
        ssh_pool: &SshConnectionPool,
        audit: &AuditLogRepo,
        confirms: &CommandConfirmRegistry,
        permission_engine: &PermissionEngine,
        app_handle: &AppHandle,
    ) -> Result<String, AppError> {
        let target_label = self.target_label();

        if guard::is_blacklisted(command) {
            audit.record(self.id, &target_label, command, "blocked", None);
            let _ = app_handle.emit("coding:command-blocked", json!({ "sessionId": self.id, "command": command }));
            return Ok(format!("已拦截高危命令：{command}，如需执行请前往终端模块手动操作"));
        }

        // 权限规则引擎是叠加在黑名单之上、白名单之外的一层（REQUIREMENTS.md §3.7）：
        // 命中 `deny` 直接拒绝；命中 `allow` 直接放行（跳过确认弹窗，但仍然写审计
        // 日志，`outcome` 标成 `auto-allow-rule` 以区分"用户点了一次仅本次允许"）；
        // 命中 `ask` 或者压根没有匹配规则，都落回原有的白名单/确认弹窗逻辑，不改变
        // 现有行为。
        match permission_engine.decide("run_command", command) {
            Some(Decision::Deny) => {
                audit.record(self.id, &target_label, command, "rejected", None);
                return Ok(format!("已按权限规则拒绝执行：{command}"));
            }
            Some(Decision::Allow) => {
                audit.record(self.id, &target_label, command, "auto-allow-rule", None);
                let output = match &self.target {
                    CodingTarget::Local => run_local_command(command, &self.workspace_root).await?,
                    CodingTarget::Remote { connection_id, .. } => {
                        let session = ssh_pool.get_or_connect(*connection_id).await?;
                        session.exec(command).await?
                    }
                };
                let summary: String = output.chars().take(2000).collect();
                audit.record(self.id, &target_label, command, "executed", Some(&summary));
                return Ok(output.chars().take(4000).collect());
            }
            _ => {}
        }

        let allowed = if self.auto_allow_readonly && guard::is_whitelisted(command) {
            true
        } else {
            let (request_id, rx) = confirms.register().await;
            let is_remote = matches!(&self.target, CodingTarget::Remote { .. });
            let _ = app_handle.emit(
                "coding:command-confirm-request",
                json!({
                    "sessionId": self.id,
                    "requestId": request_id,
                    "command": command,
                    "host": if is_remote { Some(target_label.clone()) } else { None },
                    "kind": "command",
                }),
            );
            rx.await.unwrap_or(false)
        };

        if !allowed {
            audit.record(self.id, &target_label, command, "rejected", None);
            return Ok(format!("用户拒绝执行命令：{command}"));
        }

        let output = match &self.target {
            CodingTarget::Local => run_local_command(command, &self.workspace_root).await?,
            CodingTarget::Remote { connection_id, .. } => {
                let session = ssh_pool.get_or_connect(*connection_id).await?;
                session.exec(command).await?
            }
        };

        let summary: String = output.chars().take(2000).collect();
        audit.record(self.id, &target_label, command, "executed", Some(&summary));
        Ok(output.chars().take(4000).collect())
    }

    /// 用户点击 FileChangeCard 的"应用"：这时才真正写盘（expected_mtime 传 None
    /// 强制覆盖——用户已经在 UI 里显式确认过这份 Diff，不再需要走 mtime 冲突检测
    /// 那一套，那是给"和别的编辑动作并发"场景设计的）。
    ///
    /// 开启"自动 Git 提交"时顺带 commit 这一个文件——提交失败（没配置 git 身份、
    /// 目标机器没装 git 等）不应该让"应用变更"这个更核心的动作跟着失败，只把结果
    /// 通过事件告诉前端，用户自己看得懂那段 git 输出。
    pub async fn accept_change(
        &mut self,
        change_id: Uuid,
        ssh_pool: &SshConnectionPool,
        app_handle: &AppHandle,
    ) -> Result<(), AppError> {
        let change = self
            .changes
            .iter_mut()
            .find(|c| c.id == change_id)
            .ok_or_else(|| AppError::NotFound(format!("change not found: {change_id}")))?;
        if change.status != ChangeStatus::Pending {
            return Err(AppError::Conflict(format!("change {change_id} is not pending")));
        }
        self.file_ops.write_file(&change.path, &change.new_content, None).await?;
        change.status = ChangeStatus::Applied;
        let path = change.path.clone();
        self.undo_stack.clear();

        if self.auto_git_commit && self.git_repo {
            let message = format!("AI 编程助手：修改 {path}");
            let result = git_ops::commit_file(&self.target, &self.workspace_root, &path, &message, ssh_pool).await;
            let output = match result {
                Ok(out) => out,
                Err(e) => format!("Git 提交失败：{e}"),
            };
            let _ = app_handle.emit(
                "coding:git-commit-result",
                json!({ "sessionId": self.id, "path": path, "output": output }),
            );
        }

        Ok(())
    }

    pub fn reject_change(&mut self, change_id: Uuid) -> Result<(), AppError> {
        let change = self
            .changes
            .iter_mut()
            .find(|c| c.id == change_id)
            .ok_or_else(|| AppError::NotFound(format!("change not found: {change_id}")))?;
        if change.status != ChangeStatus::Pending {
            return Err(AppError::Conflict(format!("change {change_id} is not pending")));
        }
        change.status = ChangeStatus::Rejected;
        Ok(())
    }

    pub async fn undo_change(&mut self, change_id: Uuid) -> Result<(), AppError> {
        let change = self
            .changes
            .iter_mut()
            .find(|c| c.id == change_id)
            .ok_or_else(|| AppError::NotFound(format!("change not found: {change_id}")))?;
        if change.status != ChangeStatus::Applied {
            return Err(AppError::Conflict(format!("change {change_id} is not applied")));
        }
        self.file_ops.write_file(&change.path, &change.old_content, None).await?;
        change.status = ChangeStatus::Undone;
        self.undo_stack.push(change.clone());
        Ok(())
    }

    /// 标准编辑器行为：撤销点之后一旦产生新的、真正落盘的修改（`accept_change`），
    /// 原有 redo 分支就失效，否则 redo 可能把已经被覆盖的旧内容写回
    /// （DESIGN.md §3.8.3 骨架代码后的注解）——见 `accept_change` 里的 `undo_stack.clear()`。
    pub async fn redo_change(&mut self) -> Result<Option<Uuid>, AppError> {
        let Some(mut change) = self.undo_stack.pop() else { return Ok(None) };
        self.file_ops.write_file(&change.path, &change.new_content, None).await?;
        change.status = ChangeStatus::Applied;
        let id = change.id;
        if let Some(existing) = self.changes.iter_mut().find(|c| c.id == id) {
            *existing = change;
        }
        Ok(Some(id))
    }
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
        "run_command" => args["command"].as_str().map(|s| s.chars().take(80).collect()),
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
                "read_file", "list_directory", "search_files", "web_search",
                "glob", "todo_write", "skill", "question",
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

/// OpenAI function-calling 的工具名要求匹配 `^[a-zA-Z0-9_-]+$`，MCP 服务器/工具名
/// 是用户自己起的、可能带中文或空格——拼进 `mcp__<server>__<tool>` 之前先替换掉
/// 非法字符，避免一整条请求因为工具名不合法被 Provider 直接拒绝。
fn sanitize_tool_name(name: &str) -> String {
    name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect()
}

pub(super) async fn run_local_command(command: &str, cwd: &str) -> Result<String, AppError> {
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = tokio::process::Command::new("cmd");
        c.arg("/C").arg(command);
        c
    };
    #[cfg(not(target_os = "windows"))]
    let mut cmd = {
        let mut c = tokio::process::Command::new("sh");
        c.arg("-c").arg(command);
        c
    };
    cmd.current_dir(cwd);
    let output = cmd.output().await?;
    Ok(format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    ))
}
