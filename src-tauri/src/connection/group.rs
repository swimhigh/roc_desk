use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 连接分组/文件夹（DESIGN.md §3.2.2）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionGroup {
    pub id: Uuid,
    pub name: String,
    pub parent_id: Option<Uuid>,
}
