use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 工作区档案（CODE_DESIGN.md §3.8 / DESIGN.md §3.1.1，应用入口）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceProfile {
    pub id: Uuid,
    pub kind: WorkspaceKind,
    /// 本地绝对路径，或（未来）远程主机上的绝对路径
    pub root_path: String,
    /// kind = Remote 时必填，关联 ConnectionProfile（SSH 模块尚未实现，先保留字段）
    pub connection_id: Option<Uuid>,
    pub display_name: String,
    pub last_opened_at: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkspaceKind {
    Local,
    Remote,
}

impl WorkspaceKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            WorkspaceKind::Local => "local",
            WorkspaceKind::Remote => "remote",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "remote" => WorkspaceKind::Remote,
            _ => WorkspaceKind::Local,
        }
    }
}
