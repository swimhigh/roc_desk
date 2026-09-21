CREATE TABLE ai_evidence_cache (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL,
    target_key TEXT NOT NULL,
    kind TEXT NOT NULL,
    query_hash TEXT NOT NULL,
    path_or_url TEXT NOT NULL,
    version_token TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    bytes INTEGER NOT NULL,
    status TEXT NOT NULL DEFAULT 'fresh',
    created_at TEXT NOT NULL,
    last_used_at TEXT NOT NULL,
    expires_at TEXT,
    UNIQUE(workspace_id, target_key, kind, query_hash, path_or_url, version_token)
);

CREATE INDEX idx_ai_evidence_cache_lookup
ON ai_evidence_cache(workspace_id, target_key, kind, query_hash, status);

CREATE INDEX idx_ai_evidence_cache_lru
ON ai_evidence_cache(workspace_id, last_used_at);

CREATE TABLE ai_evidence (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL,
    target_key TEXT NOT NULL,
    kind TEXT NOT NULL,
    path_or_url TEXT NOT NULL,
    version_token TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    summary TEXT NOT NULL DEFAULT '',
    content TEXT NOT NULL,
    created_at TEXT NOT NULL,
    last_used_at TEXT NOT NULL
);

CREATE VIRTUAL TABLE ai_evidence_fts USING fts5(
    path_or_url,
    summary,
    content,
    content='ai_evidence',
    content_rowid='rowid',
    tokenize='unicode61 remove_diacritics 2'
);

CREATE TRIGGER ai_evidence_ai AFTER INSERT ON ai_evidence BEGIN
    INSERT INTO ai_evidence_fts(rowid, path_or_url, summary, content)
    VALUES (new.rowid, new.path_or_url, new.summary, new.content);
END;

CREATE TRIGGER ai_evidence_ad AFTER DELETE ON ai_evidence BEGIN
    INSERT INTO ai_evidence_fts(ai_evidence_fts, rowid, path_or_url, summary, content)
    VALUES ('delete', old.rowid, old.path_or_url, old.summary, old.content);
END;

CREATE TRIGGER ai_evidence_au AFTER UPDATE ON ai_evidence BEGIN
    INSERT INTO ai_evidence_fts(ai_evidence_fts, rowid, path_or_url, summary, content)
    VALUES ('delete', old.rowid, old.path_or_url, old.summary, old.content);
    INSERT INTO ai_evidence_fts(rowid, path_or_url, summary, content)
    VALUES (new.rowid, new.path_or_url, new.summary, new.content);
END;
