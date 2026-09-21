use chrono::Utc;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::db::DbPool;
use crate::error::AppError;

pub const MAX_EVIDENCE_BYTES: usize = 128 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceEntry {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub target_key: String,
    pub kind: String,
    pub query_hash: String,
    pub path_or_url: String,
    pub version_token: String,
    pub content_hash: String,
    pub payload_json: String,
    pub summary: String,
    pub content: String,
    pub expires_at: Option<String>,
}

pub struct AiEvidenceRepo {
    pool: DbPool,
}

impl AiEvidenceRepo {
    pub fn new(pool: DbPool) -> Self { Self { pool } }

    pub fn get_exact(
        &self, workspace_id: Uuid, target_key: &str, kind: &str,
        query_hash: &str, path_or_url: &str, version_token: &str,
    ) -> Result<Option<EvidenceEntry>, AppError> {
        let conn = self.pool.get()?;
        let row = conn.query_row(
            "SELECT c.id, c.payload_json, c.content_hash, e.summary, e.content, c.expires_at
             FROM ai_evidence_cache c JOIN ai_evidence e ON e.id=c.id
             WHERE c.workspace_id=?1 AND c.target_key=?2 AND c.kind=?3
               AND c.query_hash=?4 AND c.path_or_url=?5 AND c.version_token=?6
               AND c.status='fresh' AND (c.expires_at IS NULL OR c.expires_at > datetime('now'))",
            params![workspace_id.to_string(), target_key, kind, query_hash, path_or_url, version_token],
            |r| Ok(EvidenceEntry {
                id: Uuid::parse_str(&r.get::<_, String>(0)?).unwrap_or_default(),
                workspace_id,
                target_key: target_key.to_string(), kind: kind.to_string(),
                query_hash: query_hash.to_string(), path_or_url: path_or_url.to_string(),
                version_token: version_token.to_string(), content_hash: r.get(2)?,
                payload_json: r.get(1)?, summary: r.get(3)?, content: r.get(4)?, expires_at: r.get(5)?,
            }),
        ).optional()?;
        if let Some(entry) = &row {
            conn.execute("UPDATE ai_evidence_cache SET last_used_at=?2 WHERE id=?1", params![entry.id.to_string(), Utc::now().to_rfc3339()])?;
            conn.execute("UPDATE ai_evidence SET last_used_at=?2 WHERE id=?1", params![entry.id.to_string(), Utc::now().to_rfc3339()])?;
        }
        Ok(row)
    }

    pub fn get_latest_path(&self, workspace_id: Uuid, target_key: &str, path: &str, size: u64) -> Result<Option<EvidenceEntry>, AppError> {
        let conn = self.pool.get()?;
        let suffix = format!("%size={size}");
        let row = conn.query_row(
            "SELECT c.id,c.payload_json,c.content_hash,c.version_token,e.summary,e.content,c.expires_at,c.query_hash
             FROM ai_evidence_cache c JOIN ai_evidence e ON e.id=c.id
             WHERE c.workspace_id=?1 AND c.target_key=?2 AND c.kind='file_snapshot'
               AND c.path_or_url=?3 AND c.version_token LIKE ?4 AND c.status='fresh'
             ORDER BY c.last_used_at DESC LIMIT 1",
            params![workspace_id.to_string(), target_key, path, suffix],
            |r| Ok(EvidenceEntry { id: Uuid::parse_str(&r.get::<_, String>(0)?).unwrap_or_default(), workspace_id, target_key: target_key.into(), kind: "file_snapshot".into(), query_hash: r.get(7)?, path_or_url: path.into(), version_token: r.get(3)?, content_hash: r.get(2)?, payload_json: r.get(1)?, summary: r.get(4)?, content: r.get(5)?, expires_at: r.get(6)? }),
        ).optional()?;
        Ok(row)
    }

