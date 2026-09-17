//! 请求执行引擎——直接用项目里已经在用的 `reqwest`（`coding/webfetch.rs` 也在用
//! 它做 AI 的网页抓取工具），不新增 HTTP 客户端依赖
//! （docs/HTTP_DESKTOP_PLAN.md §4.2、§2.6 决策表）。
//!
//! 范围说明（对照方案 §4.2 的简化）：本轮没有做"每个集合持有一个共享
//! `reqwest::Client`（连接池/Cookie Jar 按集合隔离）"，每次发送都新建一个
//! `Client`——牺牲了跨请求的连接复用和 Cookie 持久化，换来实现和状态管理都更简单；
//! 如果后续要补 Cookie Jar，在 `http_desk::mod` 按 workspace_id 持有一份
//! `HashMap<String, reqwest::Client>` 即可，不影响这里的函数签名。同样没有做
//! 自定义代理/CA 证书导入/mTLS（§7 提到的这几项），走 reqwest 默认的系统 TLS 信任
//! 链和重定向策略。

use base64::Engine;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, AUTHORIZATION, CONTENT_TYPE};

use crate::error::AppError;

use super::model::*;
use super::vars::{interpolate, VarContext};

/// 响应体预览上限（docs/HTTP_DESKTOP_PLAN.md §7）——超过这个大小只在返回值里
/// 截断展示的 `body`/`body_base64`，`size_bytes` 仍然是真实总大小。
/// 注意：这是"读取完成后再截断"，不是网络层提前中止——真正防住恶意超大响应
/// 撑爆内存需要流式读取 + 提前放弃连接，本轮未实现，留作后续加固项。
const MAX_PREVIEW_BYTES: usize = 5 * 1024 * 1024;
const REQUEST_TIMEOUT_SECS: u64 = 30;

fn default_content_type(body: &RequestBody) -> Option<&'static str> {
    match body {
        RequestBody::Json { .. } => Some("application/json"),
        RequestBody::FormUrlEncoded { .. } => Some("application/x-www-form-urlencoded"),
        RequestBody::Raw { .. } | RequestBody::None | RequestBody::FormData { .. } => None,
    }
}

