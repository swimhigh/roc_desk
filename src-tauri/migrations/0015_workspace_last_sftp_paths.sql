-- 工作区里最后一次 SFTP/Agent 文件传输浏览到的两边目录（用户 2026-09-01 需求：
-- "下次启动工作区中的SFTP或文件传输时，直接定位到最后记住的目录"）。NULL 表示
-- 这个工作区还没打开过 SFTP/文件传输，前端据此退回默认值（远程侧退回工作区
-- 根目录，本地侧退回用户主目录）。
ALTER TABLE workspaces ADD COLUMN last_sftp_local_path TEXT;
ALTER TABLE workspaces ADD COLUMN last_sftp_remote_path TEXT;
