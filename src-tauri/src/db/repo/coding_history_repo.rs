use std::io::{Read, Write};

use chrono::Utc;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::db::DbPool;
use crate::error::AppError;

/// gzip 魔数——存库前判断"这段字节本来就已经是压缩过的"，避免对已经压缩的
/// 内容再压一遍；读出来时判断"这是老版本遗留的明文 JSON 还是新版本压缩过的"，
/// 两种都要认得（见下面 `decompress_json` 的回退逻辑）。
const GZIP_MAGIC: [u8; 2] = [0x1f, 0x8b];

/// 2026-10 用户反馈：`coding_history` 表个别记录能长到几十、上百 MB
/// （分析一个二进制文件之类的长任务，几百次工具调用的完整结果原样塞进
/// `messages_json`/`timeline_json`，详见 `agent_llm::cap_tool_result` 单次
/// 20000 字符上限只管住"一次"，管不住"一共调用几百次"）。JSON 文本里
/// 大量重复结构（字段名、工具调用的壳、相似的路径前缀）天然适合压缩，
/// gzip 实测对这类内容能压到 1/5~1/10——不改变"存了什么"，只改变"怎么存"。
fn compress_json(value: &serde_json::Value) -> Result<Vec<u8>, AppError> {
    let text = value.to_string();
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(text.as_bytes())?;
    Ok(encoder.finish()?)
}

/// 兼容两种存法：老版本（这次改动之前）落库的是明文 JSON 文本，新版本是
/// gzip 压缩过的字节——用 gzip 魔数判断走哪条路径，不需要额外的"版本号"
/// 字段或者强制迁移旧数据才能读。`rusqlite` 的 `Vec<u8>` getter 对
/// TEXT/BLOB 两种存储类型都能读出原始字节（SQLite 本身对存储类型的要求
/// 很松），所以老的明文行不需要改列类型就能照样读。
fn decompress_json(bytes: &[u8]) -> serde_json::Value {
    if bytes.starts_with(&GZIP_MAGIC) {
        let mut decoder = GzDecoder::new(bytes);
        let mut text = String::new();
        if decoder.read_to_string(&mut text).is_ok() {
            return serde_json::from_str(&text).unwrap_or_default();
        }
        return serde_json::Value::default();
    }
    serde_json::from_slice(bytes).unwrap_or_default()
}

