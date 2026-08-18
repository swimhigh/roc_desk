-- 工作区档案 + 最近打开列表（DESIGN.md §3.1.1，CODE_DESIGN.md §五）
-- 编号跳到 0006 是有意为之：与 CODE_DESIGN.md 中规划的完整迁移序号保持一致，
-- 0002_connections / 0003_logs_fts / 0004_coding_sessions / 0005_audit_log
-- 会在对应功能（SSH、日志搜索、AI 编程助手）实现时补上，迁移系统按文件名顺序
-- 应用，中间有空号不影响正确性。
CREATE TABLE workspaces (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,                 -- 'local' | 'remote'
    root_path TEXT NOT NULL,
    connection_id TEXT,                 -- kind='remote' 时必填，将来引用 connections(id)
    display_name TEXT NOT NULL,
    last_opened_at TEXT,
    created_at TEXT NOT NULL
);
