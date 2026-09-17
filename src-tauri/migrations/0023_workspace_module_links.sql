-- 工作模块首页卡片"添加/选择工作区"关联表（2026-09 用户反馈：HTTP 桌面不应该
-- 默认展示所有工作区目录，而是要显式"添加"——选择已有的或者新建；同理，从 HTTP
-- 桌面新建的工作区目录也不应该自动出现在"工作区"卡片里）。
--
-- `workspaces` 表本身（一个目录/连接的身份、路径等）继续保持模块无关；"哪个
-- 模块的首页/选择页会展示它"完全由这张关联表决定，一个工作区可以同时被多个
-- 模块关联（比如同一个项目目录既用来写代码也用来调 HTTP 接口）。
CREATE TABLE workspace_module_links (
  workspace_id TEXT NOT NULL,
  module TEXT NOT NULL,
  added_at TEXT NOT NULL,
  PRIMARY KEY (workspace_id, module),
  FOREIGN KEY(workspace_id) REFERENCES workspaces(id) ON DELETE CASCADE
);

-- 向后兼容：在这张表出现之前，"workspace"（工作区）模块的首页/选择页一直是
-- "打开过的所有工作区都算数"，为了不让存量用户升级后"工作区"卡片突然空掉，
-- 把已有的每一条 workspaces 记录都补一条 module='workspace' 的关联。'http' 是
-- 全新模块，不做任何回填——这正是本次需求要的效果："HTTP 桌面默认不显示任何
-- 已有目录，必须显式添加"。
INSERT INTO workspace_module_links (workspace_id, module, added_at)
SELECT id, 'workspace', COALESCE(last_opened_at, created_at) FROM workspaces;
