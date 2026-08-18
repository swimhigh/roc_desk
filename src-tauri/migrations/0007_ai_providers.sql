-- AI 问答/编程助手共用的 Provider 配置（DESIGN.md §3.6）。
-- api_key_ref 存的是 credential 引用（真正的 key 走系统密钥链，参考
-- connections 表的 credential_ref 模式），本地 Ollama 等不需要 key 的
-- Provider 该列为 NULL。
CREATE TABLE ai_providers (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    api_base TEXT NOT NULL,
    api_key_ref TEXT,
    model TEXT NOT NULL,
    is_local INTEGER NOT NULL DEFAULT 0,   -- 0/1，本地 Ollama 类不受"数据出境"脱敏策略约束
    created_at TEXT NOT NULL
);
