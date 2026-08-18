-- 网页浏览历史（DESIGN.md §3.5）。网页本身用独立 WebviewWindow 承载（安全隔离，
-- 不和主窗口共享 Tauri IPC 上下文），这张表只记"访问过什么"，供历史记录面板展示
-- 和"点击重新打开"用。
CREATE TABLE browser_history (
    id TEXT PRIMARY KEY,
    url TEXT NOT NULL,
    title TEXT,
    visited_at TEXT NOT NULL
);
CREATE INDEX idx_browser_history_visited_at ON browser_history(visited_at DESC);
