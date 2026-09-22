//! 导入解析器：curl 命令（最快能覆盖"从浏览器 DevTools/Postman 复制一条 curl
//! 命令"这个高频场景）+ Postman Collection v2.x + OpenAPI 3.x（docs/HTTP_DESKTOP_PLAN.md
//! §4.7）。HAR 导入、Postman/OpenCollection 导出没有落地——导出见 `super::export`
//! （已支持导出成 Postman 格式），HAR 留作后续任务（见 http_desk/mod.rs 顶部范围说明）。

use serde_json::Value;

use crate::error::AppError;

use super::model::*;

/// Postman/OpenAPI 导入解析出来的一个请求草稿——比 `RequestDef` 多一个 `folder`
/// 字段（导入时请求还没有落盘路径，`folder` 决定落到 `requests/` 下哪个子目录），
/// 没有 `id`（落盘时才分配，和 `parse_curl` 返回 `RequestDef` 里 `id` 留空是同一个
/// 约定）。
#[derive(Debug, Clone)]
pub struct ImportedRequest {
    pub name: String,
    pub method: String,
    pub url: String,
    pub params: Vec<KeyValueItem>,
    pub headers: Vec<KeyValueItem>,
    pub auth: AuthConfig,
    pub body: RequestBody,
    pub folder: Vec<String>,
}

/// 导入一整份集合的结果——`service::import_collection` 消费这个类型，负责实际
/// 分配 slug/id 并落盘（导入解析本身不碰文件系统，纯函数、方便单测）。
#[derive(Debug, Clone)]
pub struct ImportedCollection {
    pub name: String,
    pub description: Option<String>,
    pub variables: Vec<EnvVar>,
    pub requests: Vec<ImportedRequest>,
}

/// 简化版 shell 分词：处理单/双引号、反斜杠转义和行尾 `\` 续行（curl 多行命令的
/// 常见写法），不追求 100% 复刻 POSIX shell 分词规则——覆盖"从浏览器/Postman
/// 复制的 curl 命令"这个目标场景足够。
fn tokenize(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut started = false;
    let mut in_single = false;
    let mut in_double = false;
    let mut chars = input.trim().chars().peekable();

    while let Some(c) = chars.next() {
        if in_single {
            if c == '\'' {
                in_single = false;
            } else {
                current.push(c);
            }
            continue;
        }
        if in_double {
            if c == '"' {
                in_double = false;
            } else if c == '\\' {
                match chars.peek() {
                    Some('"') | Some('\\') | Some('$') | Some('`') => {
                        current.push(chars.next().unwrap());
                    }
                    _ => current.push(c),
                }
            } else {
                current.push(c);
            }
            continue;
        }
        match c {
            '\'' => {
                in_single = true;
                started = true;
            }
            '"' => {
                in_double = true;
                started = true;
            }
            '\\' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                    continue;
                }
                if let Some(next) = chars.next() {
                    current.push(next);
                    started = true;
                }
            }
            c if c.is_whitespace() => {
                if started {
                    tokens.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            _ => {
                current.push(c);
                started = true;
            }
        }
    }
    if started {
        tokens.push(current);
    }
    tokens
}

