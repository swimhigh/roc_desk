use serde::{Deserialize, Serialize};

fn default_true() -> bool {
    true
}

fn default_method() -> String {
    "GET".into()
}

/// 一个键值对（Params/Headers/表单字段）——`enabled=false` 表示这一行先保留但不
/// 参与本次请求，对齐 Apifox/Postman 里"取消勾选但不删除"的常见交互。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct KeyValueItem {
    pub key: String,
    #[serde(default)]
    pub value: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiKeyLocation {
    Header,
    Query,
}

/// 认证方式（docs/HTTP_DESKTOP_PLAN.md §3.3 Auth Tab）。MVP 覆盖最常用的三种；
/// OAuth2/Digest/AWS 签名等留作后续扩展，不在首期范围。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuthConfig {
    None,
    Bearer {
        #[serde(default)]
        token: String,
    },
    Basic {
        #[serde(default)]
        username: String,
        #[serde(default)]
        password: String,
    },
    ApiKey {
        #[serde(default)]
        key: String,
        #[serde(default)]
        value: String,
        add_to: ApiKeyLocation,
    },
}

impl Default for AuthConfig {
    fn default() -> Self {
        AuthConfig::None
    }
}

/// 请求体（docs/HTTP_DESKTOP_PLAN.md §3.3 Body Tab 里 none/JSON/表单/原始文本
/// 这几种）。`form_data` 首期只支持文本字段，不支持文件上传（需要额外的文件选择
/// /编码 UI，留作后续增强）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RequestBody {
    None,
    Json {
        #[serde(default)]
        content: String,
    },
    /// XML/纯文本等统一走这里，`content_type` 决定发送时的 Content-Type 和前端
    /// 编辑器的语言高亮，不为每种文本类型单独建枚举分支。
    Raw {
        #[serde(default)]
        content: String,
        #[serde(default = "default_raw_content_type")]
        content_type: String,
    },
    FormUrlEncoded {
        #[serde(default)]
        items: Vec<KeyValueItem>,
    },
    FormData {
        #[serde(default)]
        items: Vec<KeyValueItem>,
    },
}

fn default_raw_content_type() -> String {
    "text/plain".into()
}

impl Default for RequestBody {
    fn default() -> Self {
        RequestBody::None
    }
}

/// 一个请求的完整定义——落盘为 `requests/<folder>/<id>.yaml`
/// （docs/HTTP_DESKTOP_PLAN.md §4.4），YAML 是唯一真相，SQLite 不存正文。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestDef {
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default = "default_method")]
    pub method: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub params: Vec<KeyValueItem>,
    #[serde(default)]
    pub headers: Vec<KeyValueItem>,
    #[serde(default)]
    pub auth: AuthConfig,
    #[serde(default)]
    pub body: RequestBody,
}

impl RequestDef {
    pub fn new(id: String, name: String) -> Self {
        Self {
            id,
            name,
            method: default_method(),
            url: String::new(),
            params: Vec::new(),
            headers: Vec::new(),
            auth: AuthConfig::None,
            body: RequestBody::None,
        }
    }
}

/// 请求树里一个节点——文件夹只是路径前缀，不是独立实体，扫目录时按相对路径
/// 拼出 `folder`（docs/HTTP_DESKTOP_PLAN.md §3.3 左栏请求树）。
#[derive(Debug, Clone, Serialize)]
pub struct RequestSummary {
    pub id: String,
    pub name: String,
    pub method: String,
    /// 相对 `requests/` 的文件夹路径分段，比如请求文件在
    /// `requests/用户模块/子模块/<id>.yaml` 下就是 `["用户模块", "子模块"]`。
    pub folder: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HttpCollectionMeta {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub auth: AuthConfig,
    /// 集合级变量（docs/HTTP_DESKTOP_PLAN.md §4.5 第 4 层）。
    #[serde(default)]
    pub variables: Vec<EnvVar>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HttpCollectionSummary {
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub request_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvVar {
    pub key: String,
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub secret: bool,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvironmentDef {
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub variables: Vec<EnvVar>,
}

impl EnvironmentDef {
    pub fn new(id: String, name: String) -> Self {
        Self {
            id,
            name,
            variables: Vec::new(),
        }
    }
}

/// `http_send_request` 的返回结果——大响应体首期直接整体返回（不做 SQL 桌面那种
/// 事件流分块），前端对超过阈值的响应做截断展示（docs/HTTP_DESKTOP_PLAN.md §7），
/// 后续如果真的遇到超大响应卡 UI 的情况，再补事件流通道。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpExecuteResult {
    pub status: u16,
    pub status_text: String,
    pub headers: Vec<(String, String)>,
    /// 尽力按 UTF-8 解码的正文；`body_is_text=false` 时这里是空字符串，正文在
    /// `body_base64` 里。
    pub body: String,
    pub body_is_text: bool,
    pub body_base64: Option<String>,
    pub duration_ms: u64,
    pub size_bytes: u64,
    /// 变量插值后的最终请求 URL，方便用户核对"这次到底发到哪了"。
    pub resolved_url: String,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct HttpRequestHistoryEntry {
    pub id: String,
    pub workspace_id: String,
    pub collection_slug: String,
    pub request_id: Option<String>,
    pub environment_id: Option<String>,
    pub method: String,
    pub url: String,
    pub status_code: Option<i64>,
    pub duration_ms: Option<i64>,
    pub response_size_bytes: Option<i64>,
    pub error_message: Option<String>,
    pub created_at: String,
}

/// `get_history_detail` 用——列表页（`HttpRequestHistoryEntry`）不带快照正文，
/// 避免一次拉一屏历史记录时把所有请求/响应体都传过来；点开某一条才单独取详情。
#[derive(Debug, Clone, Serialize)]
pub struct HttpRequestHistoryDetail {
    pub id: String,
    pub request_snapshot: String,
    pub response_snapshot: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpWorkspaceTab {
    pub id: String,
    pub workspace_id: String,
    pub collection_slug: String,
    pub request_id: String,
    pub title: String,
    pub sort_order: i64,
    pub updated_at: String,
}