/// 把 `RequestDef` 按当前变量上下文解析成真实要发出去的请求并执行。
pub async fn execute(req: &RequestDef, ctx: &VarContext<'_>) -> Result<HttpExecuteResult, AppError> {
    let method_str = interpolate(&req.method, ctx).to_uppercase();
    let method = reqwest::Method::from_bytes(method_str.as_bytes())
        .map_err(|_| AppError::Internal(format!("非法的 HTTP 方法: {method_str}")))?;

    let mut raw_url = interpolate(&req.url, ctx);
    if raw_url.trim().is_empty() {
        return Err(AppError::Internal("请求 URL 不能为空".into()));
    }
    if !raw_url.contains("://") {
        raw_url = format!("https://{raw_url}");
    }
    let mut url = reqwest::Url::parse(&raw_url)
        .map_err(|e| AppError::Internal(format!("URL 解析失败: {e}")))?;

    {
        let mut pairs = url.query_pairs_mut();
        for p in req.params.iter().filter(|p| p.enabled && !p.key.is_empty()) {
            pairs.append_pair(&interpolate(&p.key, ctx), &interpolate(&p.value, ctx));
        }
    }
    // ApiKey 放在 query 里的话，此时一起拼进 URL，晚于上面的普通 Params——具体
    // 顺序不影响服务端解析，query string 本来就是无序键值对集合。
    if let AuthConfig::ApiKey {
        key,
        value,
        add_to: ApiKeyLocation::Query,
    } = &req.auth
    {
        url.query_pairs_mut()
            .append_pair(&interpolate(key, ctx), &interpolate(value, ctx));
    }

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(10))
        .timeout(std::time::Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .build()?;

    let mut headers = HeaderMap::new();
    for h in req.headers.iter().filter(|h| h.enabled && !h.key.is_empty()) {
        let name = HeaderName::from_bytes(interpolate(&h.key, ctx).as_bytes())
            .map_err(|e| AppError::Internal(format!("非法的请求头名: {e}")))?;
        let value = HeaderValue::from_str(&interpolate(&h.value, ctx))
            .map_err(|e| AppError::Internal(format!("非法的请求头值: {e}")))?;
        headers.insert(name, value);
    }

    match &req.auth {
        AuthConfig::None => {}
        AuthConfig::Bearer { token } => {
            let value = HeaderValue::from_str(&format!("Bearer {}", interpolate(token, ctx)))
                .map_err(|e| AppError::Internal(e.to_string()))?;
            headers.insert(AUTHORIZATION, value);
        }
        AuthConfig::Basic { username, password } => {
            let raw = format!(
                "{}:{}",
                interpolate(username, ctx),
                interpolate(password, ctx)
            );
            let encoded = base64::engine::general_purpose::STANDARD.encode(raw);
            let value = HeaderValue::from_str(&format!("Basic {encoded}"))
                .map_err(|e| AppError::Internal(e.to_string()))?;
            headers.insert(AUTHORIZATION, value);
        }
        AuthConfig::ApiKey {
            key,
            value,
            add_to: ApiKeyLocation::Header,
        } => {
            let name = HeaderName::from_bytes(interpolate(key, ctx).as_bytes())
                .map_err(|e| AppError::Internal(e.to_string()))?;
            let hv = HeaderValue::from_str(&interpolate(value, ctx))
                .map_err(|e| AppError::Internal(e.to_string()))?;
            headers.insert(name, hv);
        }
        AuthConfig::ApiKey {
            add_to: ApiKeyLocation::Query,
            ..
        } => {} // 已经在上面拼进 URL 了
    }

    if let Some(ct) = default_content_type(&req.body) {
        if !headers.contains_key(CONTENT_TYPE) {
            headers.insert(CONTENT_TYPE, HeaderValue::from_static(ct));
        }
    }
    if let RequestBody::Raw { content_type, .. } = &req.body {
        if !headers.contains_key(CONTENT_TYPE) {
            if let Ok(hv) = HeaderValue::from_str(content_type) {
                headers.insert(CONTENT_TYPE, hv);
            }
        }
    }

    let mut builder = client.request(method.clone(), url.clone()).headers(headers);

    builder = match &req.body {
        RequestBody::None => builder,
        RequestBody::Json { content } => builder.body(interpolate(content, ctx)),
        RequestBody::Raw { content, .. } => builder.body(interpolate(content, ctx)),
        RequestBody::FormUrlEncoded { items } => {
            let pairs: Vec<(String, String)> = items
                .iter()
                .filter(|i| i.enabled && !i.key.is_empty())
                .map(|i| (interpolate(&i.key, ctx), interpolate(&i.value, ctx)))
                .collect();
            builder.form(&pairs)
        }
        RequestBody::FormData { items } => {
            let mut form = reqwest::multipart::Form::new();
            for i in items.iter().filter(|i| i.enabled && !i.key.is_empty()) {
                form = form.text(interpolate(&i.key, ctx), interpolate(&i.value, ctx));
            }
            builder.multipart(form)
        }
    };

    let started = std::time::Instant::now();
    let resp = builder.send().await?;
    let status = resp.status();
    let resp_headers: Vec<(String, String)> = resp
        .headers()
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();
    let bytes = resp.bytes().await?;
    let duration_ms = started.elapsed().as_millis() as u64;

    let size_bytes = bytes.len() as u64;
    let truncated = bytes.len() > MAX_PREVIEW_BYTES;
    let preview = if truncated {
        &bytes[..MAX_PREVIEW_BYTES]
    } else {
        &bytes[..]
    };

    let (body, body_is_text, body_base64) = match std::str::from_utf8(preview) {
        Ok(text) => (text.to_string(), true, None),
        Err(_) => (
            String::new(),
            false,
            Some(base64::engine::general_purpose::STANDARD.encode(preview)),
        ),
    };

    Ok(HttpExecuteResult {
        status: status.as_u16(),
        status_text: status.canonical_reason().unwrap_or("").to_string(),
        headers: resp_headers,
        body,
        body_is_text,
        body_base64,
        duration_ms,
        size_bytes,
        resolved_url: url.to_string(),
        truncated,
    })
}
