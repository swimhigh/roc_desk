use rusqlite::{params, OptionalExtension};
use uuid::Uuid;

use crate::connection::group::ConnectionGroup;
use crate::db::DbPool;
use crate::error::AppError;

pub struct ConnectionGroupsRepo {
    pool: DbPool,
}

impl ConnectionGroupsRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub fn list(&self) -> Result<Vec<ConnectionGroup>, AppError> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare("SELECT id, name, parent_id FROM connection_groups ORDER BY name")?;
        let rows = stmt.query_map([], Self::map_row)?.collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn get(&self, id: Uuid) -> Result<Option<ConnectionGroup>, AppError> {
        let conn = self.pool.get()?;
        conn.query_row(
            "SELECT id, name, parent_id FROM connection_groups WHERE id = ?1",
            params![id.to_string()],
            Self::map_row,
        )
        .optional()
        .map_err(AppError::from)
    }

    pub fn create(&self, group: &ConnectionGroup) -> Result<(), AppError> {
        let conn = self.pool.get()?;
        conn.execute(
            "INSERT INTO connection_groups (id, name, parent_id) VALUES (?1, ?2, ?3)",
            params![group.id.to_string(), group.name, group.parent_id.map(|p| p.to_string())],
        )?;
        Ok(())
    }

    pub fn update(&self, group: &ConnectionGroup) -> Result<(), AppError> {
        let conn = self.pool.get()?;
        conn.execute(
            "UPDATE connection_groups SET name = ?2, parent_id = ?3 WHERE id = ?1",
            params![group.id.to_string(), group.name, group.parent_id.map(|p| p.to_string())],
        )?;
        Ok(())
    }

    /// 删除一个分组：子分组一并上移到被删分组的 parent（不是级联删除——文件夹结构
    /// "塌陷"一层，而不是连同里面的东西一起消失），直属连接的 group_id 置空（掉回
    /// 会话树的"未分组"根部），本身不删除任何连接档案。
    pub fn delete(&self, id: Uuid) -> Result<(), AppError> {
        let conn = self.pool.get()?;
        let parent_id: Option<String> = conn
            .query_row(
                "SELECT parent_id FROM connection_groups WHERE id = ?1",
                params![id.to_string()],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        conn.execute(
            "UPDATE connection_groups SET parent_id = ?2 WHERE parent_id = ?1",
            params![id.to_string(), parent_id],
        )?;
        conn.execute(
            "UPDATE connections SET group_id = NULL WHERE group_id = ?1",
            params![id.to_string()],
        )?;
        conn.execute("DELETE FROM connection_groups WHERE id = ?1", params![id.to_string()])?;
        Ok(())
    }

    fn map_row(row: &rusqlite::Row) -> rusqlite::Result<ConnectionGroup> {
        let id: String = row.get(0)?;
        let parent_id: Option<String> = row.get(2)?;
        Ok(ConnectionGroup {
            id: Uuid::parse_str(&id).unwrap_or_else(|_| Uuid::nil()),
            name: row.get(1)?,
            parent_id: parent_id.and_then(|s| Uuid::parse_str(&s).ok()),
        })
    }
}
