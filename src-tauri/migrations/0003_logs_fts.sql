-- 字段设计见 DESIGN.md §3.4.2：非文本字段标 UNINDEXED，避免被当全文分词。
CREATE VIRTUAL TABLE logs USING fts5(
    content, file_path UNINDEXED, line_number UNINDEXED,
    timestamp UNINDEXED, log_level UNINDEXED, host_name UNINDEXED,
    tokenize = 'unicode61'
);
CREATE TABLE log_import_jobs (
    id TEXT PRIMARY KEY, host_name TEXT, file_path TEXT,
    status TEXT, bytes_total INTEGER, bytes_done INTEGER, created_at TEXT
);
