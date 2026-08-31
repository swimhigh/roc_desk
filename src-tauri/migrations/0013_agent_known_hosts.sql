-- 远程 Windows Agent（AGENT_DESIGN.md §3.1）的 TLS 证书指纹 TOFU 表——和 SSH 的
-- `known_hosts` 是完全不同的信任链条（自签名证书指纹 vs SSH 主机公钥指纹），故意
-- 不合并进同一张表。按 connection_id 而不是 host/port 做主键：Agent 场景下"同一台
-- 机器换个端口"或"同一个 host:port 换了一个连接档案"在语义上应该分别重新确认信任，
-- 不像 SSH 主机指纹那样天然按 host/port 复用。
CREATE TABLE IF NOT EXISTS agent_known_hosts (
    connection_id TEXT PRIMARY KEY,
    fingerprint TEXT NOT NULL,
    trusted_at TEXT NOT NULL
);
