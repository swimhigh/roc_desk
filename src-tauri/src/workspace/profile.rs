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
    /// 这个工作区里 SFTP/Agent 双栏文件浏览器最后停留的两边目录（用户需求：
    /// "下次启动工作区中的SFTP或文件传输时，直接定位到最后记住的目录"）。NULL
    /// 表示还没打开过，前端退回默认值（远程退回 `root_path`，本地退回主目录）。
    pub last_sftp_local_path: Option<String>,
    pub last_sftp_remote_path: Option<String>,
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