/// 把一条 curl 命令解析成 `RequestDef`（`id`/`name` 留给调用方补——导入是
/// "生成一份草稿"，落盘/命名由 `commands::http_desk::http_import_curl` 决定）。
pub fn parse_curl(command: &str) -> Result<RequestDef, AppError> {
    let tokens = tokenize(command);
    let mut iter = tokens.into_iter().peekable();
    if iter.peek().map(|s| s.as_str()) == Some("curl") {
        iter.next();
    }

    let mut url: Option<String> = None;
    let mut method: Option<String> = None;
    let mut headers = Vec::new();
    let mut data: Option<String> = None;
    let mut basic_auth: Option<(String, String)> = None;

    while let Some(tok) = iter.next() {
        match tok.as_str() {
            "-X" | "--request" => method = iter.next(),
            "-H" | "--header" => {
                if let Some(h) = iter.next() {
                    if let Some((k, v)) = h.split_once(':') {
                        headers.push(KeyValueItem {
                            key: k.trim().to_string(),
                            value: v.trim().to_string(),
                            enabled: true,
                        });
                    }
                }
            }
            "-d" | "--data" | "--data-raw" | "--data-binary" | "--data-urlencode" => {
                if let Some(d) = iter.next() {
                    data = Some(match data {
                        Some(existing) => format!("{existing}&{d}"),
                        None => d,
                    });
                }
            }
            "-u" | "--user" => {
                if let Some(cred) = iter.next() {
                    basic_auth = Some(match cred.split_once(':') {
                        Some((u, p)) => (u.to_string(), p.to_string()),
                        None => (cred, String::new()),
                    });
                }
            }
            "-b" | "--cookie" => {
                if let Some(c) = iter.next() {
                    headers.push(KeyValueItem {
                        key: "Cookie".into(),
                        value: c,
                        enabled: true,
                    });
                }
            }
            "-A" | "--user-agent" => {
                if let Some(v) = iter.next() {
                    headers.push(KeyValueItem {
                        key: "User-Agent".into(),
                        value: v,
                        enabled: true,
                    });
                }
            }
            "--compressed" | "-k" | "--insecure" | "-s" | "--silent" | "-v" | "--verbose"
            | "-i" | "--include" | "-L" | "--location" => {}
            other if other.starts_with('-') => {
                // 未识别的 flag：保守地吞掉紧随其后、看起来是它的值的一个 token
                // （不是以 `-` 开头），避免把值误判成 URL——不追求识别所有 curl
                // flag，只求不把大多数场景下的 URL 解析搞错。
                if iter.peek().map(|s| !s.starts_with('-')).unwrap_or(false) {
                    iter.next();
                }
            }
            other => {
                if url.is_none() {
                    url = Some(other.trim_matches(['\'', '"']).to_string());
                }
            }
        }
    }

    let url = url.ok_or_else(|| AppError::Internal("未能从 curl 命令中解析出 URL".into()))?;
    let method = method.unwrap_or_else(|| if data.is_some() { "POST".into() } else { "GET".into() });

    let body = match data {
        Some(d) => {
            let looks_json = headers
                .iter()
                .any(|h| h.key.eq_ignore_ascii_case("content-type") && h.value.contains("json"))
                || d.trim_start().starts_with('{')
                || d.trim_start().starts_with('[');
            if looks_json {
                RequestBody::Json { content: d }
            } else {
                RequestBody::Raw {
                    content: d,
                    content_type: "application/x-www-form-urlencoded".into(),
                }
            }
        }
        None => RequestBody::None,
    };

    let auth = match basic_auth {
        Some((username, password)) => AuthConfig::Basic { username, password },
        None => AuthConfig::None,
    };

    Ok(RequestDef {
        id: String::new(),
        name: "导入的请求".into(),
        method: method.to_uppercase(),
        url,
        params: Vec::new(),
        headers,
        auth,
        body,
    })
}

// ---------------------------------------------------------------------
// Postman Collection v2.x 导入
// ---------------------------------------------------------------------

/// Postman 的 `description` 字段实践中见过两种形态：纯字符串，或者
/// `{"content": "...", "type": "text/markdown"}` 对象——两种都取出纯文本，空字符串
/// 当成"没有描述"（`None`）而不是保留一个空字符串，界面上少一个"有描述但是空的"
/// 的边界情况。
fn value_to_text(v: &Value) -> Option<String> {
    let text = match v {
        Value::String(s) => s.clone(),
        Value::Object(_) => v.get("content")?.as_str()?.to_string(),
        _ => return None,
    };
    if text.trim().is_empty() { None } else { Some(text) }
}

