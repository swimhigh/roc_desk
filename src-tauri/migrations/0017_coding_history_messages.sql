-- 用户 2026-09-09 反馈："历史会话只能只读不能继续修改"——根因是发给 AI 的真实
-- 对话上下文（CodingSession 内部的 messages）只存在内存里，从未持久化，重开/
-- 切走工作区就丢了，"打开历史会话"只能回放一份文件改动记录，没法真的接着聊。
-- 加这一列把它落库，配合 coding_history_resume 命令，"打开历史会话"能在原有
-- 上下文基础上真正继续对话。
ALTER TABLE coding_history ADD COLUMN messages_json TEXT NOT NULL DEFAULT '[]';
