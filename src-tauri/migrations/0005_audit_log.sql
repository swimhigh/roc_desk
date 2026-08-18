-- run_command 审计日志（DESIGN.md §3.8.2.1：所有调用含被拒绝的都要记录，
-- 时间/目标主机/命令/结果/AI 会话 ID，可在设置中查看/导出）。
CREATE TABLE command_audit_log (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    target_label TEXT NOT NULL,       -- '本地' 或远程主机名，供审计时不需要再关联查询
    command TEXT NOT NULL,
    outcome TEXT NOT NULL,            -- 'blocked' | 'rejected' | 'executed'
    output_summary TEXT,              -- 执行成功时的输出前若干字符，被拒绝/拦截时为空
    created_at TEXT NOT NULL
);
