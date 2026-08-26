-- AI 编程助手权限规则（对齐 OpenCode 的 allow/ask/deny 引擎，见 REQUIREMENTS.md §3.7）。
-- 只接管 run_command / webfetch / mcp:<server>:<tool> 这几个"有副作用、值得反复确认"
-- 的工具维度；write_file/edit_file 继续用已有的 Diff-Accept 流程做门禁，不重复加一层。
-- `tool` 固定取值之一：'run_command' | 'webfetch' | 'mcp'。`pattern` 按 tool 含义不同：
-- run_command 匹配命令文本、webfetch 匹配 URL、mcp 匹配 "<server>:<tool>"（都支持
-- `*`/`?` 通配符，见 coding/permission.rs）。规则按 created_at 升序取，
-- `rules.iter().rev()` 意味着后创建的规则优先命中——用户新加的规则总能覆盖旧规则，
-- 不需要额外的优先级字段。
CREATE TABLE permission_rules (
    id TEXT PRIMARY KEY,
    tool TEXT NOT NULL,
    pattern TEXT NOT NULL,
    decision TEXT NOT NULL CHECK (decision IN ('allow', 'ask', 'deny')),
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL
);
