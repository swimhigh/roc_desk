-- 2026-10 用户明确要求：AI 工具的会话历史内容（timeline/changes/messages）
-- 只应该存在工作区自己的 .rock_desk 子目录里，不该在 exe 旁边的全局
-- roc_desk.db 里再留一份镜像——此前为了让"打开历史列表"脱离网络能快速展示，
-- 把这三份可能长到几十上百 MB 的内容整份复制进这张表，实测把全局库撑到
-- 598MB（只有十几个工作区）。内容的唯一权威来源收敛回工作区目录文件后，
-- 这张表里留的这份就是纯粹的历史包袱，清空后用 VACUUM 真正把磁盘空间要
-- 回来（UPDATE/DELETE 本身不会让 SQLite 文件变小）。
UPDATE coding_history SET timeline_json = '', changes_json = '', messages_json = '[]';
VACUUM;
