-- 2026-09：`coding/session.rs` 的 `limit_context` 原来对所有 Provider 一律用同一个
-- 写死的保守估算值（60_000 token），大窗口 Provider（比如支持 128K/200K+ 上下文的
-- 模型）被按小窗口的阈值频繁触发压缩，同一轮对话里反复丢弃刚探索过的文件内容/
-- 搜索结果，逼着模型对同一个任务重复探索（用户真实反馈：一次任务打了 87 万
-- token，后续追问同一个任务时又把所有文件重新翻了一遍）。
--
-- 加一列让用户按自己接的 Provider 真实上下文窗口手动填一个估算值；NULL 表示不填，
-- 继续退回原来的保守默认值，不影响任何存量数据/行为。
ALTER TABLE ai_providers ADD COLUMN context_window_tokens INTEGER;