fn postman_kv_lookup(auth_block: &Value, field: &str, want_key: &str) -> Option<String> {
    auth_block
        .get(field)?
        .as_array()?
        .iter()
        .find(|e| e.get("key").and_then(|k| k.as_str()) == Some(want_key))
        .and_then(|e| e.get("value"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Postman 的 `auth` 对象——`{"type": "bearer", "bearer": [{"key":"token","value":"..."}]}`
/// 这种形态，`type` 决定要去哪个同名子数组里按 `key` 找值。OAuth2/Digest/AWS 签名等
/// 映射不到 roc_desk 现有的 `AuthConfig` 变体，降级成 `None`（导入后请求还能用，只是
/// 认证信息需要用户手动补，不是解析失败）。
fn parse_postman_auth(v: &Value) -> AuthConfig {
    match v.get("type").and_then(|t| t.as_str()).unwrap_or("noauth") {
        "bearer" => AuthConfig::Bearer {
            token: postman_kv_lookup(v, "bearer", "token").unwrap_or_default(),
        },
        "basic" => AuthConfig::Basic {
            username: postman_kv_lookup(v, "basic", "username").unwrap_or_default(),
            password: postman_kv_lookup(v, "basic", "password").unwrap_or_default(),
        },
        "apikey" => {
            let key = postman_kv_lookup(v, "apikey", "key").unwrap_or_default();
            let value = postman_kv_lookup(v, "apikey", "value").unwrap_or_default();
            let add_to = match postman_kv_lookup(v, "apikey", "in").as_deref() {
                Some("query") => ApiKeyLocation::Query,
                _ => ApiKeyLocation::Header,
            };
            AuthConfig::ApiKey { key, value, add_to }
        }
        _ => AuthConfig::None,
    }
}

/// 按 `?` 手工切一次 URL——不用 `reqwest::Url` 解析，是因为这里的 URL 大概率
/// 带着 `{{变量}}`（Postman 变量语法和 roc_desk 自己的插值语法碰巧一致，直接原样
/// 保留），标准 URL 解析器不认识花括号，会直接报错或把它们错误转义。
fn split_url_query(raw: &str) -> (String, Vec<KeyValueItem>) {
    match raw.split_once('?') {
        Some((base, query)) if !query.is_empty() => {
            let params = query
                .split('&')
                .filter(|s| !s.is_empty())
                .map(|pair| match pair.split_once('=') {
                    Some((k, v)) => KeyValueItem { key: k.to_string(), value: v.to_string(), enabled: true },
                    None => KeyValueItem { key: pair.to_string(), value: String::new(), enabled: true },
                })
                .collect();
            (base.to_string(), params)
        }
        _ => (raw.to_string(), Vec::new()),
    }
}

/// Postman 的 `url` 字段可能是裸字符串，也可能是带 `raw`/`query`/`variable` 的对象
/// （Postman 桌面客户端导出的几乎都是对象形态，程序生成的 collection 有时是字符串）。
/// query 参数优先用结构化的 `url.query` 数组（带 `disabled` 标记，比重新切 `raw`
/// 里的查询串更准），没有就退回手工切 `raw`。返回值是 `(不含查询串的 base url, 参数列表)`——
/// `RequestDef.url` 和 `RequestDef.params` 在发送时是分开拼的（见 `client.rs`），
/// 两者都放查询串会导致最终请求 URL 里参数重复一遍。
fn parse_postman_url(v: &Value) -> (String, Vec<KeyValueItem>) {
    match v {
        Value::String(s) => split_url_query(s),
        Value::Object(_) => {
            let raw = v.get("raw").and_then(|r| r.as_str()).unwrap_or("");
            let (base, fallback_params) = split_url_query(raw);
            let params = match v.get("query").and_then(|q| q.as_array()) {
                Some(arr) if !arr.is_empty() => arr
                    .iter()
                    .filter_map(|q| {
                        let key = q.get("key").and_then(|k| k.as_str())?.to_string();
                        let value = q.get("value").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        let enabled = !q.get("disabled").and_then(|d| d.as_bool()).unwrap_or(false);
                        Some(KeyValueItem { key, value, enabled })
                    })
                    .collect(),
                _ => fallback_params,
            };
            (base, params)
        }
        _ => (String::new(), Vec::new()),
    }
}

fn parse_postman_kv_array(v: Option<&Value>) -> Vec<KeyValueItem> {
    v.and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| {
                    let key = item.get("key").and_then(|k| k.as_str())?.to_string();
                    let value = item.get("value").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let enabled = !item.get("disabled").and_then(|d| d.as_bool()).unwrap_or(false);
                    Some(KeyValueItem { key, value, enabled })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `formdata` 数组里的条目可能是 `"type": "file"`（`src` 指向本机文件路径，导出
/// 到另一台机器上通常已经失效）——roc_desk 首期不支持表单文件上传（见
/// `model.rs::RequestBody::FormData` 文档注释），这类字段原样保留成一个禁用状态
/// 的文本行（value 留空+名字提示），让用户知道"这里原来有个文件字段"而不是被
/// 静默丢弃，需要的话自己在 UI 里重新配置。
fn parse_postman_formdata(v: Option<&Value>) -> Vec<KeyValueItem> {
    v.and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| {
                    let key = item.get("key").and_then(|k| k.as_str())?.to_string();
                    let is_file = item.get("type").and_then(|t| t.as_str()) == Some("file");
                    if is_file {
                        Some(KeyValueItem { key, value: "(文件字段，需要重新选择文件)".into(), enabled: false })
                    } else {
                        let value = item.get("value").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        let enabled = !item.get("disabled").and_then(|d| d.as_bool()).unwrap_or(false);
                        Some(KeyValueItem { key, value, enabled })
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

fn parse_postman_body(v: Option<&Value>) -> RequestBody {
    let Some(v) = v else { return RequestBody::None };
    match v.get("mode").and_then(|m| m.as_str()).unwrap_or("") {
        "raw" => {
            let content = v.get("raw").and_then(|r| r.as_str()).unwrap_or("").to_string();
            let language = v
                .get("options")
                .and_then(|o| o.get("raw"))
                .and_then(|r| r.get("language"))
                .and_then(|l| l.as_str())
                .unwrap_or("text");
            if language == "json" {
                RequestBody::Json { content }
            } else {
                let content_type = match language {
                    "xml" => "application/xml",
                    "html" => "text/html",
                    _ => "text/plain",
                };
                RequestBody::Raw { content, content_type: content_type.into() }
            }
        }
        "urlencoded" => RequestBody::FormUrlEncoded { items: parse_postman_kv_array(v.get("urlencoded")) },
        "formdata" => RequestBody::FormData { items: parse_postman_formdata(v.get("formdata")) },
        // GraphQL body 没有对应的 roc_desk 变体，降级成原始文本（query 部分基本
        // 就是要发的内容，`variables` 字段丢失——GraphQL 请求本轮不是重点场景）。
        "graphql" => {
            let query = v.get("graphql").and_then(|g| g.get("query")).and_then(|q| q.as_str()).unwrap_or("");
            RequestBody::Raw { content: query.to_string(), content_type: "application/json".into() }
        }
        _ => RequestBody::None,
    }
}

fn parse_postman_request(name: &str, req: &Value, folder: &[String]) -> ImportedRequest {
    let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("GET").to_uppercase();
    let (url, params) = req.get("url").map(parse_postman_url).unwrap_or_default();
    let headers = parse_postman_kv_array(req.get("header"));
    let auth = req.get("auth").map(parse_postman_auth).unwrap_or(AuthConfig::None);
    let body = parse_postman_body(req.get("body"));
    ImportedRequest { name: name.to_string(), method, url, params, headers, auth, body, folder: folder.to_vec() }
}

/// Postman 的 `item` 数组是"文件夹和请求混在同一个数组里"——有自己的 `item` 子数组
/// 就是文件夹（继续递归），有 `request` 字段就是叶子请求，两者互斥。
fn walk_postman_items(items: &[Value], folder: &[String], out: &mut Vec<ImportedRequest>) {
    for item in items {
        let name = item.get("name").and_then(|n| n.as_str()).unwrap_or("未命名").to_string();
        if let Some(children) = item.get("item").and_then(|i| i.as_array()) {
            let mut next = folder.to_vec();
            next.push(name);
            walk_postman_items(children, &next, out);
        } else if let Some(req) = item.get("request") {
            out.push(parse_postman_request(&name, req, folder));
        }
    }
}

/// 解析一份 Postman Collection（v2.0/v2.1 导出文件，两者字段形状基本一致，没有
/// 专门按 `info.schema` 分支处理版本差异）。
pub fn parse_postman_collection(content: &str) -> Result<ImportedCollection, AppError> {
    let root: Value = serde_json::from_str(content)
        .map_err(|e| AppError::Internal(format!("Postman 集合 JSON 解析失败：{e}")))?;
    let info = root.get("info").cloned().unwrap_or(Value::Null);
    let name = info
        .get("name")
        .and_then(|n| n.as_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("导入的集合")
        .to_string();
    let description = info.get("description").and_then(value_to_text);
    let variables = root
        .get("variable")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| {
                    let key = item.get("key").and_then(|k| k.as_str())?.to_string();
                    let value = item.get("value").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    Some(EnvVar { key, value, secret: false, enabled: true })
                })
                .collect()
        })
        .unwrap_or_default();
    let items = root.get("item").and_then(|i| i.as_array()).cloned().unwrap_or_default();
    let mut requests = Vec::new();
    walk_postman_items(&items, &[], &mut requests);
    if requests.is_empty() {
        return Err(AppError::Internal(
            "未从文件中解析出任何请求，请确认这是一份 Postman Collection v2.x 导出文件（Postman 里 Export → Collection v2.1）".into(),
        ));
    }
    Ok(ImportedCollection { name, description, variables, requests })
}

// ---------------------------------------------------------------------
// OpenAPI 3.x 导入
// ---------------------------------------------------------------------

/// OpenAPI 文件可能是 JSON 也可能是 YAML——`serde_yaml` 能同时解析两者（JSON 是
/// YAML 的子集），解析成 `serde_yaml::Value` 后转一遍 `serde_json::Value`，这样
/// 下面能复用和 Postman 解析同一套 `.get()`/`.as_str()` 链式取值写法，不用另外
/// 学一遍 `serde_yaml::Value` 的 API。
fn parse_yaml_or_json(content: &str) -> Result<Value, AppError> {
    let yaml: serde_yaml::Value =
        serde_yaml::from_str(content).map_err(|e| AppError::Internal(format!("解析失败（不是合法的 JSON/YAML）：{e}")))?;
    serde_json::to_value(&yaml).map_err(|e| AppError::Internal(format!("转换失败：{e}")))
}

const OPENAPI_METHODS: &[&str] = &["get", "post", "put", "delete", "patch", "head", "options"];

/// `parameters` 数组（`in: query/header/path`）——`path` 类型的参数不进
/// params/headers（已经作为 `{{name}}` 嵌在 URL 里），只收集名字用来在集合变量里
/// 占位（见 `parse_openapi` 里的处理）。
fn split_openapi_params(params: &[Value]) -> (Vec<KeyValueItem>, Vec<KeyValueItem>, Vec<String>) {
    let mut query = Vec::new();
    let mut header = Vec::new();
    let mut path_names = Vec::new();
    for p in params {
        let Some(name) = p.get("name").and_then(|n| n.as_str()) else { continue };
        let default_value = p
            .get("schema")
            .and_then(|s| s.get("default"))
            .or_else(|| p.get("example"))
            .and_then(|v| v.as_str().map(|s| s.to_string()).or_else(|| Some(v.to_string())))
            .unwrap_or_default();
        match p.get("in").and_then(|i| i.as_str()) {
            Some("query") => query.push(KeyValueItem { key: name.to_string(), value: default_value, enabled: true }),
            Some("header") => header.push(KeyValueItem { key: name.to_string(), value: default_value, enabled: true }),
            Some("path") => path_names.push(name.to_string()),
            _ => {}
        }
    }
    (query, header, path_names)
}

/// 从 `requestBody.content` 里挑一种媒体类型生成请求体草稿——优先 JSON（最常见），
/// 其次 form-urlencoded，其余媒体类型降级成空 JSON 占位（比完全不填强，至少 Content-Type
/// 语义对，用户自己补内容）。示例值来源：`example` 字段，或 `examples` 下第一个，
/// 都没有就是空对象。
fn openapi_request_body(rb: Option<&Value>) -> RequestBody {
    let Some(content) = rb.and_then(|rb| rb.get("content")) else { return RequestBody::None };
    let example_of = |media: &Value| -> String {
        if let Some(ex) = media.get("example") {
            serde_json::to_string_pretty(ex).unwrap_or_default()
        } else if let Some(examples) = media.get("examples").and_then(|e| e.as_object()) {
            examples
                .values()
                .next()
                .and_then(|e| e.get("value"))
                .map(|v| serde_json::to_string_pretty(v).unwrap_or_default())
                .unwrap_or_default()
        } else {
            String::new()
        }
    };
    if let Some(json_media) = content.get("application/json") {
        let example = example_of(json_media);
        return RequestBody::Json { content: if example.is_empty() { "{}".into() } else { example } };
    }
    if let Some(form_media) = content.get("application/x-www-form-urlencoded") {
        let props = form_media
            .get("schema")
            .and_then(|s| s.get("properties"))
            .and_then(|p| p.as_object());
        let items = props
            .map(|p| p.keys().map(|k| KeyValueItem { key: k.clone(), value: String::new(), enabled: true }).collect())
            .unwrap_or_default();
        return RequestBody::FormUrlEncoded { items };
    }
    RequestBody::None
}

/// operation 的 `security` 数组（没有就退回 spec 根级默认 `security`）+
/// `components.securitySchemes` 对照出认证方式——OpenAPI 的 `security` 只写了
/// "用哪个 scheme"，没有真实凭据（规范本来就不该带凭据），映射出来的
/// `AuthConfig` 是"认证类型对了、值留空待填"，不是完整可用的认证信息。
fn openapi_auth(security: Option<&Value>, schemes: &Value) -> AuthConfig {
    let Some(security) = security.and_then(|s| s.as_array()).and_then(|arr| arr.first()) else {
        return AuthConfig::None;
    };
    let Some(scheme_name) = security.as_object().and_then(|o| o.keys().next()) else {
        return AuthConfig::None;
    };
    let Some(scheme) = schemes.get(scheme_name) else { return AuthConfig::None };
    match scheme.get("type").and_then(|t| t.as_str()) {
        Some("http") if scheme.get("scheme").and_then(|s| s.as_str()) == Some("bearer") => {
            AuthConfig::Bearer { token: String::new() }
        }
        Some("http") if scheme.get("scheme").and_then(|s| s.as_str()) == Some("basic") => {
            AuthConfig::Basic { username: String::new(), password: String::new() }
        }
        Some("apiKey") => {
            let key = scheme.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
            let add_to = match scheme.get("in").and_then(|i| i.as_str()) {
                Some("query") => ApiKeyLocation::Query,
                _ => ApiKeyLocation::Header,
            };
            AuthConfig::ApiKey { key, value: String::new(), add_to }
        }
        _ => AuthConfig::None,
    }
}

/// 解析 OpenAPI 3.x 规范——按 `paths` 下每个路径 × 每个 HTTP 方法生成一个请求，
/// 按 `tags` 的第一个分组当文件夹（没有 tag 就放集合根目录）。`servers[0].url`
/// 存成集合变量 `base_url`，每个请求的 URL 是 `{{base_url}}<path>`——比每个请求
/// 各自硬编码一份 server URL 更好改（比如切换测试环境的 server 只用改这一处），
/// 也是很多人从 OpenAPI 导入 Postman 时的既有习惯。路径参数 `{id}` 转成 roc_desk
/// 自己的插值语法 `{{id}}`（和 Postman 变量语法凑巧一致），同名变量以空值形式
/// 写进集合变量里，保证请求一打开就有能填的地方，不是必须先手动加变量才能用。
pub fn parse_openapi(content: &str) -> Result<ImportedCollection, AppError> {
    let root = parse_yaml_or_json(content)?;
    if root.get("openapi").is_none() && root.get("swagger").is_none() {
        return Err(AppError::Internal("未识别到 openapi/swagger 版本字段，确认这是一份 OpenAPI/Swagger 规范文件".into()));
    }
    let name = root
        .get("info")
        .and_then(|i| i.get("title"))
        .and_then(|t| t.as_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("导入的 OpenAPI 集合")
        .to_string();
    let description = root.get("info").and_then(|i| i.get("description")).and_then(value_to_text);
    let base_url = root
        .get("servers")
        .and_then(|s| s.as_array())
        .and_then(|arr| arr.first())
        .and_then(|s| s.get("url"))
        .and_then(|u| u.as_str())
        .unwrap_or("")
        .to_string();
    let empty_schemes = Value::Object(Default::default());
    let schemes = root
        .get("components")
        .and_then(|c| c.get("securitySchemes"))
        .unwrap_or(&empty_schemes);
    let root_security = root.get("security");

    let mut requests = Vec::new();
    let mut path_var_names: Vec<String> = Vec::new();
    if let Some(paths) = root.get("paths").and_then(|p| p.as_object()) {
        for (path, path_item) in paths {
            let shared_params: Vec<Value> = path_item
                .get("parameters")
                .and_then(|p| p.as_array())
                .cloned()
                .unwrap_or_default();
            for method in OPENAPI_METHODS {
                let Some(op) = path_item.get(*method) else { continue };
                let name = op
                    .get("summary")
                    .and_then(|s| s.as_str())
                    .or_else(|| op.get("operationId").and_then(|s| s.as_str()))
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| format!("{} {}", method.to_uppercase(), path));
                let folder = op
                    .get("tags")
                    .and_then(|t| t.as_array())
                    .and_then(|arr| arr.first())
                    .and_then(|t| t.as_str())
                    .map(|t| vec![t.to_string()])
                    .unwrap_or_default();

                let mut all_params = shared_params.clone();
                if let Some(op_params) = op.get("parameters").and_then(|p| p.as_array()) {
                    all_params.extend(op_params.clone());
                }
                let (params, headers, path_names) = split_openapi_params(&all_params);
                for n in &path_names {
                    if !path_var_names.contains(n) {
                        path_var_names.push(n.clone());
                    }
                }

                let url_path = path.replace('{', "{{").replace('}', "}}");
                let auth = openapi_auth(op.get("security").or(root_security), schemes);
                let body = openapi_request_body(op.get("requestBody"));

                requests.push(ImportedRequest {
                    name,
                    method: method.to_uppercase(),
                    url: format!("{{{{base_url}}}}{url_path}"),
                    params,
                    headers,
                    auth,
                    body,
                    folder,
                });
            }
        }
    }
    if requests.is_empty() {
        return Err(AppError::Internal("未从 paths 下解析出任何请求".into()));
    }

    let mut variables = vec![EnvVar { key: "base_url".into(), value: base_url, secret: false, enabled: true }];
    for name in path_var_names {
        variables.push(EnvVar { key: name, value: String::new(), secret: false, enabled: true });
    }

    Ok(ImportedCollection { name, description, variables, requests })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_get() {
        let req = parse_curl("curl https://api.example.com/users").unwrap();
        assert_eq!(req.method, "GET");
        assert_eq!(req.url, "https://api.example.com/users");
    }

    #[test]
    fn parses_post_with_json_body_and_headers() {
        let req = parse_curl(
            r#"curl -X POST https://api.example.com/users -H "Content-Type: application/json" -H "Authorization: Bearer abc" -d '{"name":"a"}'"#,
        )
        .unwrap();
        assert_eq!(req.method, "POST");
        assert_eq!(req.headers.len(), 2);
        match req.body {
            RequestBody::Json { content } => assert_eq!(content, r#"{"name":"a"}"#),
            _ => panic!("expected json body"),
        }
    }

    #[test]
    fn parses_basic_auth() {
        let req = parse_curl("curl -u admin:secret https://api.example.com/ping").unwrap();
        match req.auth {
            AuthConfig::Basic { username, password } => {
                assert_eq!(username, "admin");
                assert_eq!(password, "secret");
            }
            _ => panic!("expected basic auth"),
        }
    }

    #[test]
    fn parses_postman_collection_with_folder_and_query() {
        let json = r#"{
            "info": {"name": "示例集合", "description": "desc"},
            "item": [
                {
                    "name": "用户模块",
                    "item": [
                        {
                            "name": "获取用户",
                            "request": {
                                "method": "GET",
                                "header": [{"key": "X-Token", "value": "abc", "disabled": false}],
                                "url": {
                                    "raw": "https://api.example.com/users?page=1",
                                    "query": [{"key": "page", "value": "1", "disabled": false}]
                                },
                                "auth": {"type": "bearer", "bearer": [{"key": "token", "value": "{{token}}"}]}
                            }
                        }
                    ]
                }
            ],
            "variable": [{"key": "base_url", "value": "https://api.example.com"}]
        }"#;
        let collection = parse_postman_collection(json).unwrap();
        assert_eq!(collection.name, "示例集合");
        assert_eq!(collection.variables.len(), 1);
        assert_eq!(collection.requests.len(), 1);
        let req = &collection.requests[0];
        assert_eq!(req.folder, vec!["用户模块".to_string()]);
        assert_eq!(req.url, "https://api.example.com/users");
        assert_eq!(req.params.len(), 1);
        assert_eq!(req.params[0].key, "page");
        assert_eq!(req.headers.len(), 1);
        match &req.auth {
            AuthConfig::Bearer { token } => assert_eq!(token, "{{token}}"),
            _ => panic!("expected bearer auth"),
        }
    }

    #[test]
    fn parses_postman_json_body() {
        let json = r#"{
            "info": {"name": "c"},
            "item": [{
                "name": "创建",
                "request": {
                    "method": "POST",
                    "url": "https://api.example.com/users",
                    "body": {"mode": "raw", "raw": "{\"name\":\"a\"}", "options": {"raw": {"language": "json"}}}
                }
            }]
        }"#;
        let collection = parse_postman_collection(json).unwrap();
        match &collection.requests[0].body {
            RequestBody::Json { content } => assert_eq!(content, r#"{"name":"a"}"#),
            _ => panic!("expected json body"),
        }
    }

    #[test]
    fn rejects_postman_collection_without_requests() {
        let err = parse_postman_collection(r#"{"info": {"name": "空集合"}, "item": []}"#);
        assert!(err.is_err());
    }

    #[test]
    fn parses_openapi_paths_into_requests() {
        let spec = r#"{
            "openapi": "3.0.0",
            "info": {"title": "示例 API", "description": "desc"},
            "servers": [{"url": "https://api.example.com"}],
            "paths": {
                "/users/{id}": {
                    "get": {
                        "summary": "获取用户",
                        "tags": ["Users"],
                        "parameters": [
                            {"name": "id", "in": "path", "required": true},
                            {"name": "limit", "in": "query", "schema": {"default": "10"}}
                        ]
                    },
                    "post": {
                        "operationId": "createUser",
                        "requestBody": {
                            "content": {
                                "application/json": {"example": {"name": "a"}}
                            }
                        }
                    }
                }
            }
        }"#;
        let collection = parse_openapi(spec).unwrap();
        assert_eq!(collection.name, "示例 API");
        assert_eq!(collection.requests.len(), 2);
        let get_req = collection.requests.iter().find(|r| r.method == "GET").unwrap();
        assert_eq!(get_req.url, "{{base_url}}/users/{{id}}");
        assert_eq!(get_req.folder, vec!["Users".to_string()]);
        assert_eq!(get_req.params.len(), 1);
        assert_eq!(get_req.params[0].key, "limit");
        // base_url + 路径参数 id 都应该被收进集合变量，保证导入后请求立刻能填值。
        assert!(collection.variables.iter().any(|v| v.key == "base_url"));
        assert!(collection.variables.iter().any(|v| v.key == "id"));
        let post_req = collection.requests.iter().find(|r| r.method == "POST").unwrap();
        match &post_req.body {
            RequestBody::Json { content } => assert!(content.contains("\"name\"")),
            _ => panic!("expected json body"),
        }
    }

    #[test]
    fn rejects_non_openapi_input() {
        let err = parse_openapi(r#"{"foo": "bar"}"#);
        assert!(err.is_err());
    }
}
