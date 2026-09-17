-- HTTP 桌面模块（docs/HTTP_DESKTOP_PLAN.md）。
--
-- 不新增"集合"/"环境"表：集合、环境、请求的内容都落在已打开工作区目录下的
-- `.rock_desk/http/` 子目录（YAML 文件，见 src-tauri/src/http_desk/service.rs），
-- 集合列表靠扫目录发现，不靠数据库查询。这里只存两张纯 UI/审计状态表，且必须和
-- `workspaces` 表同一个数据库文件（0006_workspaces.sql）才能让 FK 生效——`workspaces`
-- 是独立的数据库文件（db/migrate.rs 的 WORKSPACES_MIGRATIONS），本迁移也要挂在那个
-- 列表下，不能加进主库的 MAIN_MIGRATIONS。
CREATE TABLE http_workspace_tabs (
  id TEXT PRIMARY KEY,
  workspace_id TEXT NOT NULL,
  collection_slug TEXT NOT NULL,
  request_id TEXT NOT NULL,
  title TEXT NOT NULL,
  sort_order INTEGER NOT NULL DEFAULT 0,
  updated_at TEXT NOT NULL,
  FOREIGN KEY(workspace_id) REFERENCES workspaces(id) ON DELETE CASCADE
);

-- 请求历史：MVP 阶段直接把请求/响应快照序列化成 JSON 存进 `request_snapshot`/
-- `response_snapshot` 两列，不走 HTTP_DESKTOP_PLAN.md §4.4 提到的
-- `fallback_cache_dir` 文件落盘方案——省掉一层文件 IO，历史列表体量可控（响应体
-- 超过阈值会先截断再入库，见 service.rs），后续如果历史数据量成为问题，再迁移到
-- 文件落盘也不影响这张表已有的行。
CREATE TABLE http_request_history (
  id TEXT PRIMARY KEY,
  workspace_id TEXT NOT NULL,
  collection_slug TEXT NOT NULL,
  request_id TEXT,
  environment_id TEXT,
  method TEXT NOT NULL,
  url TEXT NOT NULL,
  status_code INTEGER,
  duration_ms INTEGER,
  response_size_bytes INTEGER,
  error_message TEXT,
  request_snapshot TEXT NOT NULL,
  response_snapshot TEXT,
  created_at TEXT NOT NULL,
  FOREIGN KEY(workspace_id) REFERENCES workspaces(id) ON DELETE CASCADE
);
CREATE INDEX idx_http_request_history_workspace ON http_request_history(workspace_id, created_at DESC);
