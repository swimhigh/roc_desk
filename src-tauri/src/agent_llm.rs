//! AI Agent 的多轮工具调用引擎里，跟"具体有哪些工具"完全无关的那一层——
//! 一次带工具调用的 LLM 请求怎么打（chat/completions vs Responses API 两种
//! 协议归一化）、429/5xx 怎么退避重试、usage 怎么统一抽取。原本这套逻辑只
//! 写在 `coding/session.rs` 里，SQL Desktop 需要一个自己的多轮 Agent
//! （`sql/agent`）时没有照抄一遍——两边的"给 LLM 发一轮请求、处理重试、解析
//! 出文本/tool_calls/usage"要做的事情完全一样，只是可用的工具集合不同，
//! 这部分因此被抽出来供两边共用。工具怎么解析、怎么执行（`coding::tools`/
//! `sql::agent::tools`）、每一轮怎么整体推进（要不要摘要压缩上下文、要不要
//! 强制收尾）仍然各自实现——那些部分和"文件/git" vs "SQL 连接"这些领域概念
//! 深度绑定，勉强抽成一个通用接口只会增加一层没有实际收益的间接层。

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

use crate::ai::AiProvider;

/// 2026-09 用户反馈："GPT 模型经常报错（Selected model is at capacity /
/// 503），希望有重试机制"——这几个状态码/关键词基本都是"服务端临时顶不住"，
/// 不是请求本身有问题，重试大概率能成。3 次（不算首次请求，总共最多打 4 次）
/// 配合指数退避，既给瞬时过载留出恢复时间，又不会让用户等太久。
pub const MAX_HTTP_RETRIES: u32 = 3;
/// 重试间隔的基数（秒），第 N 次重试等 `RETRY_BASE_DELAY_SECS * 2^(N-1)`——
/// 1s/2s/4s，指数退避，不是每次固定等一样久。
pub const RETRY_BASE_DELAY_SECS: u64 = 1;
/// 单条工具结果塞进对话历史前的字符数上限，见 `cap_tool_result`。
pub const MAX_TOOL_RESULT_CHARS: usize = 20_000;

/// HTTP 状态码层面判断"值得重试"：429（限流）、5xx（服务端错误/网关/过载）——
/// 4xx 里其余的（401 认证失败、400 参数错误）重试没有意义，问题不会自己消失。
pub fn is_retryable_http_status(status: reqwest::StatusCode) -> bool {
    status.as_u16() == 429 || status.is_server_error()
}

/// 有些 Provider 即使返回的 HTTP 状态码看着正常（甚至 200），也会在响应体
/// 文本里说"当前过载/请换个模型"这类话，纯看状态码会漏掉这种情况，所以额外
/// 兜底扫一遍响应体文本里的关键词。只做英文关键词匹配，不做多语言穷举。
pub fn is_retryable_error_text(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    ["overloaded", "at capacity", "rate limit", "try again", "temporarily unavailable"]
        .iter()
        .any(|keyword| lower.contains(keyword))
}

/// 单条工具结果的字符数上限——不做限制的话，读到一个很大的结果集/文件会在一次
/// 多轮对话里被原样重复重发好几十次，真实复现过因此内存耗尽被系统直接杀掉的
/// 情况（Rust 默认分配器分配失败会直接 abort，不会留下任何崩溃日志）。
pub fn cap_tool_result(text: String) -> String {
    if text.chars().count() <= MAX_TOOL_RESULT_CHARS {
        return text;
    }
    let truncated: String = text.chars().take(MAX_TOOL_RESULT_CHARS).collect();
    format!(
        "{truncated}\n\n[内容过长，已截断到前 {MAX_TOOL_RESULT_CHARS} 字符——需要更多内容时请缩小范围重新获取]"
    )
}

/// 把字节数换算成一个粗略的 token 估算——3 字节 ≈ 1 token，向上取整（中文场景
/// 一个汉字 UTF-8 占 3 字节，真实分词器通常给 1-2 个 token，宁可估多不要估少：
/// 估多只会让上下文压缩触发得早一点，估少才会让真正超限的请求被 Provider 拒绝）。
pub fn estimate_tokens(byte_len: usize) -> usize {
    byte_len.div_ceil(3)
}

