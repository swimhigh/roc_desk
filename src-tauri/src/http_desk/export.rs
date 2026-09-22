//! 导出成 Postman Collection v2.1（docs/HTTP_DESKTOP_PLAN.md §4.7）——`import.rs`
//! 的逆操作，字段映射基本是对称的（同一套 `AuthConfig`/`RequestBody` 变体互转）。
//! 没有做 OpenCollection YAML 导出，留作后续任务；HAR 格式只记录"已经发生过的
//! 请求/响应"，不是给别的工具"导入成可编辑集合"用的格式，本来就不适合当导出目标。

use serde_json::{json, Value};

use crate::error::AppError;
use crate::fsops::FileOps;

use super::model::*;
use super::service;

fn auth_to_postman(auth: &AuthConfig) -> Value {
    match auth {
        AuthConfig::None => json!({ "type": "noauth" }),
        AuthConfig::Bearer { token } => json!({
            "type": "bearer",
            "bearer": [{ "key": "token", "value": token, "type": "string" }],
        }),
        AuthConfig::Basic { username, password } => json!({
            "type": "basic",
            "basic": [
                { "key": "username", "value": username, "type": "string" },
                { "key": "password", "value": password, "type": "string" },
            ],
        }),
        AuthConfig::ApiKey { key, value, add_to } => json!({
            "type": "apikey",
            "apikey": [
                { "key": "key", "value": key, "type": "string" },
                { "key": "value", "value": value, "type": "string" },
                {
                    "key": "in",
                    "value": match add_to {
                        ApiKeyLocation::Header => "header",
                        ApiKeyLocation::Query => "query",
                    },
                    "type": "string",
                },
            ],
        }),
    }
}

fn kv_array_to_postman(items: &[KeyValueItem]) -> Value {
    Value::Array(
        items
            .iter()
            .map(|i| json!({ "key": i.key, "value": i.value, "disabled": !i.enabled }))
            .collect(),
    )
}

fn body_to_postman(body: &RequestBody) -> Option<Value> {
    match body {
        RequestBody::None => None,
        RequestBody::Json { content } => Some(json!({
            "mode": "raw",
            "raw": content,
            "options": { "raw": { "language": "json" } },
        })),
        RequestBody::Raw { content, content_type } => {
            let language = if content_type.contains("xml") {
                "xml"
            } else if content_type.contains("html") {
                "html"
            } else {
                "text"
            };
            Some(json!({
                "mode": "raw",
                "raw": content,
                "options": { "raw": { "language": language } },
            }))
        }
        RequestBody::FormUrlEncoded { items } => Some(json!({
            "mode": "urlencoded",
            "urlencoded": kv_array_to_postman(items),
        })),
        RequestBody::FormData { items } => Some(json!({
            "mode": "formdata",
            "formdata": kv_array_to_postman(items),
        })),
    }
}

fn url_to_postman(url: &str, params: &[KeyValueItem]) -> Value {
    let enabled: Vec<&KeyValueItem> = params.iter().filter(|p| p.enabled && !p.key.is_empty()).collect();
    let raw = if enabled.is_empty() {
        url.to_string()
    } else {
        let qs: Vec<String> = enabled.iter().map(|p| format!("{}={}", p.key, p.value)).collect();
        format!("{url}?{}", qs.join("&"))
    };
    json!({
        "raw": raw,
        "query": params
            .iter()
            .filter(|p| !p.key.is_empty())
            .map(|p| json!({ "key": p.key, "value": p.value, "disabled": !p.enabled }))
            .collect::<Vec<_>>(),
    })
}

fn request_to_postman_item(req: &RequestDef) -> Value {
    let mut request = json!({
        "method": req.method,
        "header": kv_array_to_postman(&req.headers),
        "url": url_to_postman(&req.url, &req.params),
        "auth": auth_to_postman(&req.auth),
    });
    if let Some(body) = body_to_postman(&req.body) {
        request["body"] = body;
    }
    json!({ "name": req.name, "request": request })
}

/// 按 `folder`（相对 `requests/` 的路径分段）把扁平的请求列表重新组装成 Postman
/// `item` 数组该有的嵌套结构——用"按下一级目录名分组再递归"而不是一次性建一整棵
/// 树结构，实现简单、集合规模（几十到几百个请求）下性能完全够用。按插入顺序分组
/// （不用 HashMap），保证同一份集合每次导出的文件夹顺序稳定，不会因为哈希顺序
/// 随机变化导致每次导出的 diff 全是无意义的顺序抖动。
fn build_items(entries: &[(Vec<String>, Value)]) -> Vec<Value> {
    let mut out: Vec<Value> = entries
        .iter()
        .filter(|(folder, _)| folder.is_empty())
        .map(|(_, item)| item.clone())
        .collect();

    let mut folder_order: Vec<String> = Vec::new();
    for (folder, _) in entries.iter().filter(|(f, _)| !f.is_empty()) {
        if !folder_order.contains(&folder[0]) {
            folder_order.push(folder[0].clone());
        }
    }
    for folder_name in folder_order {
        let children: Vec<(Vec<String>, Value)> = entries
            .iter()
            .filter(|(f, _)| !f.is_empty() && f[0] == folder_name)
            .map(|(f, v)| (f[1..].to_vec(), v.clone()))
            .collect();
        out.push(json!({ "name": folder_name, "item": build_items(&children) }));
    }
    out
}

/// 导出指定集合成 Postman Collection v2.1 JSON 字符串——调用方（`commands::http_desk`）
/// 拿到字符串后交给前端写盘，这一层不碰文件系统之外的东西（只读，不改集合本身）。
pub async fn to_postman_collection(
    file_ops: &dyn FileOps,
    workspace_root: &str,
    slug: &str,
) -> Result<String, AppError> {
    let meta = service::get_collection_meta(file_ops, workspace_root, slug).await?;
    let summaries = service::list_requests(file_ops, workspace_root, slug).await?;
    let mut entries = Vec::with_capacity(summaries.len());
    for s in &summaries {
        let req = service::get_request(file_ops, workspace_root, slug, &s.id).await?;
        entries.push((s.folder.clone(), request_to_postman_item(&req)));
    }
    let items = build_items(&entries);
    let variables: Vec<Value> = meta
        .variables
        .iter()
        .map(|v| json!({ "key": v.key, "value": v.value }))
        .collect();
    let collection = json!({
        "info": {
            "name": meta.name,
            "description": meta.description,
            "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json",
        },
        "item": items,
        "variable": variables,
    });
    serde_json::to_string_pretty(&collection).map_err(|e| AppError::Internal(format!("导出失败：{e}")))
}
