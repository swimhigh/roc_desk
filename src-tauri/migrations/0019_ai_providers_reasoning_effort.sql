-- 2026-09：对齐 Codex `model_reasoning_effort`（config.toml 里的
-- `model_reasoning_effort = "medium"`）——gpt-5/o 系列这类推理模型可以从客户端
-- 调节内部推理力度，roc_desk 之前完全没有传这个参数，效果等同于永远交给服务端
-- 自己的默认值。NULL/空字符串表示不传这个参数（维持现状，兼容现有全部数据和
-- 不支持这个参数的 Provider），非空时按 `wire_api` 决定塞进请求体的哪个位置
-- （见 `coding/session.rs`）。
ALTER TABLE ai_providers ADD COLUMN reasoning_effort TEXT;
