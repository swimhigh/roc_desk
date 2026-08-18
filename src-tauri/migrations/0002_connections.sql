CREATE TABLE connection_groups (
    id TEXT PRIMARY KEY, name TEXT NOT NULL, parent_id TEXT REFERENCES connection_groups(id)
);
CREATE TABLE connections (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    host TEXT NOT NULL,
    port INTEGER NOT NULL DEFAULT 22,
    username TEXT NOT NULL,
    auth_method TEXT NOT NULL,          -- 'password' | 'key' | 'agent'
    credential_ref TEXT,                -- keyring 条目引用，机密不落库
    group_id TEXT REFERENCES connection_groups(id),
    tags TEXT,                          -- JSON array
    jump_host_id TEXT REFERENCES connections(id),
    last_connected_at TEXT,
    created_at TEXT NOT NULL
);
CREATE TABLE known_hosts (
    host TEXT NOT NULL, port INTEGER NOT NULL,
    fingerprint TEXT NOT NULL, trusted_at TEXT NOT NULL,
    PRIMARY KEY (host, port)
);