fn chat_tools_to_responses_tools(tools: &Value) -> Value {
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

/// 把内部统一存储（chat/completions 形状）的对话历史转换成 Responses API 的
/// `(instructions, input)`——`system` 角色的内容抽出来拼成 `instructions`；
/// `user`/`assistant` 文本消息转成 `message` item；`assistant` 消息里的
/// `tool_calls` 数组每个转成一个独立的 `function_call` item；`tool` 消息转成
/// `function_call_output` item。
pub fn messages_to_responses_input(messages: &[Value]) -> (String, Vec<Value>) {
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
/// 数组，转成 Responses API 对应的 `input_text`/`input_image` item。
fn content_to_responses_input_parts(content: &Value) -> Vec<Value> {
    match content {
        Value::String(text) => vec![json!({ "type": "input_text", "text": text })],
        Value::Array(parts) => parts
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

/// 解析 Responses API 的非流式响应体，归一化成跟 chat/completions 原生
/// `message` 完全一样的形状，调用方之后完全不需要关心是哪种 wire 协议。
pub fn parse_responses_output(body: &Value) -> (String, Vec<Value>) {
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

/// 两种协议的 `usage` 字段名不一样，统一抽成同一个形状。拿不到就返回
/// `None`——有些 provider 压根不回传 usage，静默跳过，不影响主流程。
pub fn extract_token_usage(body: &Value, wire_api: &str) -> Option<(i64, i64, i64)> {
    let usage = &body["usage"];
    if usage.is_null() {
        return None;
    }
    let (prompt, completion) = if wire_api == "responses" {
        (usage["input_tokens"].as_i64()?, usage["output_tokens"].as_i64()?)
    } else {
        (usage["prompt_tokens"].as_i64()?, usage["completion_tokens"].as_i64()?)
    };
    let total = usage["total_tokens"].as_i64().unwrap_or(prompt + completion);
    Some((prompt, completion, total))
}

/// 一次成功的 LLM 往返：归一化后的 assistant 消息（可以直接 push 进对话历史）、
/// 里面的 `tool_calls`（已经展开成数组方便调用方遍历）、这次请求的 token 用量。
pub struct LlmRoundResult {
    pub message: Value,
    pub tool_calls: Vec<Value>,
    pub usage: Option<(i64, i64, i64)>,
}

/// `call_llm_once` 的失败结果——细分成三种，是因为调用方（`CodingSession`/
/// `SqlAgentSession`）在这三种情况下对"要不要往自己的对话历史里补一条系统
/// 提示消息"的处理不一样，细分之后调用方不需要用字符串匹配错误信息来猜是
/// 哪一种失败。
pub enum LlmCallError {
    /// 用户点了"停止"——调用方不需要往对话历史里补任何东西，下一条用户消息
    /// 会正常接上当前上下文继续。
    Cancelled,
    /// 重试用完仍未成功（网络错误，或服务端持续返回错误状态码）。
    RequestFailed(String),
    /// HTTP 请求本身成功，但响应体解析失败。
    ParseFailed(String),
}

/// 请求失败重试用完之后，往对话历史里补的说明——不讲清楚这一点，用户下一句
/// "报错了请继续回答"很容易被模型理解成"用户程序运行出错了"而不是"你上次没
/// 答完"，答非所问。
pub fn request_failed_message(detail: &str) -> Value {
    json!({
        "role": "system",
        "content": format!(
            "[系统提示] 上一次请求失败（{detail}），重试后仍未成功，没有得到正常回复，\
             上面那个问题还没有被回答。如果用户接下来说\"继续\"\"报错了请继续回答\"之类的话，\
             是指继续回答上面被打断的那个问题，不是在描述一个新的、跟这次请求无关的错误。"
        )
    })
}

/// 响应体解析失败之后，往对话历史里补的说明，理由同 [`request_failed_message`]。
pub fn parse_failed_message(err: &str) -> Value {
    json!({
        "role": "system",
        "content": format!(
            "[系统提示] 上一次请求返回的内容解析失败（{err}），没有得到正常回复，\
             上面那个问题还没有被回答。如果用户接下来说\"继续\"之类的话，是指继续回答上面\
             被打断的那个问题。"
        )
    })
}

/// 一轮完整的"发请求 → 失败重试 → 解析响应"，两种 wire 协议（chat/completions
/// 和 Responses API）在这里统一处理，调用方拿到的永远是 chat/completions 形状
/// 的 assistant 消息。`tools` 传 `None` 表示这一轮不带任何工具（强制模型直接
/// 收尾给结论）。`event_prefix` 用来区分是哪个 Agent 发出的事件
/// （`"coding"`/`"sqlagent"`），前端按这个前缀订阅。
#[allow(clippy::too_many_arguments)]
pub async fn call_llm_once(
    client: &reqwest::Client,
    provider: &AiProvider,
    api_key: &Option<String>,
    messages: &[Value],
    tools: Option<&Value>,
    app_handle: &AppHandle,
    session_id: Uuid,
    event_prefix: &str,
    cancel_token: &tokio_util::sync::CancellationToken,
) -> Result<LlmRoundResult, LlmCallError> {
    let is_responses = provider.wire_api == "responses";
    let url = if is_responses {
        format!("{}/responses", provider.api_base.trim_end_matches('/'))
    } else {
        format!("{}/chat/completions", provider.api_base.trim_end_matches('/'))
    };

    let mut body = if is_responses {
        let (instructions, input) = messages_to_responses_input(messages);
        let mut body = json!({
            "model": provider.model,
            "instructions": instructions,
            "input": input,
            "stream": false,
        });
        if let Some(tools) = tools {
            body["tools"] = chat_tools_to_responses_tools(tools);
            body["tool_choice"] = json!("auto");
        }
        body
    } else {
        let mut body = json!({ "model": provider.model, "messages": messages });
        if let Some(tools) = tools {
            body["tools"] = tools.clone();
            body["tool_choice"] = json!("auto");
        }
        body
    };
    if let Some(effort) = &provider.reasoning_effort {
        if is_responses {
            body["reasoning"] = json!({ "effort": effort });
        } else {
            body["reasoning_effort"] = json!(effort);
        }
    }

    let mut retry_count = 0u32;
    let (resp, body_text) = loop {
        let mut req = client.post(&url).json(&body);
        if let Some(key) = api_key {
            req = req.bearer_auth(key);
        }
        let send_result = tokio::select! {
            biased;
            result = req.send() => result,
            _ = cancel_token.cancelled() => return Err(LlmCallError::Cancelled),
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
            None => true,
        } || is_retryable_error_text(&body_text);
        if retryable && retry_count < MAX_HTTP_RETRIES {
            retry_count += 1;
            let _ = app_handle.emit(
                &format!("{event_prefix}:assistant-note"),
                json!({
                    "sessionId": session_id,
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
            let delay = std::time::Duration::from_secs(RETRY_BASE_DELAY_SECS * (1u64 << (retry_count - 1)));
            tokio::select! {
                _ = tokio::time::sleep(delay) => {}
                _ = cancel_token.cancelled() => return Err(LlmCallError::Cancelled),
            }
            continue;
        }
        break (
            None,
            format!("{}: {body_text}", status.map(|s| s.to_string()).unwrap_or_else(|| "连接失败".to_string())),
        );
    };
    let Some(resp) = resp else {
        return Err(LlmCallError::RequestFailed(body_text));
    };
    let body: Value = match resp.json().await {
        Ok(b) => b,
        Err(e) => return Err(LlmCallError::ParseFailed(e.to_string())),
    };

    let usage = extract_token_usage(&body, &provider.wire_api);
    if let Some((prompt, completion, total)) = usage {
        let _ = app_handle.emit(
            &format!("{event_prefix}:token-usage"),
            json!({ "sessionId": session_id, "promptTokens": prompt, "completionTokens": completion, "totalTokens": total }),
        );
    }
    let message = if is_responses {
        let (text, tool_calls) = parse_responses_output(&body);
        json!({ "role": "assistant", "content": text, "tool_calls": tool_calls })
    } else {
        body["choices"][0]["message"].clone()
    };
    let tool_calls = message["tool_calls"].as_array().cloned().unwrap_or_default();
    Ok(LlmRoundResult { message, tool_calls, usage })
}

/// 一整轮对话（一条用户消息到最终结论）的 token 用量累加——工具循环可能要跑
/// 好几次 API 请求，单次请求的消耗只在过程中当进度参考，这里累加起来才是用户
/// 真正关心的"这轮对话一共花了多少 token"。
#[derive(Default)]
pub struct TurnUsage {
    pub prompt: i64,
    pub completion: i64,
    pub total: i64,
}

impl TurnUsage {
    pub fn add(&mut self, usage: Option<(i64, i64, i64)>) {
        if let Some((prompt, completion, total)) = usage {
            self.prompt += prompt;
            self.completion += completion;
            self.total += total;
        }
    }

    /// `total == 0` 时不发（这一轮压根没有成功请求过），避免时间线里堆一堆没有
    /// 信息量的"0 tokens"提示。
    pub fn emit_summary(&self, app_handle: &AppHandle, session_id: Uuid, event_prefix: &str) {
        if self.total > 0 {
            let _ = app_handle.emit(
                &format!("{event_prefix}:token-usage-summary"),
                json!({
                    "sessionId": session_id,
                    "promptTokens": self.prompt,
                    "completionTokens": self.completion,
                    "totalTokens": self.total,
                }),
            );
        }
    }
}
