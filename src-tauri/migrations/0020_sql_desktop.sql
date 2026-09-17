-- SQL 桌面模块（docs/SQL_DESKTOP_PLAN.md §5）。密码等机密只存 credential_ref，
-- 真正的密钥走系统 keyring（见 credential::CredentialStore）。
CREATE TABLE sql_data_sources (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  db_kind TEXT NOT NULL,
  host TEXT NOT NULL,
  port INTEGER,
  database_name TEXT,
  default_schema TEXT,
  username TEXT,
  credential_ref TEXT,
  environment TEXT NOT NULL DEFAULT 'dev',
  group_name TEXT,
  readonly INTEGER NOT NULL DEFAULT 1,
  ssl_required INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  last_used_at TEXT
);

CREATE TABLE sql_query_history (
  id TEXT PRIMARY KEY,
  data_source_id TEXT NOT NULL,
  sql_text TEXT NOT NULL,
  status TEXT NOT NULL,
  duration_ms INTEGER,
  row_count INTEGER,
  error_message TEXT,
  created_at TEXT NOT NULL,
  FOREIGN KEY(data_source_id) REFERENCES sql_data_sources(id) ON DELETE CASCADE
);
CREATE INDEX idx_sql_query_history_ds ON sql_query_history(data_source_id, created_at DESC);

-- 标签页内容的唯一真相是磁盘上的 .sql 文件（`file_path`，相对
-- `<cache_root>/sql/<data_source_id>/queries/`），这张表只存元数据——
-- 详见 docs/SQL_DESKTOP_PLAN.md §4.4。
CREATE TABLE sql_workspace_tabs (
  id TEXT PRIMARY KEY,
  data_source_id TEXT NOT NULL,
  title TEXT NOT NULL,
  file_path TEXT NOT NULL,
  result_view_mode TEXT NOT NULL DEFAULT 'table',
  cursor_json TEXT,
  sort_order INTEGER NOT NULL DEFAULT 0,
  updated_at TEXT NOT NULL,
  FOREIGN KEY(data_source_id) REFERENCES sql_data_sources(id) ON DELETE CASCADE
);
