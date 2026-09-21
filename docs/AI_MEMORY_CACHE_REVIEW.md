# AI 工具记忆与搜索缓存评审

> 评审范围：roc_desk 的 AI 对话、Coding Agent、工具调用上下文、项目记忆、会话历史和前端状态缓存。
>
> 评审日期：2026-09-18
>
> 结论：建议新增“工作区级证据缓存（Evidence Cache）”，把搜索结果和文件快照从 LLM 对话消息中分离出来；同时保留结构化会话摘要和可恢复的工具轨迹。这样可以减少重复搜索，也避免上下文压缩后丢失已定位文件。

## 1. 现状与问题定位

### 1.1 Coding Agent 的上下文链路

主要代码路径如下：

- `src-tauri/src/coding/session.rs`：`CodingSession` 持有 `messages: Vec<Value>`，每次 `read_file`、`search_files`、`list_directory`、`web_search` 的结果都作为 `tool` 消息追加。
- `src-tauri/src/agent_llm.rs`：单条工具结果统一经过 `MAX_TOOL_RESULT_CHARS = 20_000` 截断；每轮请求都把 `self.messages` 重新序列化发送给模型。
- `CodingSession::limit_context`：接近上下文预算时，删除较早的 assistant/tool 交换，并调用模型生成摘要；摘要上限为 16,000 字符。
- `messages_snapshot`/`restore_messages`、`coding_history.messages_json`：已经可以持久化和恢复真实 LLM 消息，但保存依赖前端事件和约 5 秒节流的 checkpoint。
- `fetch_project_memory`：会话开始时读取 `AGENTS.md`、`CLAUDE.md` 等项目记忆文件，并把全文放进 system 消息。

### 1.2 普通 AI Chat 的搜索链路

`src-web/src/stores/aiChatStore.ts` 每次发送都会把当前前端消息历史整体传给 `ai_chat_send`。`src-tauri/src/ai/chat.rs` 在开启联网搜索时，从最近 3 条用户消息拼接查询、改写查询，然后每次请求都调用 Bing 搜索。搜索结果只插入本轮临时的 system 消息，没有查询缓存、来源快照或跨轮引用标识。

### 1.3 造成“忘记”的直接原因

1. **搜索结果是消息，不是证据对象**：结果只能靠原始 tool 消息留在上下文里；被 `limit_context` 删除后，摘要未必保留文件路径、行号、符号和 URL。
2. **缓存层级不对**：前端 `byWorkspace` 只缓存 UI 时间线和变更状态，后端搜索结果没有按工作区、目标主机、路径和查询指纹缓存。
3. **缓存没有失效语义**：即使把结果放进内存，也无法判断文件已经被编辑、Accept、外部进程修改，容易返回过期代码。
4. **每轮重复做全量发现**：Coding Agent 重新进入上下文后只能再次调用 `search_files`/`read_file`；普通 AI Chat 每次开启 web search 都重新请求 Bing。
5. **项目记忆注入过重且不可分层**：项目记忆全文放在 system 消息，挤占上下文；`AGENTS.md`/`CLAUDE.md` 的“规则”与本轮搜索证据没有区分优先级。
6. **持久化仍有窗口**：前端 5 秒 checkpoint 能降低 I/O，但进程在两次 checkpoint 之间退出时，最新搜索、工具结果和纯文本回答可能丢失。

### 1.4 现有优点

- 已有上下文预算、摘要压缩、工具结果截断和强制收敛机制，具备演进基础。
- `coding_history.messages_json` 已支持真实会话恢复，不需要推翻现有会话历史。
- `FileOps` 统一本地、SSH、Windows Agent，缓存可以放在工具层之上，不必复制三套搜索逻辑。
- `ChangeStore` 能识别未落盘的 pending 内容，适合成为缓存读取的最高优先级来源。

## 2. 参考 Codex / Claude Code 的可借鉴机制

这里借鉴的是公开产品表现和常见实现模式，不假设直接复制其内部代码。

