use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 连接分组/文件夹（DESIGN.md §3.2.2；远程工具模式会话树的文件夹，§3.9）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionGroup {
    pub id: Uuid,
    pub name: String,
    pub parent_id: Option<Uuid>,
}

/// 新建/重命名/移动分组的输入，三个操作复用同一个结构体——移动就是"改 parent_id
/// 不改 name"，重命名反过来，没必要拆成三个不同形状的 struct。
#[derive(Debug, Clone, Deserialize)]
pub struct ConnectionGroupInput {
    pub name: String,
    pub parent_id: Option<Uuid>,
}
