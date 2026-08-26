CREATE TABLE coding_history (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL,
    title TEXT NOT NULL,
    provider_id TEXT NOT NULL,
    provider_label TEXT NOT NULL,
    model TEXT NOT NULL,
    mode TEXT NOT NULL,
    timeline_json TEXT NOT NULL,
    changes_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX idx_coding_history_workspace_updated
ON coding_history(workspace_id, updated_at DESC);
