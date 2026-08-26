-- MCP（Model Context Protocol）服务器配置（REQUIREMENTS.md §3.7"未实现：MCP 客户端"
-- 补上的部分）。敏感的 HTTP 鉴权 token 走系统密钥链（`auth_token_ref`，和
-- ai_providers.api_key_ref 是同一套模式），其余非敏感请求头留在 headers_json 明文存。
CREATE TABLE mcp_servers (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    transport TEXT NOT NULL CHECK (transport IN ('stdio', 'http')),
    command TEXT,            -- stdio: 可执行文件
    args_json TEXT,          -- stdio: JSON 字符串数组
    env_json TEXT,           -- stdio: JSON 字符串对象
    url TEXT,                 -- http: 端点地址
    headers_json TEXT,        -- http: 非敏感请求头 JSON 对象
    auth_token_ref TEXT,      -- http: 敏感 token 的 credential 引用
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL
);
