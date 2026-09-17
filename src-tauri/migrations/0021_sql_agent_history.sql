CREATE TABLE IF NOT EXISTS sql_agent_history (
    id TEXT PRIMARY KEY,
    data_source_id TEXT NOT NULL,
    title TEXT NOT NULL,
    provider_id TEXT NOT NULL,
    provider_label TEXT NOT NULL,
    model TEXT NOT NULL,
    timeline_json TEXT NOT NULL,
    messages_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_sql_agent_history_ds ON sql_agent_history(data_source_id, updated_at DESC);
