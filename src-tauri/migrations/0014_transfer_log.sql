-- SFTP/Agent 文件传输日志（用户 2026-09-01 需求："传输日志需要记录，并可在界面上
-- 查询追溯"）——和 command_audit_log 是同一种"尽力而为审计"模式：只记录，不影响
-- 传输本身的执行流程。
CREATE TABLE transfer_log (
    id TEXT PRIMARY KEY,
    protocol TEXT NOT NULL,          -- 'sftp' | 'agent'
    direction TEXT NOT NULL,         -- 'upload' | 'download'
    profile_id TEXT,                 -- 连接档案 id；连接被删除后这里可能查不到名字，
                                      -- 所以下面单独存一份当时的名字快照
    profile_name TEXT NOT NULL,
    local_path TEXT NOT NULL,
    remote_path TEXT NOT NULL,
    is_dir INTEGER NOT NULL,
    file_count INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL,            -- 'completed' | 'cancelled' | 'failed'
    error_message TEXT,
    started_at TEXT NOT NULL,
    finished_at TEXT NOT NULL
);

CREATE INDEX idx_transfer_log_finished_at ON transfer_log(finished_at DESC);