/// 读出一列的原始字节，不管这一列实际的 SQLite 存储类型是 TEXT 还是 BLOB——
/// `rusqlite` 的 `Vec<u8>` getter 只认 BLOB，对着老版本（这次改动之前）留下的
/// TEXT 存储类型的行会直接报 `InvalidType` 错误（不是静默兼容，实测会报错，
/// 不是猜测）。这里用 `get_ref` 拿到 `ValueRef` 后手动按两种类型各自取字节，
/// 新老两种存法才能用同一套读取代码处理。
fn column_bytes(row: &rusqlite::Row<'_>, idx: usize) -> rusqlite::Result<Vec<u8>> {
    use rusqlite::types::ValueRef;
    match row.get_ref(idx)? {
        ValueRef::Blob(b) => Ok(b.to_vec()),
        ValueRef::Text(t) => Ok(t.to_vec()),
        ValueRef::Null => Ok(Vec::new()),
        other => Err(rusqlite::Error::InvalidColumnType(
            idx,
            format!("{other:?}"),
            other.data_type(),
        )),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodingHistoryInput {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub title: String,
    pub provider_id: Uuid,
    pub provider_label: String,
    pub model: String,
    pub mode: String,
    pub timeline: serde_json::Value,
    pub changes: serde_json::Value,
    /// 发给 AI 的真实对话上下文（`CodingSession` 内部的 `messages`）——前端不知道
    /// 也不需要填这个字段（`#[serde(default)]`：前端传来的 JSON 里没有这个键也能
    /// 反序列化成功），`commands::coding::coding_history_save` 在落库前会用当前
    /// 存活会话的最新 `messages` 覆盖它。`coding_history_resume` 靠这份数据把
    /// 历史会话真正续上，而不只是回放 `changes`/`timeline`。
    #[serde(default)]
    pub messages: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct CodingHistorySummary {
    pub id: Uuid,
    pub title: String,
    /// 摘要里也带上 provider id（不只是 `provider_label`）——前端
    /// `restoreOrStart` 需要在"只查本地缓存列表、还没去远程工作区对账"的阶段就
    /// 知道最近一条历史用的是哪个 Provider，好用它直接起会话；否则就得先拉一遍
    /// 完整详情（`coding_history_get`，会跨网络读一份可能上兆的快照）才能知道，
    /// 那正是"打开 AI 工具要等十几秒"的一段耗时。
    pub provider_id: Uuid,
    pub provider_label: String,
    pub model: String,
    pub mode: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CodingHistoryDetail {
    #[serde(flatten)]
    pub summary: CodingHistorySummary,
    pub workspace_id: Uuid,
    pub timeline: serde_json::Value,
    pub changes: serde_json::Value,
    /// 不通过 IPC 序列化给前端（前端没有必要、也不应该直接看到原始 LLM 消息
    /// 上下文）——只在后端内部 `coding_history_resume` 里读取用来重建会话。
    #[serde(skip)]
    pub messages: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceHistorySnapshot {
    pub input: CodingHistoryInput,
    pub created_at: String,
    pub updated_at: String,
}

pub struct CodingHistoryRepo {
    pool: DbPool,
}

impl CodingHistoryRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub fn save(&self, input: &CodingHistoryInput) -> Result<(), AppError> {
        let conn = self.pool.get()?;
        let now = Utc::now().to_rfc3339();
        let timeline = compress_json(&input.timeline)?;
        let changes = compress_json(&input.changes)?;
        let messages = compress_json(&input.messages)?;
        conn.execute(
            "INSERT INTO coding_history (id, workspace_id, title, provider_id, provider_label, model, mode, timeline_json, changes_json, messages_json, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11)
             ON CONFLICT(id) DO UPDATE SET title=excluded.title, provider_id=excluded.provider_id,
             provider_label=excluded.provider_label, model=excluded.model, mode=excluded.mode,
             timeline_json=excluded.timeline_json, changes_json=excluded.changes_json,
             messages_json=excluded.messages_json, updated_at=excluded.updated_at",
            params![input.id.to_string(), input.workspace_id.to_string(), input.title, input.provider_id.to_string(),
                input.provider_label, input.model, input.mode, timeline, changes,
                messages, now],
        )?;
        Ok(())
    }

    pub fn list(&self, workspace_id: Uuid) -> Result<Vec<CodingHistorySummary>, AppError> {
        let conn = self.pool.get()?;
        let mut stmt = conn.prepare("SELECT id, title, provider_id, provider_label, model, mode, created_at, updated_at FROM coding_history WHERE workspace_id=?1 ORDER BY updated_at DESC")?;
        let rows = stmt
            .query_map([workspace_id.to_string()], |r| {
                Ok(CodingHistorySummary {
                    id: Uuid::parse_str(&r.get::<_, String>(0)?).unwrap_or_default(),
                    title: r.get(1)?,
                    provider_id: Uuid::parse_str(&r.get::<_, String>(2)?).unwrap_or_default(),
                    provider_label: r.get(3)?,
                    model: r.get(4)?,
                    mode: r.get(5)?,
                    created_at: r.get(6)?,
                    updated_at: r.get(7)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn get(&self, id: Uuid) -> Result<Option<CodingHistoryDetail>, AppError> {
        let conn = self.pool.get()?;
        conn.query_row("SELECT workspace_id, title, provider_id, provider_label, model, mode, timeline_json, changes_json, created_at, updated_at, messages_json FROM coding_history WHERE id=?1", [id.to_string()], |r| {
            let parse_uuid = |s: String| Uuid::parse_str(&s).unwrap_or_default();
            let timeline = column_bytes(r, 6)?;
            let changes = column_bytes(r, 7)?;
            let messages = column_bytes(r, 10)?;
            Ok(CodingHistoryDetail {
                summary: CodingHistorySummary { id, title: r.get(1)?, provider_id: parse_uuid(r.get(2)?), provider_label: r.get(3)?, model: r.get(4)?, mode: r.get(5)?, created_at: r.get(8)?, updated_at: r.get(9)? },
                workspace_id: parse_uuid(r.get(0)?),
                timeline: decompress_json(&timeline), changes: decompress_json(&changes),
                messages: decompress_json(&messages),
            })
        }).optional().map_err(AppError::from)
    }

    pub fn import_snapshot(&self, snapshot: &WorkspaceHistorySnapshot) -> Result<(), AppError> {
        let input = &snapshot.input;
        let timeline = compress_json(&input.timeline)?;
        let changes = compress_json(&input.changes)?;
        let messages = compress_json(&input.messages)?;
        self.pool.get()?.execute(
            "INSERT INTO coding_history (id, workspace_id, title, provider_id, provider_label, model, mode, timeline_json, changes_json, messages_json, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT(id) DO UPDATE SET title=excluded.title, provider_id=excluded.provider_id,
             provider_label=excluded.provider_label, model=excluded.model, mode=excluded.mode,
             timeline_json=excluded.timeline_json, changes_json=excluded.changes_json,
             messages_json=excluded.messages_json,
             updated_at=excluded.updated_at WHERE excluded.updated_at > coding_history.updated_at",
            params![input.id.to_string(), input.workspace_id.to_string(), input.title, input.provider_id.to_string(),
                input.provider_label, input.model, input.mode, timeline, changes,
                messages, snapshot.created_at, snapshot.updated_at],
        )?;
        Ok(())
    }

    pub fn rename(&self, id: Uuid, title: &str) -> Result<(), AppError> {
        self.pool.get()?.execute(
            "UPDATE coding_history SET title=?2, updated_at=?3 WHERE id=?1",
            params![id.to_string(), title, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    pub fn delete(&self, id: Uuid) -> Result<(), AppError> {
        self.pool
            .get()?
            .execute("DELETE FROM coding_history WHERE id=?1", [id.to_string()])?;
        Ok(())
    }

    /// 一次性"压缩存量数据 + 回收空间"：
    /// 1. 老版本（这次改动之前）落库的记录还是明文 JSON，这里统一解压再用
    ///    `compress_json` 重新写回——对已经是 gzip 的行直接跳过，不做没意义的
    ///    "解压再压缩"。
    /// 2. `DELETE`/`UPDATE` 本身不会让 SQLite 文件变小（空出来的页留在文件里
    ///    等着复用，不会还给操作系统），所以最后跑一次 `VACUUM` 把这些空闲页
    ///    真正收回去——这也是 2026-10 用户反馈"598MB 的库，freelist 就占了
    ///    140MB"那部分的唯一回收办法。
    ///
    /// 这是用户在设置里手动触发的维护操作，不是每次启动自动跑——扫一遍全表、
    /// 重写大量记录、再 VACUUM 整个文件，在历史很大时本身就要跑几秒到几十秒，
    /// 放进启动路径会拖慢每次打开 app，不划算。
    pub fn compact_storage(&self) -> Result<CompactStorageStats, AppError> {
        let conn = self.pool.get()?;
        let before_bytes = Self::db_byte_size(&conn)?;

        let mut stmt = conn
            .prepare("SELECT id, timeline_json, changes_json, messages_json FROM coding_history")?;
        let rows: Vec<(String, Vec<u8>, Vec<u8>, Vec<u8>)> = stmt
            .query_map([], |r| {
                Ok((r.get(0)?, column_bytes(r, 1)?, column_bytes(r, 2)?, column_bytes(r, 3)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        drop(stmt);

        let mut recompressed = 0u32;
        for (id, timeline, changes, messages) in rows {
            if timeline.starts_with(&GZIP_MAGIC)
                && changes.starts_with(&GZIP_MAGIC)
                && messages.starts_with(&GZIP_MAGIC)
            {
                continue;
            }
            let timeline = compress_json(&decompress_json(&timeline))?;
            let changes = compress_json(&decompress_json(&changes))?;
            let messages = compress_json(&decompress_json(&messages))?;
            conn.execute(
                "UPDATE coding_history SET timeline_json=?2, changes_json=?3, messages_json=?4 WHERE id=?1",
                params![id, timeline, changes, messages],
            )?;
            recompressed += 1;
        }

        conn.execute_batch("VACUUM;")?;
        let after_bytes = Self::db_byte_size(&conn)?;
        Ok(CompactStorageStats {
            before_bytes,
            after_bytes,
            recompressed_rows: recompressed,
        })
    }

    fn db_byte_size(conn: &rusqlite::Connection) -> Result<i64, AppError> {
        let page_count: i64 = conn.query_row("PRAGMA page_count", [], |r| r.get(0))?;
        let page_size: i64 = conn.query_row("PRAGMA page_size", [], |r| r.get(0))?;
        Ok(page_count * page_size)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CompactStorageStats {
    pub before_bytes: i64,
    pub after_bytes: i64,
    pub recompressed_rows: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compress_then_decompress_round_trips() {
        let original = serde_json::json!({"role": "tool", "content": "a".repeat(50_000)});
        let compressed = compress_json(&original).unwrap();
        assert!(compressed.starts_with(&GZIP_MAGIC), "compressed output should start with the gzip magic bytes");
        assert!(
            compressed.len() < original.to_string().len() / 4,
            "highly repetitive content should compress to well under 1/4 its original size"
        );
        assert_eq!(decompress_json(&compressed), original);
    }

    /// 老版本（这次改动之前）落库的是明文 JSON 字节，不是 gzip——`decompress_json`
    /// 必须认得这种"没有魔数"的情况，不然升级后打开旧历史记录全部读出空值。
    #[test]
    fn decompress_falls_back_to_plain_json_for_legacy_rows() {
        let original = serde_json::json!({"legacy": true, "rows": [1, 2, 3]});
        let plain_bytes = original.to_string().into_bytes();
        assert!(!plain_bytes.starts_with(&GZIP_MAGIC));
        assert_eq!(decompress_json(&plain_bytes), original);
    }

    #[test]
    fn decompress_handles_empty_and_garbage_input_without_panicking() {
        assert_eq!(decompress_json(&[]), serde_json::Value::Null);
        // 两个字节恰好撞上 gzip 魔数、但后面不是合法 gzip 流——必须走
        // "解压失败就返回默认值"这条路径，不能 panic。
        assert_eq!(decompress_json(&GZIP_MAGIC), serde_json::Value::Null);
    }
}
