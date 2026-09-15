-- 2026-09：codex-core 集成删除后，老引擎自己补上 OpenAI Responses API 支持（原来
-- 引入 codex-core 就是为了覆盖 chat/completions 说不通话的官方 OpenAI/Azure/Bedrock
-- 端点）。每个 provider 现在要标注自己用哪种协议——`chat_completions`（默认，兼容
-- 现有全部数据）或 `responses`，由用户在 Provider 设置里手动选择，不做自动探测。
ALTER TABLE ai_providers ADD COLUMN wire_api TEXT NOT NULL DEFAULT 'chat_completions';
