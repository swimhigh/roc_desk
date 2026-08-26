-- 远程工具模式（会话管理器）：connections 增加协议字段，SSH/RDP 共用同一张表和
-- 同一套分组（connection_groups 早在 0002 就建好了，只是分组的 CRUD 命令一直没实现，
-- 见 commands/connection_group.rs）。options 是协议相关的少量额外字段（目前只有 RDP
-- 用：domain/width/height/color_depth），JSON 存，不为这几个字段单独开列。
ALTER TABLE connections ADD COLUMN protocol TEXT NOT NULL DEFAULT 'ssh';
ALTER TABLE connections ADD COLUMN options TEXT;
