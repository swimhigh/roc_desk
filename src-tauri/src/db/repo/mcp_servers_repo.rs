use std::collections::HashMap;

use rusqlite::{params, OptionalExtension};
use uuid::Uuid;

use crate::db::DbPool;
use crate::error::AppError;
use crate::mcp::{McpServer, McpTransportKind};

/// MCP 服务器配置持久化（`0012_mcp_servers.sql`），结构照抄 `ai_providers_repo.rs`；
/// `args`/`env`/`headers` 在 DB 里落地成 JSON 文本列，读出来时反序列化回结构化类型。
pub struct McpServersRepo {
    pool: DbPool,
}

impl McpServersRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub fn create(&self, server: &McpServer) -> Result<(), AppError> {
        let conn = self.pool.get()?;
        conn.execute(
            "INSERT INTO mcp_servers (id, name, transport, command, args_json, env_json, url, headers_json, auth_token_ref, enabled, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                server.id.to_string(),
                server.name,
                transport_str(server.transport),
                server.command,
                serde_json::to_string(&server.args).unwrap_or_default(),
                serde_json::to_string(&server.env).unwrap_or_default(),
                server.url,
                serde_json::to_string(&server.headers).unwrap_or_default(),
                server.auth_token_ref,
                server.enabled as i64,
                server.created_at,
            ],
        )?;
        Ok(())
    }

    pub fn update(&self, server: &McpServer) -> Result<(), AppError> {
        let conn = self.pool.get()?;
        conn.execute(
            "UPDATE mcp_servers SET name = ?2, transport = ?3, command = ?4, args_json = ?5, env_json = ?6,
             url = ?7, headers_json = ?8, auth_token_ref = ?9, enabled = ?10 WHERE id = ?1",
            params![
                server.id.to_string(),
                server.name,
                transport_str(server.transport),
                server.command,
                serde_json::to_string(&server.args).unwrap_or_default(),
                serde_json::to_string(&server.env).unwrap_or_default(),
                server.url,
                serde_json::to_string(&server.headers).unwrap_or_default(),
                server.auth_token_ref,
                server.enabled as i64,
            ],
        )?;
        Ok(())
    }

    pub fn delete(&self, id: Uuid) -> Result<(), AppError> {
        let conn = self.pool.get()?;
        conn.execute("DELETE FROM mcp_servers WHERE id = ?1", params![id.to_string()])?;
        Ok(())
    }

    pub fn get(&self, id: Uuid) -> Result<Option<McpServer>, AppError> {
        let conn = self.pool.get()?;
        conn.query_row(
            "SELECT id, name, transport, command, args_json, env_json, url, headers_json, auth_token_ref, enabled, created_at
             FROM mcp_servers WHERE id = ?1",
            params![id.to_string()],
            Self::map_row,
        )
        .optional()
        .map_err(AppError::from)
    }

    pub fn list(&self) -> Result<Vec<McpServer>, AppError> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare(
            "SELECT id, name, transport, command, args_json, env_json, url, headers_json, auth_token_ref, enabled, created_at
             FROM mcp_servers ORDER BY created_at",
        )?;
        let rows = stmt.query_map([], Self::map_row)?.collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    fn map_row(row: &rusqlite::Row) -> rusqlite::Result<McpServer> {
        let id: String = row.get(0)?;
        let transport: String = row.get(2)?;
        let args_json: Option<String> = row.get(4)?;
        let env_json: Option<String> = row.get(5)?;
        let headers_json: Option<String> = row.get(7)?;
        Ok(McpServer {
            id: Uuid::parse_str(&id).unwrap_or_else(|_| Uuid::nil()),
            name: row.get(1)?,
            transport: if transport == "http" { McpTransportKind::Http } else { McpTransportKind::Stdio },
            command: row.get(3)?,
            args: args_json.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default(),
            env: env_json.and_then(|s| serde_json::from_str::<HashMap<String, String>>(&s).ok()).unwrap_or_default(),
            url: row.get(6)?,
            headers: headers_json.and_then(|s| serde_json::from_str::<HashMap<String, String>>(&s).ok()).unwrap_or_default(),
            auth_token_ref: row.get(8)?,
            enabled: row.get::<_, i64>(9)? != 0,
            created_at: row.get(10)?,
        })
    }
}

fn transport_str(kind: McpTransportKind) -> &'static str {
    match kind {
        McpTransportKind::Stdio => "stdio",
        McpTransportKind::Http => "http",
    }
}
