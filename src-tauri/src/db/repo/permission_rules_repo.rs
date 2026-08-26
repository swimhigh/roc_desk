use rusqlite::params;
use uuid::Uuid;

use crate::coding::permission::{Decision, PermissionRule};
use crate::db::DbPool;
use crate::error::AppError;

/// 权限规则持久化（`0011_permission_rules.sql`），结构照抄 `ai_providers_repo.rs`
/// 的 create/delete/list 三件套——规则只需要增删查，没有"编辑"场景（前端删了重加）。
pub struct PermissionRulesRepo {
    pool: DbPool,
}

impl PermissionRulesRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub fn create(&self, rule: &PermissionRule) -> Result<(), AppError> {
        let conn = self.pool.get()?;
        conn.execute(
            "INSERT INTO permission_rules (id, tool, pattern, decision, enabled, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                rule.id.to_string(),
                rule.tool,
                rule.pattern,
                rule.decision.as_str(),
                rule.enabled as i64,
                rule.created_at,
            ],
        )?;
        Ok(())
    }

    pub fn delete(&self, id: Uuid) -> Result<(), AppError> {
        let conn = self.pool.get()?;
        conn.execute("DELETE FROM permission_rules WHERE id = ?1", params![id.to_string()])?;
        Ok(())
    }

    /// 按创建时间升序返回——调用方（`PermissionEngine::decide`）反向遍历，
    /// 让"后创建的规则优先命中"，不需要额外的优先级字段。
    pub fn list(&self) -> Result<Vec<PermissionRule>, AppError> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT id, tool, pattern, decision, enabled, created_at
             FROM permission_rules WHERE enabled = 1 ORDER BY created_at",
        )?;
        let rows = stmt.query_map([], Self::map_row)?.collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    fn map_row(row: &rusqlite::Row) -> rusqlite::Result<PermissionRule> {
        let id: String = row.get(0)?;
        let decision: String = row.get(3)?;
        Ok(PermissionRule {
            id: Uuid::parse_str(&id).unwrap_or_else(|_| Uuid::nil()),
            tool: row.get(1)?,
            pattern: row.get(2)?,
            decision: Decision::from_str(&decision),
            enabled: row.get::<_, i64>(4)? != 0,
            created_at: row.get(5)?,
        })
    }
}