    pub fn upsert(&self, entry: &EvidenceEntry) -> Result<(), AppError> {
        let conn = self.pool.get()?;
        let now = Utc::now().to_rfc3339();
        conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| {
            conn.execute("INSERT INTO ai_evidence (id,workspace_id,target_key,kind,path_or_url,version_token,content_hash,summary,content,created_at,last_used_at)
                VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?10)
                ON CONFLICT(id) DO UPDATE SET summary=excluded.summary,content=excluded.content,last_used_at=excluded.last_used_at",
                params![entry.id.to_string(), entry.workspace_id.to_string(), entry.target_key, entry.kind, entry.path_or_url, entry.version_token, entry.content_hash, entry.summary, entry.content, now])?;
            conn.execute("INSERT INTO ai_evidence_cache (id,workspace_id,target_key,kind,query_hash,path_or_url,version_token,content_hash,payload_json,bytes,status,created_at,last_used_at,expires_at)
                VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'fresh',?11,?11,?12)
                ON CONFLICT(workspace_id,target_key,kind,query_hash,path_or_url,version_token) DO UPDATE SET id=excluded.id,content_hash=excluded.content_hash,payload_json=excluded.payload_json,bytes=excluded.bytes,status='fresh',last_used_at=excluded.last_used_at,expires_at=excluded.expires_at",
                params![entry.id.to_string(), entry.workspace_id.to_string(), entry.target_key, entry.kind, entry.query_hash, entry.path_or_url, entry.version_token, entry.content_hash, entry.payload_json, entry.content.len() as i64, now, entry.expires_at])?;
            Ok::<(), rusqlite::Error>(())
        })();
        match result { Ok(()) => { conn.execute_batch("COMMIT")?; Ok(()) }, Err(e) => { let _=conn.execute_batch("ROLLBACK"); Err(e.into()) } }
    }

    pub fn invalidate_path(&self, workspace_id: Uuid, target_key: &str, path: &str) -> Result<(), AppError> {
        self.pool.get()?.execute("UPDATE ai_evidence_cache SET status='invalidated' WHERE workspace_id=?1 AND target_key=?2 AND path_or_url=?3", params![workspace_id.to_string(), target_key, path])?;
        Ok(())
    }

    pub fn invalidate_target(&self, workspace_id: Uuid, target_key: &str) -> Result<(), AppError> {
        self.pool.get()?.execute("UPDATE ai_evidence_cache SET status='invalidated' WHERE workspace_id=?1 AND target_key=?2", params![workspace_id.to_string(), target_key])?;
        Ok(())
    }

    pub fn get_by_id(&self, id: Uuid) -> Result<Option<EvidenceEntry>, AppError> {
        let conn = self.pool.get()?;
        conn.query_row("SELECT id,workspace_id,target_key,kind,content_hash,payload_json,path_or_url,version_token,summary,content,NULL FROM ai_evidence WHERE id=?1", [id.to_string()], |r| {
            Ok(EvidenceEntry { id, workspace_id: Uuid::parse_str(&r.get::<_, String>(1)?).unwrap_or_default(), target_key: r.get(2)?, kind: r.get(3)?, query_hash: String::new(), path_or_url: r.get(6)?, version_token: r.get(7)?, content_hash: r.get(4)?, payload_json: r.get(5)?, summary: r.get(8)?, content: r.get(9)?, expires_at: r.get(10)? })
        }).optional().map_err(AppError::from)
    }

    pub fn search_fts(&self, workspace_id: Uuid, target_key: &str, query: &str, limit: usize) -> Result<Vec<(Uuid, String, String)>, AppError> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare("SELECT e.id,e.path_or_url,e.summary FROM ai_evidence_fts f JOIN ai_evidence e ON e.rowid=f.rowid WHERE ai_evidence_fts MATCH ?1 AND e.workspace_id=?2 AND e.target_key=?3 ORDER BY bm25(ai_evidence_fts) LIMIT ?4")?;
        let rows = stmt.query_map(params![query, workspace_id.to_string(), target_key, limit as i64], |r| Ok((Uuid::parse_str(&r.get::<_, String>(0)?).unwrap_or_default(), r.get(1)?, r.get(2)?)))?.collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}