- **Codex 类工具代理**：会话状态、线程事件和工具执行结果是持久化对象；上下文压缩是“保留可继续工作的摘要”，而不是简单截断；文件读取通常带路径、范围和当前状态信息。可参考 OpenAI Codex 文档：[developers.openai.com/codex](https://developers.openai.com/codex)。
- **Claude Code 类代理**：项目指令文件（如 `CLAUDE.md`）与会话记忆分开；工具结果通常按需读取，长任务通过摘要/压缩继续；重复的文件发现依赖工作区状态，而不是要求模型记住整段原文。可参考 Anthropic 文档：[docs.anthropic.com/en/docs/claude-code](https://docs.anthropic.com/en/docs/claude-code)。
- **共同原则**：持久化“事实和状态”，压缩“叙述”；工具结果采用可复用的引用；缓存必须有作用域、版本和失效条件；下一轮上下文只注入与当前意图相关的少量证据。

## 3. 推荐目标架构：四层记忆

```text
工作区/远程目标
   |
   +-- Evidence Cache：搜索命中、文件片段、目录列表、网页结果
   |       key = target + normalized query + options + content version
   |
   +-- Session State：用户目标、决策、todo、变更、工具轨迹、摘要
   |       持久化到 coding_history 和新增 session_memory
   |
   +-- Context Builder：按本轮意图选择证据，生成带引用的上下文
   |
   +-- LLM messages：只保留最近对话、结构化摘要和当前轮证据引用
```

### 3.1 Evidence Cache（核心改造）

缓存条目建议包含：

| 字段 | 说明 |
|---|---|
| `workspace_id` | 工作区隔离 |
| `target_key` | local / SSH connection / Agent + 主机标识 |
| `kind` | `content_search`、`file_snapshot`、`directory_listing`、`web_search`、`web_page` |
| `query_hash` | 规范化查询、路径、大小写/正则选项的 SHA-256 |
| `path_or_url` | 证据来源 |
| `content_hash` | 返回内容哈希，便于去重和引用 |
| `version_token` | 本地 mtime+size；远程可用协议返回的 mtime/size，拿不到时使用短 TTL |
| `payload` | 原始结果或压缩后的证据正文 |
| `snippet_index` | 文件路径、行号、符号、匹配片段等结构化索引 |
| `created_at` / `last_used_at` / `expires_at` | LRU 和 TTL |
| `status` | fresh / stale / invalidated |

建议新增 SQLite migration，例如 `0025_ai_evidence_cache.sql`：

```sql
CREATE TABLE ai_evidence_cache (
  id TEXT PRIMARY KEY,
  workspace_id TEXT NOT NULL,
  target_key TEXT NOT NULL,
  kind TEXT NOT NULL,
  query_hash TEXT NOT NULL,
  path_or_url TEXT NOT NULL,
  content_hash TEXT NOT NULL,
  version_token TEXT,
  payload_json TEXT NOT NULL,
  bytes INTEGER NOT NULL,
  created_at TEXT NOT NULL,
  last_used_at TEXT NOT NULL,
  expires_at TEXT,
  status TEXT NOT NULL DEFAULT 'fresh',
  UNIQUE(workspace_id, target_key, kind, query_hash, path_or_url, version_token)
);
CREATE INDEX idx_ai_evidence_lru ON ai_evidence_cache(workspace_id, last_used_at);
CREATE INDEX idx_ai_evidence_lookup ON ai_evidence_cache(workspace_id, target_key, kind, query_hash, status);
```

第一版不必引入向量数据库。先用精确哈希、路径索引和最近使用排序解决“同一轮/下一轮重复搜索”；语义检索属于后续优化。

### 3.2 查询规范化与命中策略

- `search_files`：规范化 pattern、根路径、大小写、正则/整词选项和目标主机；同一 `version_token` 下命中直接返回缓存。
- `read_file`：以 `path + requested range + version_token + pending_content_hash` 为 key；若 `ChangeStore` 有 pending 内容，优先返回 pending 快照，不能使用磁盘缓存覆盖它。
- `list_directory/glob`：按目录路径、过滤器和目录版本缓存，默认短 TTL。
- `web_search`：按改写后的查询、语言、区域和 provider 缓存，建议 TTL 10 分钟；每条结果保留 URL、标题、摘要和抓取时间。
- `webfetch`：按 URL 和响应 ETag/Last-Modified 缓存；没有验证头时 TTL 不超过 10 分钟。
- 缓存命中时工具结果应标记 `cache_hit=true`、`evidence_id` 和 `version_token`，让模型知道结果来自哪一版内容。

### 3.3 失效与一致性

按成本从低到高实施：

1. `ChangeStore::stage/accept/reject/undo/redo/revert_turn` 对相关路径主动失效。
2. 本地文件读取比较 mtime+size；mtime 不可靠时追加内容哈希。
3. SSH/Agent 返回 mtime、size，无法提供时使用 30~60 秒 TTL，并在再次读取时校验。
4. 工作区切换、连接切换、Provider 切换只改变作用域，不共享目标文件缓存。
5. 用户明确刷新、重新搜索或工具参数变化时绕过缓存。
6. 后台 LRU 清理：按 workspace 保留上限（例如 64 MB），超限删除最久未使用条目；不删除会话摘要和变更状态。

## 4. “记忆”如何进入下一轮上下文

### 4.1 不再把整个工具结果当长期记忆

每个工具调用完成后，同时写入：

- **事件轨迹**：保留在 `messages_json`，用于审计、历史回放和必要时重建。
- **证据记录**：写入 `ai_evidence_cache`，包含结构化路径、行号、版本和哈希。
- **会话记忆索引**：写入本轮产生的 `evidence_id`、结论、决策和待办，不复制全文。

### 4.2 Context Builder 的注入顺序

每轮发送前按以下顺序拼装：

1. 固定 system 规则和目标环境。
2. 项目记忆的“规则摘要”，全文只在首次或文件版本变化时注入。
3. 会话摘要：用户目标、已经确认的结论、修改过的文件、未完成 todo。
4. 最近 2~4 个用户/assistant 交换。
5. 与当前用户消息匹配的证据卡片：每个卡片包括 `evidence_id`、路径/URL、版本、行号、关键片段和一句结论。
6. 当前轮完整工具结果；只保留模型即将使用的结果。

摘要中必须采用可解析格式，至少保留：

```text
[事实]
- 已读取：src-tauri/src/coding/session.rs@version=...
- 关键位置：limit_context()、execute_tool()
- 结论：工具结果当前直接写入 messages，未建立跨轮证据缓存
[决策]
- 先实现精确哈希缓存，再评估向量检索
[待办]
- 增加缓存失效事件和命中率指标
```

这样上下文压缩可以删除旧正文，但不会删除“证据 ID + 来源 + 版本 + 结论”。模型需要原文时调用 `read_evidence(evidence_id, range)`，而不是盲目重新搜索。

### 4.3 项目记忆分层

- `AGENTS.md` / `CLAUDE.md`：规则层，按文件路径和版本缓存；只注入摘要，规则变化时重新读取。
- 会话摘要：任务层，随 `coding_history` 持久化。
- Evidence Cache：事实层，保存可验证来源和版本。
- UI 时间线：展示层，不作为模型唯一记忆来源。

## 5. 建议新增/调整的接口

### 后端

1. `EvidenceCacheRepo::{get, put, touch, invalidate_path, evict}`。
2. `ToolEvidence` 结构：`evidence_id、kind、source、version_token、content_hash、summary、range`。
3. 工具返回从纯字符串升级为内部结构：对模型仍可序列化为文本，同时向前端事件附带 `evidence_id/cache_hit`。
4. 新增只读工具 `read_evidence`：按证据 ID 和范围取回缓存内容；版本失效时返回 `stale` 并建议重新读取。
5. `ContextBuilder` 独立于 `CodingSession::limit_context`，负责摘要、证据选择和 token 预算。
6. `coding_history` 保留原始 messages；新增 `session_memory_json` 或独立表保存结构化事实/决策/evidence 引用。

### 前端

- Coding 时间线显示“缓存命中/版本”和“重新读取”动作；默认不把完整证据正文塞进持久化 UI timeline。
- 工作区切换时只恢复 session state，不把一个 workspace 的证据混入另一个 workspace。
- 搜索结果面板支持固定（pin）证据、手动刷新和查看来源版本。
- `aiChatStore` 增加请求级 `search_context_id` 或由后端自动复用缓存；不要依赖浏览器内存作为唯一缓存。

## 6. 分阶段落地计划

### Phase 0：观测（1~2 天）

- 记录每次工具的 `query_hash、result_bytes、cache_hit、version_token、context_tokens`。
- 统计重复查询率、同一文件重复读取率、上下文压缩次数、恢复后重新搜索次数。
- 不改变模型行为，先确认问题基线。

### Phase 1：精确缓存（3~5 天）

- 加 migration、repo 和内存 LRU；先覆盖 `search_files/read_file/list_directory/glob`。
- 接入 `ChangeStore` 主动失效和 mtime/size 校验。
- 工具事件增加 `evidence_id/cache_hit`。
- 缓存命中只返回相同版本的结果，宁可 miss 也不返回无法验证的新旧混合结果。

### Phase 2：记忆索引与上下文构建（5~8 天）

- 增加 `session_memory` 表或 `session_memory_json`。
- 把项目规则、任务摘要、证据引用分层；改造 `limit_context` 为“摘要 + 证据引用”压缩。
- 实现 `read_evidence`，在压缩后按需恢复原文。
- 保存策略从“前端事件触发”补强为后端每个完整工具交换后 checkpoint；继续保留前端节流作为展示层兜底。

### Phase 3：联网搜索缓存与跨会话复用（2~4 天）

- `ai_chat_send` 使用查询规范化后的 web cache。
- 搜索结果保留抓取时间、URL 和来源集合；过期结果明确标注，不静默当作最新事实。
- 允许同一 workspace 的新会话复用证据，但不跨 workspace/目标主机复用本地文件证据。

### Phase 4：可选语义索引

只有精确缓存命中率仍不足时，才增加 embedding/向量索引。向量索引只做“候选证据召回”，最终仍需用路径版本和内容哈希校验，不能替代一致性检查。

## 7. 风险与约束

- **陈旧代码风险**：任何没有 version token 的文件缓存只能短 TTL；缓存命中必须对模型可见。
- **敏感信息风险**：缓存沿用现有工作区权限和本地数据库保护；联网搜索结果与本地文件证据分开存储，避免把本地内容发送给云 Provider。
- **体积风险**：不在 `messages_json` 中重复保存完整证据；缓存设置 workspace 配额和 LRU。
- **远程一致性风险**：SSH/Agent 若不能提供可靠 mtime，默认视为弱一致性，优先短 TTL 和显式刷新。
- **兼容性风险**：先在现有自研 HTTP/工具循环中落地，不把 Codex 引擎迁移作为缓存前置条件；这样不影响 SSH、Windows Agent、Diff/Accept/Undo 和 MCP。

## 8. 验收指标

上线前后至少比较以下指标：

| 指标 | 目标 |
|---|---:|
| 同一会话重复 `search_files` 命中率 | >= 60% |
| 同一版本文件重复读取命中率 | >= 70% |
| 下一轮追问触发全量重新搜索比例 | 下降 >= 50% |
| 上下文压缩后能恢复原证据的比例 | >= 95% |
| 缓存命中返回过期文件比例 | 0（可验证版本下） |
| 缓存对单工作区磁盘占用 | 默认 <= 64 MB |
| 进程异常退出后丢失的完整工具交换 | <= 最近一次 checkpoint |

## 9. 优先级结论

P0：先做 `Evidence Cache + version_token + ChangeStore 失效 + evidence_id`，这是解决“每轮重新搜索”的最短路径。

P1：把 `limit_context` 改成结构化摘要和证据引用，解决“压缩后忘记搜索结果”。

P1：后端每次工具交换完成后保存 checkpoint，解决前端节流窗口内的会话丢失。

P2：给普通 AI Chat 的 web search 增加短 TTL 查询缓存和来源快照。

P3：在有命中率和延迟数据后，再决定是否需要 embedding；当前没有必要直接引入向量数据库或完整迁移 Codex 引擎。

## 10. SQLite FTS5 是否适合本方案

适合，建议使用 **SQLite FTS5 + 精确缓存表** 的组合，而不是用 FTS5 替代缓存。

### 10.1 两者分工

| 能力 | 精确缓存表 | FTS5 |
|---|---|---|
| 判断同一个查询是否已执行 | 适合 | 不适合 |
| 校验文件是否发生变化 | 适合，通过 `version_token` | 不适合 |
| 按路径、主机、工作区隔离 | 适合 | 适合，配合过滤列 |
| 按关键词查找历史证据 | 一般 | 很适合 |
| 召回“上轮搜索过的相关文件” | 需要额外逻辑 | 很适合 |
| 保证返回内容是最新版本 | 需要版本校验 | 仍需回查精确缓存/文件系统 |

推荐流程是：

```text
用户问题
  -> FTS5 召回相关 evidence_id
  -> 按 workspace/target/version_token 过滤
  -> 精确缓存命中则直接使用
  -> 版本过期则重新读取文件并更新 FTS 索引
```

### 10.2 推荐表结构

FTS5 表只存可搜索文本和必要的过滤字段，正文仍以 `ai_evidence_cache` 为准：

```sql
CREATE TABLE ai_evidence (
  id TEXT PRIMARY KEY,
  workspace_id TEXT NOT NULL,
  target_key TEXT NOT NULL,
  kind TEXT NOT NULL,
  path_or_url TEXT NOT NULL,
  version_token TEXT,
  content_hash TEXT NOT NULL,
  summary TEXT NOT NULL DEFAULT '',
  content TEXT NOT NULL,
  created_at TEXT NOT NULL,
  last_used_at TEXT NOT NULL
);

CREATE VIRTUAL TABLE ai_evidence_fts USING fts5(
  path_or_url,
  summary,
  content,
  content='ai_evidence',
  content_rowid='rowid',
  tokenize='unicode61 remove_diacritics 2'
);
```

如果项目需要中文按词检索，SQLite 默认 `unicode61` 对中文只能提供有限的字符级匹配能力。第一版可以采用以下策略：

- 路径、函数名、类名、英文标识符：直接使用 FTS5。
- 中文自然语言查询：先由模型或轻量规则抽取关键词，再用 FTS5 的 AND/OR 查询组合。
- 需要高质量中文分词时，再增加自定义 tokenizer 或在写入时生成 `search_terms`；不要一开始引入独立搜索服务。

### 10.3 外部内容表与触发器

`content='ai_evidence'` 属于 external-content FTS5，必须通过触发器保持同步：

```sql
CREATE TRIGGER ai_evidence_ai AFTER INSERT ON ai_evidence BEGIN
  INSERT INTO ai_evidence_fts(rowid, path_or_url, summary, content)
  VALUES (new.rowid, new.path_or_url, new.summary, new.content);
END;

CREATE TRIGGER ai_evidence_ad AFTER DELETE ON ai_evidence BEGIN
  INSERT INTO ai_evidence_fts(ai_evidence_fts, rowid, path_or_url, summary, content)
  VALUES ('delete', old.rowid, old.path_or_url, old.summary, old.content);
END;

CREATE TRIGGER ai_evidence_au AFTER UPDATE ON ai_evidence BEGIN
  INSERT INTO ai_evidence_fts(ai_evidence_fts, rowid, path_or_url, summary, content)
  VALUES ('delete', old.rowid, old.path_or_url, old.summary, old.content);
  INSERT INTO ai_evidence_fts(rowid, path_or_url, summary, content)
  VALUES (new.rowid, new.path_or_url, new.summary, new.content);
END;
```

查询时必须先过滤作用域，再排序：

```sql
SELECT e.id, e.path_or_url, e.version_token, e.summary,
       bm25(ai_evidence_fts) AS rank
FROM ai_evidence_fts f
JOIN ai_evidence e ON e.rowid = f.rowid
WHERE f MATCH ?1
  AND e.workspace_id = ?2
  AND e.target_key = ?3
ORDER BY rank
LIMIT 20;
```

### 10.4 使用边界

- FTS 命中只代表“可能相关”，不能代表内容仍然有效。
- `ChangeStore` 有 pending 内容时，应优先使用 pending 快照，并更新对应 evidence 的版本和索引。
- `search_files` 的精确查询结果可以进入 FTS；但模型下一轮需要原文时，仍通过 `evidence_id` 调用 `read_evidence`。
- 只索引摘要、路径和受控长度的正文，避免把大文件完整复制到 FTS；建议单条正文上限 64~128 KB。
- 删除或失效证据时同步删除 FTS 行；如果采用软失效，查询必须过滤 `status='fresh'` 或明确允许 stale。
- SQLite 构建时需确认启用 FTS5。项目当前已经使用 SQLite FTS 迁移（日志搜索），可以沿用同一数据库能力和迁移模式。

### 10.5 最终建议

采用“**精确缓存负责复用和一致性，FTS5 负责相关证据召回**”的双层设计：

1. Phase 1 先实现 `ai_evidence_cache` 的精确 key、版本校验和失效。
2. Phase 2 为证据增加 `ai_evidence` 与 `ai_evidence_fts`，支持上下文压缩后按当前问题召回历史证据。
3. 只有 FTS5 的关键词召回不足时，再考虑 embedding；不建议直接用向量数据库替换 SQLite。

## 11. 详细执行步骤

以下步骤按依赖顺序排列。每一步都可以独立提交和验证，建议不要一次性改写整个 `CodingSession`。

### Step 1：建立基线和开关

**目标**：先确认重复搜索和上下文丢失的真实比例，保证新缓存可以随时关闭。

改动：

- 增加配置项 `ai_evidence_cache_enabled`，默认在开发版本开启、生产版本先灰度开启。
- 在工具调用结束事件和后端日志中记录：`tool`、规范化查询、结果字节数、耗时、是否压缩、上下文估算 token 数。
- 为 `search_files/read_file/list_directory/glob/web_search/webfetch` 统一生成 `operation_id`。
- 增加指标：`cache_lookup_total`、`cache_hit_total`、`cache_stale_total`、`cache_write_total`、`context_compaction_total`、`resume_after_compaction_total`。

验收：连续执行同一搜索 3 次，日志能够区分查询相同、结果相同和缓存命中；关闭开关后行为与当前版本一致。

### Step 2：新增 SQLite 数据表和仓储层

**目标**：先把数据结构落地，不接入模型行为。

新增 migration（例如 `0025_ai_evidence_cache.sql`）：

- `ai_evidence_cache`：精确缓存正文、版本、作用域、TTL 和状态。
- `ai_evidence`：供 FTS5 使用的证据元数据和受控长度正文。
- `ai_evidence_fts`：external-content FTS5 表。
- 插入、更新、删除触发器；必要时增加 `rebuild` 脚本用于修复索引。

新增 Rust repo：

```text
src-tauri/src/db/repo/ai_evidence_repo.rs
  get_exact(key)
  upsert(entry)
  touch(id)
  search_fts(scope, query, limit)
  invalidate_path(scope, path)
  invalidate_target(target_key)
  evict_lru(workspace_id, max_bytes)
  prune_expired(now)
```

要求：

- 所有查询都必须带 `workspace_id` 和 `target_key`。
- `payload_json` 和 FTS 正文限制最大字节数，防止单个大文件拖垮 SQLite。
- `upsert` 使用事务，同时更新精确缓存表和 FTS 表。
- FTS 查询结果只返回 `evidence_id` 和摘要，不把大正文直接复制到上下文。

验收：数据库迁移成功；插入、更新、删除后 FTS 查询结果一致；重启应用后缓存仍可读取。

### Step 3：实现统一的缓存键和版本探测

**目标**：避免不同工具各自定义 key，造成命中不一致。

新增模块：

```text
src-tauri/src/ai/evidence.rs
  EvidenceScope
  EvidenceKey
  VersionToken
  EvidenceEntry
  normalize_query()
  make_query_hash()
  make_content_hash()
```

缓存键规则：

- `search_files`：`target + root + pattern + case_sensitive + whole_word + regex + version_token`。
- `read_file`：`target + path + range + pending_content_hash + version_token`。
- `list_directory/glob`：`target + path + filter + directory_version`。
- `web_search`：`provider + normalized_query + language + region`。
- `webfetch`：`url + etag/last_modified`，无响应头时使用 TTL。

版本探测顺序：

1. `ChangeStore` 的 pending 内容哈希。
2. `FileOps` 返回的 mtime、size。
3. 必要时计算内容哈希。
4. 远程端无法提供版本时使用短 TTL，并将 `version_strength=weak` 写入记录。

验收：同一文件未变化时 key 稳定；文件修改后 key 必须变化；工作区、远程连接和主机不同不能互相命中。

### Step 4：先接入只读文件工具

**目标**：以风险最低的方式验证缓存收益。

接入顺序：

1. `read_file`
2. `list_directory`
3. `glob`
4. `search_files`

每个工具调用统一执行：

```text
normalize args
  -> calculate scope/version/key
  -> exact cache lookup
  -> hit and version valid: return cached result
  -> miss/stale: call existing FileOps/search_stream
  -> cap result for model
  -> persist evidence + FTS index
  -> return result with evidence_id/cache_hit/version_token
```

注意：

- `read_file` 必须保留现有 `pending_content_for` 优先级。
- 缓存返回给模型的文本格式保持兼容，只在末尾增加机器可读元信息，例如：`[evidence_id=... cache_hit=true version=...]`。
- 结果过长时只把截断文本放入消息，完整受控正文留在缓存；模型需要更多内容时使用 `read_evidence`。
- 不缓存失败结果、权限拒绝结果和未确认的副作用工具结果。

验收：重复调用相同工具时第二次不访问文件系统；修改文件后自动 miss；pending change 场景读取到变更后的内容。

### Step 5：新增 `read_evidence` 工具

**目标**：上下文压缩后可以按 ID 恢复原证据，避免模型重新全量搜索。

工具参数：

```json
{
  "evidence_id": "...",
  "start_line": 1,
  "end_line": 120
}
```

返回内容必须包含：

- 当前状态：`fresh`、`stale`、`missing`。
- 原始来源路径或 URL。
- 当前版本与缓存版本。
- 请求范围和实际返回范围。
- 内容正文和必要的截断提示。

如果版本已经过期：

1. 不直接返回旧正文并伪装成最新内容。
2. 返回 `stale` 及原因。
3. 可由模型继续调用 `read_file` 或 `search_files` 获取新版本。

验收：压缩并删除旧 tool 消息后，模型使用 `read_evidence` 可以恢复同一文件的指定区间；过期文件被明确拒绝复用。

### Step 6：接入 `ChangeStore` 主动失效

**目标**：文件写入、接受、撤销后，缓存立即反映当前状态。

在以下操作成功后调用 `invalidate_path(workspace_id, target_key, path)`：

- `stage_change`
- `accept_change`
- `reject_change`
- `undo_change`
- `redo_change`
- `revert_turn`
- 编辑器外部保存同步

实现建议：

- `stage_change` 不删除旧磁盘快照，但将 pending 内容作为新证据版本。
- `accept/reject/undo/redo` 完成后删除该路径旧版本的 `fresh` 状态；保留历史记录为 `stale` 供审计。
- 目录变化时同时失效父目录的 `list_directory/glob` 缓存。
- 失效操作必须幂等，缓存不可用时不能阻断文件修改主流程。

验收：对文件执行 Accept/Undo 后，下一次 `read_file` 不会命中旧内容；目录新增/删除文件后，glob 不返回旧列表。

### Step 7：把上下文压缩改成“摘要 + 证据引用”

**目标**：解决“搜索过但下一轮忘了”的核心问题。

在 `CodingSession::limit_context` 中增加 `ContextBuilder`：

1. 保留 system 规则和项目记忆规则摘要。
2. 保留最近 2~4 个用户/assistant 交换。
3. 旧工具交换压缩为结构化事实：路径、符号、行号、结论、`evidence_id`、版本。
4. 删除旧正文时不删除证据缓存。
5. 当前用户消息到达后，先用 FTS5 召回相关证据，再按 token 预算注入 3~10 个证据卡片。
6. 当前轮新工具结果优先级高于历史召回结果。

证据卡片格式：

```text
[EVIDENCE id=ev_123 kind=file_snapshot status=fresh]
source: src-tauri/src/coding/session.rs
version: mtime=... size=...
range: lines 350-470
summary: limit_context 会删除旧 assistant/tool 交换，并写入摘要
retrieve: read_evidence("ev_123", 350, 470)
```

验收：上下文压缩后，下一轮追问同一文件时能通过证据 ID 定位；上下文 token 下降，但关键路径和结论保留。

### Step 8：项目记忆文件改为版本化分层加载

**目标**：避免 `AGENTS.md`/`CLAUDE.md` 全文长期占用上下文。

改动：

- 为每个项目记忆文件保存 `path + version_token + content_hash`。
- 首次会话读取全文并生成“规则摘要”。
- 后续会话只注入规则摘要和文件路径；版本变化时重新读取。
- 规则按目录作用域匹配：根目录规则、子目录规则、当前文件规则分开记录。
- `project_memory_loaded` 继续用于前端展示，但增加 `memory_version` 和 `refreshed_at`。

验收：未修改记忆文件时，新会话不重复读取全文；修改后下一次会话能检测到并刷新规则。

### Step 9：普通 AI Chat 接入联网搜索缓存

**目标**：解决 `ai_chat_send` 每轮重复 Bing 搜索的问题。

改动位置：`src-tauri/src/ai/chat.rs`。

流程：

1. 保留现有查询改写逻辑。
2. 对改写后的 query 做规范化和哈希。
3. 先查 `web_search` 精确缓存。
4. 命中且未过期时复用结果，并在 system 消息中注明抓取时间。
5. 未命中时请求 Bing，写入 URL、标题、摘要、抓取时间和 TTL。
6. 对“最新、今天、当前、实时”等查询缩短 TTL 或强制刷新。
7. 搜索结果过期时可以作为背景候选，但必须标注 stale，不能直接当成最新事实。

建议默认 TTL：普通搜索 10 分钟；明确实时查询 0~2 分钟；网页抓取按 ETag/Last-Modified 验证。

验收：同一会话相同查询不重复访问 Bing；过期后能重新搜索；回答仍保留 URL 引用。

### Step 10：后端 checkpoint 和历史恢复增强

**目标**：缩小进程异常退出造成的记忆丢失窗口。

改动：

- 工具交换完成后由后端异步保存 `messages_snapshot`、结构化 session memory 和 evidence 引用。
- 前端 5 秒节流保存继续保留，作为 UI 状态兜底，不再承担唯一持久化责任。
- `coding_history` 增加 `session_memory_json` 或新表 `coding_session_memory`。
- 恢复会话时加载：消息快照、任务摘要、todo、变更状态、证据引用；不把所有 evidence 正文重新塞进消息。
- 保存失败只记录可见状态，不阻断当前 AI 任务。

验收：在工具调用中强制结束进程，重启后至少能恢复到最近一个完整工具交换，并能找到对应 evidence。

### Step 11：前端可视化和手动控制

**目标**：让用户知道模型引用了哪一版内容，并提供可控刷新。

改动：

- Coding Agent 时间线的工具卡显示：`命中缓存`、`重新读取`、版本时间、来源路径。
- 增加“固定证据”操作，固定条目不参与普通 LRU 清理，但仍遵守版本失效。
- 增加“刷新此证据”和“清理当前工作区缓存”。
- 搜索/文件面板展示命中来源和抓取时间；web 结果显示 TTL 状态。
- 前端只保存展示所需的 `evidence_id` 和摘要，不保存完整正文副本。

验收：用户可以查看证据来源、强制刷新和清理缓存；工作区切换后不会看到其他工作区的证据。

### Step 12：测试、灰度和回滚

#### 自动化测试

- key 规范化：参数顺序、大小写、路径分隔符、远程目标隔离。
- 版本失效：mtime、size、hash、pending 内容和目录变化。
- FTS5：插入、更新、删除触发器，中文/英文/路径检索，workspace/target 过滤。
- 工具行为：hit、miss、stale、权限错误和超长结果。
- 上下文压缩：摘要必须保留 evidence_id；`read_evidence` 可恢复指定行区间。
- 历史恢复：异常退出后恢复最新 checkpoint。
- web cache：TTL、实时查询强制刷新、URL 引用保留。

#### 灰度策略

1. 仅记录命中，不改变返回内容。
2. 只对 `read_file` 开启实际复用。
3. 扩展到 `search_files/list_directory/glob`。
4. 最后开启 FTS 召回和 web search 缓存。

每阶段观察 1~2 个版本周期：命中率、平均工具耗时、上下文 token、过期命中数和用户手动刷新率。

#### 回滚策略

- 关闭 `ai_evidence_cache_enabled` 即可回到现有工具路径。
- FTS 查询失败自动退化为精确 key 或原始搜索，不阻断会话。
- 缓存数据库损坏时重建 FTS 表，必要时删除缓存表；不删除 `coding_history` 和文件变更记录。
- Context Builder 失败时沿用现有 `limit_context` 摘要逻辑。

## 12. 推荐任务拆分

建议拆成以下提交或任务单：

1. `observability/ai-cache-metrics`：指标、operation_id、配置开关。
2. `db/ai-evidence-cache`：migration、repo、LRU/TTL 清理。
3. `ai/evidence-key-version`：规范化、版本 token、哈希。
4. `coding/read-file-cache`：只读文件工具接入。
5. `coding/search-cache`：search_files、glob、目录缓存。
6. `coding/read-evidence`：证据恢复工具。
7. `coding/change-invalidation`：ChangeStore 失效事件。
8. `coding/context-builder`：摘要、FTS 召回、token 预算。
9. `ai/web-search-cache`：普通 AI Chat 搜索缓存。
10. `coding/backend-checkpoint`：后端持久化和恢复。
11. `ui/evidence-status`：前端展示、刷新和清理。
12. `test/ai-memory-cache`：集成测试、灰度指标和故障回退。

每个任务完成后都应能独立编译和运行；涉及 schema 的任务先完成迁移回滚验证，再接入业务代码。

## 13. 当前执行状态（2026-09-18）

本轮已完成第一批可运行实现：

- 新增 `0025_ai_evidence_cache.sql`：精确缓存表、证据表、FTS5 external-content 表和同步触发器。
- 新增 `AiEvidenceRepo`：精确读取、写入、路径失效、按 ID 读取和 FTS 检索接口。
- `AppState`/CodingSession 已持有 Evidence Repo。
- `read_file` 已按文件大小和版本信息尝试复用快照；读取成功后写入证据并返回 `evidence_id`。
- 新增 `read_evidence` 工具，可以在上下文压缩后按证据 ID 和行号恢复内容。
- `search_files` 已接入按工作区、目标、查询和路径的精确缓存；写入/生成变更时会使相关路径缓存失效。
- 已通过 `cargo check --manifest-path src-tauri/Cargo.toml` 和后端单元测试编译验证。

本轮继续完成：Context Builder 的 FTS 自动召回、普通 AI Chat 的进程内 10 分钟联网搜索 TTL 缓存，以及 `read_evidence` 的上下文恢复。后端工具交换 checkpoint、前端缓存状态展示和远程 mtime/ETag 的强校验仍属于下一步；当前实现优先保证缓存命中不会绕过 `ChangeStore` 的 pending 内容，缓存异常会退回原始工具路径。
