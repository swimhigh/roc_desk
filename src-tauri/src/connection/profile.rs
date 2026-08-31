use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 连接档案（DESIGN.md §3.2.2）。敏感字段（密码/私钥口令）不在这个结构体里——
/// `credential_ref` 只是系统密钥链条目的引用 key，实际机密经 `CredentialStore` 存取。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionProfile {
    pub id: Uuid,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_method: AuthMethod,
    /// keyring 条目引用，形如 `ssh:{id}:secret`；None 表示 Agent 认证，无需存密文
    pub credential_ref: Option<String>,
    pub group_id: Option<Uuid>,
    pub tags: Vec<String>,
    pub jump_host_id: Option<Uuid>,
    /// SSH 还是 RDP（远程工具模式会话树，DESIGN.md §3.9）——两者共用这一张表和同一套
    /// 分组，只是可用的动作不同：SSH 能开终端/SFTP，RDP 只能开远程桌面。
    pub protocol: Protocol,
    /// 协议相关的少量额外字段，目前只有 RDP 用（domain/width/height/color_depth），
    /// 犯不上为这几个字段单独开列，JSON 存。
    pub options: Option<serde_json::Value>,
    pub last_connected_at: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Ssh,
    Rdp,
    /// 远程 Windows Agent（AGENT_DESIGN.md）：不需要 OpenSSH Server 的第三种远程
    /// 后端，认证靠配对令牌而不是 `AuthMethod`（`username`/`auth_method` 字段对
    /// 这个协议不适用，UI 层隐藏，`ConnectionProfileInput.secret` 装的是配对令牌）。
    Agent,
}

impl Protocol {
    pub fn as_str(&self) -> &'static str {
        match self {
            Protocol::Ssh => "ssh",
            Protocol::Rdp => "rdp",
            Protocol::Agent => "agent",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "rdp" => Protocol::Rdp,
            "agent" => Protocol::Agent,
            _ => Protocol::Ssh,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthMethod {
    Password,
    Key,
    Agent,
}

impl AuthMethod {
    pub fn as_str(&self) -> &'static str {
        match self {
            AuthMethod::Password => "password",
            AuthMethod::Key => "key",
            AuthMethod::Agent => "agent",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "password" => AuthMethod::Password,
            "agent" => AuthMethod::Agent,
            _ => AuthMethod::Key,
        }
    }
}

/// 创建/编辑连接档案的输入（`secret` 是明文，只在这一次调用中经过内存，
/// 立刻写入 keyring，不会随 `ConnectionProfile` 一起被序列化回前端或落库）。
#[derive(Debug, Clone, Deserialize)]
pub struct ConnectionProfileInput {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_method: AuthMethod,
    pub secret: Option<String>,
    pub group_id: Option<Uuid>,
    pub tags: Vec<String>,
    pub jump_host_id: Option<Uuid>,
    #[serde(default)]
    pub protocol: Protocol,
    #[serde(default)]
    pub options: Option<serde_json::Value>,
}

impl Default for Protocol {
    fn default() -> Self {
        Protocol::Ssh
    }
}
