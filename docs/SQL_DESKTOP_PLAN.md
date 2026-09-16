# roc_desk SQL 桌面模块落地方案

> 状态：方案评审稿（可作为实现任务拆分的基线）  
> 目标：在现有 roc_desk 桌面首页增加 SQL 桌面入口和数据源管理，并提供类似 DBeaver 的多数据库 SQL 工作区。

## 1. 目标与边界

### 1.1 用户目标

- 在桌面首页看到“SQL 桌面”内容卡片、最近使用的数据源和连接状态。
- 可以新增、编辑、复制、测试、删除数据源；密码等凭据不落明文数据库。
- 点击数据源后进入独立 SQL 工作区：左侧 Schema/Table 树，中间 SQL 编辑器与标签页，下方结果/消息/执行计划，右侧 AI 工具栏。
- 首期支持 MySQL、PostgreSQL、openGauss、Oracle、SQL Server，并允许后续增加 SQLite、达梦、人大金仓等适配器。
- 查询默认只读、可取消、有超时和行数上限；写操作必须明确确认并保留审计记录。

### 1.2 非目标（首期不做）

- 不复制完整 DBeaver 的建模、ETL、数据同步和可视化报表能力。
- 不在客户端保存数据库业务数据，只保存连接配置、工作区布局、查询历史和脱敏元数据。
- 不让 AI 默认直接执行 SQL；AI 只能生成、解释、改写和在用户确认后执行。

## 2. 与现有工程的集成原则

项目已经采用 `src-web`（React）+ `src-tauri`（Rust）+ SQLite 元数据存储的分层结构。本模块遵循现有约定：

1. 前端组件不直接调用 `invoke`，统一通过 `src-web/src/services/sql/`。
2. Tauri command 只做参数校验、权限检查和调用 domain；业务逻辑放在 `src-tauri/src/sql/`。
3. 所有跨端 DTO 通过共享类型/生成类型维护，错误使用统一 `AppError`。
4. 数据源密码、SSH 私钥口令等机密放系统 keyring；SQLite 只存 `credential_ref`。
5. 每个 SQL 工作区拥有独立的连接会话和取消令牌，窗口/标签页关闭时释放资源；但同一数据源应该
   共享一个连接池（而不是每打开一个标签页就新建一个 pool），池大小按数据源配置且要给个保守的
   默认上限（参考 rainfrog 对 Postgres 连接池默认 `max_connections(3)`），避免用户开多个标签页
   时把数据库连接数打满。

## 3. 产品形态与信息架构

### 3.1 首页新增 SQL 桌面区域

首页（Launcher/HomeShell）新增一级卡片“SQL 桌面”：

- **新建数据源**：打开数据源向导。
- **数据源列表**：按最近使用、分组、类型筛选；显示连接状态、环境标签（开发/测试/生产）。
- **最近查询**：显示名称、数据源、执行时间和成功/失败状态，点击恢复到工作区。
- **打开 SQL 桌面**：无指定数据源时进入数据源选择页。

数据源卡片操作：连接/断开、测试连接、编辑、复制、删除、打开工作区。生产环境数据源显示醒目标记，删除需要二次确认。

### 3.2 数据源向导

向导步骤：

1. 选择数据库类型和驱动状态；
2. 填写名称、主机、端口、数据库/服务名、Schema、用户名、认证方式、环境标签；
3. 高级选项：TLS、代理/SSH 隧道、连接池、默认超时、只读模式、`search_path`/`NLS` 等会话参数；
4. 保存凭据（系统 keyring）并测试连接；
5. 成功后可立即打开 SQL 工作区。

默认端口：MySQL 3306、PostgreSQL/openGauss 5432、Oracle 1521、SQL Server 1433。端口必须可编辑。

### 3.3 SQL 工作区布局

```text
┌─ 工作区工具栏：数据源 | 连接状态 | Schema | 运行 Ctrl+Enter | 停止 | 格式化 | 导出 ─┐
├──────────────┬──────────────────────────────┬──────────────────────┤
│ 对象浏览器    │ SQL 标签页/编辑器              │ AI 工具                │
│ ▾ Schemas     │ -- 可多标签、拆分编辑器         │ 生成 SQL               │
│   ▾ Tables    │                                  │ 解释/优化 SQL           │
│   ▾ Views     │                                  │ 根据表结构提问           │
│   ▾ Functions │                                  │ 修复错误                │
│ 搜索对象      │                                  │ 脱敏预览与确认           │
├──────────────┴──────────────────────────────┴──────────────────────┤
│ 输出区：结果集 | 消息 | 执行计划 | 历史 | 变量/参数                      │
└──────────────────────────────────────────────────────────────────────┘
```

- 左栏支持懒加载、搜索、刷新、展开对象详情；双击表可生成 `SELECT` 模板，右键可查看前 N 行/复制名称。
- 中栏采用 Monaco（沿用现有编辑器能力），支持 SQL 语法高亮、补全、括号匹配、格式化、参数占位符、多个标签页。
- 下栏结果支持**表格**和**纯文本**两种展示模式，工具栏一个切换按钮，按标签页记忆选择
  （对应第 5 节 `sql_workspace_tabs.result_view_mode`）：
  - 表格模式：虚拟滚动网格，默认最多展示 1,000 行；支持分页、列排序、复制单元格/整行、
    NULL 与二进制值有专门的展示态（参考 [4.2.1](#421-参考实现rainfrog) rainfrog
    `parse_value` 对不同列类型的处理方式，避免所有类型都粗暴转字符串）。
  - 纯文本模式：等宽对齐渲染（类似 `psql`/`mysql` CLI 的对齐输出），一次性可全选复制、
    方便贴到聊天/工单/AI 对话里；行数超过表格模式的展示上限时提示"仅预览前 N 行，导出获取全量"。
  两种模式渲染同一份已拉取的结果数据，纯前端切换，不需要重新查询，也不新增 IPC command。
  - 导出（CSV/JSON）与展示模式解耦，导出任务始终走第 6 节的流式通道，不受当前展示模式限制。
- 右栏 AI 工具与现有 AI provider 集成，显示当前数据源、Schema 上下文和脱敏策略；可折叠以适应小屏。

## 4. 技术架构

### 4.1 Rust domain 分层

建议新增目录：

```text
src-tauri/src/sql/
├── mod.rs
├── model.rs              # DataSourceProfile、QueryRequest、ResultPage 等 DTO
├── service.rs             # 数据源 CRUD、会话生命周期、查询编排
├── adapter.rs             # DatabaseAdapter trait
├── registry.rs            # 类型 -> adapter 工厂、能力声明
├── metadata.rs            # schema/table/column 元数据读取与缓存
├── executor.rs            # 超时、取消、行数限制、流式结果
├── policy.rs              # 只读/生产环境/危险 SQL 检查
├── history.rs             # 查询历史和收藏
└── adapters/
    ├── mysql.rs
    ├── postgres.rs
    ├── opengauss.rs
    ├── oracle.rs
    └── sqlserver.rs
```

`src-tauri/src/commands/sql.rs` 只暴露 command；前端新增：

```text
src-web/src/
├── components/SqlDesktop/
│   ├── SqlDesktopHome.tsx
│   ├── DataSourceWizard.tsx
│   ├── DataSourceList.tsx
│   ├── SqlWorkspace.tsx
│   ├── ObjectExplorer.tsx
│   ├── SqlEditorTabs.tsx
│   ├── ResultPanel/
│   │   ├── ResultPanel.tsx      # 表格/纯文本模式切换的容器
│   │   ├── ResultTableView.tsx  # 虚拟滚动网格
│   │   └── ResultTextView.tsx   # 等宽对齐纯文本渲染
│   └── SqlAiPanel.tsx
├── services/sql/{datasource,metadata,query,history}.ts
├── stores/sqlWorkspaceStore.ts
└── types/sql.ts
```

### 4.2 适配器接口

```rust
#[async_trait]
pub trait DatabaseAdapter: Send + Sync {
    fn kind(&self) -> DatabaseKind;
    fn capabilities(&self) -> DbCapabilities;
    async fn test_connection(&self, profile: &ResolvedProfile) -> Result<DbInfo>;
    async fn list_catalogs(&self, session: &DbSession) -> Result<Vec<Catalog>>;
    async fn list_objects(&self, session: &DbSession, scope: ObjectScope) -> Result<ObjectPage>;
    async fn describe_object(&self, session: &DbSession, object: ObjectRef) -> Result<ObjectDefinition>;
    async fn execute(&self, session: &DbSession, request: ExecuteRequest) -> Result<ExecuteResult>;
    /// 取消必须能真正终止服务端正在执行的语句（如 Postgres `pg_cancel_backend`、
    /// MySQL `KILL QUERY`），不能只中断本地 tokio 任务。
    async fn cancel(&self, session_id: Uuid, query_id: Uuid) -> Result<()>;
    /// 危险/写操作先在事务中执行，返回受影响行数供前端确认，确认后再 commit，否则 rollback；
    /// 不支持事务内回滚的语句（多数 DDL）走单纯确认，不进这条路径。
    async fn start_confirmable_write(&self, session: &DbSession, request: ExecuteRequest) -> Result<PendingWrite>;
    async fn commit_write(&self, session: &DbSession, pending_id: Uuid) -> Result<ExecuteResult>;
    async fn rollback_write(&self, session: &DbSession, pending_id: Uuid) -> Result<()>;
}
```

适配器必须负责方言差异（分页、标识符引用、元数据 SQL、执行计划、参数占位符），UI 和 service 层不得拼接数据库专用 SQL。

### 4.2.1 参考实现：rainfrog

`trait DatabaseAdapter` 的执行/取消/事务确认部分不建议从零设计，直接借鉴
[`achristmascarl/rainfrog`](https://github.com/achristmascarl/rainfrog)（MIT 协议，Rust + sqlx
实现的终端数据库客户端，支持 Postgres/MySQL/SQLite/Oracle/DuckDB，持续维护中）。它已经解决了本方案
第 3.3、6、7 节里目前只有原则性描述、缺少落地机制的三个问题：

1. **执行不阻塞、可轮询**：`start_query` 只 `tokio::spawn` 任务并立刻返回，UI 侧靠
   `get_query_results()` 轮询 `DbTaskResult::{Pending, Finished, NoTask}`。比同步 await 更适合
   Tauri command + 前端事件流的模型，`sql_execute` 可以直接对应 `start_query` +
   周期性 `get_query_results` 轮询后转发 `sql://query/{id}/chunk` 事件。
2. **取消是真取消，不是"扔掉 Future"**：`abort_query()` 除了 `handle.abort()` 中断 tokio 任务外，
   还会在 Postgres 上另开一条连接执行 `SELECT pg_cancel_backend(<pid>)` 去杀服务端正在跑的查询——
   仅 abort Rust 侧的 Future 并不会让数据库停止执行已经下发的 SQL，这是很多简易实现会漏掉的坑。
   `sql_cancel` 的适配器实现里必须照这个模式做：每种数据库都要有对应的服务端强制终止手段
   （MySQL `KILL QUERY <id>`、SQL Server 从另一连接 `KILL <spid>`、Oracle OCIBreak），
   仅取消 Rust 端 Future 是不够的。
3. **写操作的二次确认用"开事务 + 展示影响行数 + 用户确认后再 commit/rollback"**，而不是"预览 SQL
   弹窗后直接执行"：`start_tx` 开事务执行语句、暂停在 `DbTaskResult::ConfirmTx(rows_affected, ...)`，
   等前端调用确认后再 `commit_tx`／`rollback_tx`。这样"生产环境二次确认"展示的是数据库真实
   反馈的受影响行数，而不是靠 SQL 文本本身估算，风险更低。DDL 类语句（`DROP`/`TRUNCATE`/`ALTER`）
   因为在 MySQL 等库上不支持事务内回滚，走单纯确认弹窗（`ExecutionType::Confirm`），不进事务。
4. **危险 SQL 判定用 SQL AST 而非正则**：rainfrog 用 `sqlparser` crate 解析出 `Statement`，按语句
   类型分类为 `Normal`（直接执行）/`Confirm`（弹窗确认，如 DDL）/`Transaction`（先开事务再确认，如
   UPDATE/DELETE），连 CTE 包裹的写操作（`WITH ... UPDATE/DELETE`）也能正确识别到内层语句类型。
   本方案第 7 节"SQL 解析失败时采取拒绝策略，不以正则作为唯一防线"这句话可以直接换成"复用
   `sqlparser` 对语句分类，解析失败或分类未知一律降级为 `Confirm`"。

以上四点建议直接参考（必要时可以移植改写）rainfrog 的 `src/database/mod.rs`（trait 与语句分类逻辑）
和 `src/database/postgresql.rs`（连接池、任务轮询、`pg_cancel_backend` 取消、按列类型解析结果值的
`parse_value` 函数，这段对第 6 节"结果集"如何把任意数据库类型序列化成前端可用的字符串/JSON 很有参考
价值）。MIT 协议下移植代码时，在对应 Rust 文件顶部保留一行来源与版权注释即可，无需额外审批；
但 rainfrog 目前没有 SQL Server 适配器，`tiberius` 部分（分页、`KILL <spid>` 取消、元数据 SQL）
仍需自行实现，社区里没有现成的高质量 Rust 参考可抄。

第二个可以参考、但**只借鉴设计不借代码**的项目是 Beekeeper Studio（`lib/db/clients/*`，覆盖
MySQL/Postgres/SQL Server/Oracle/Redshift 等更多方言的元数据归一化模式）。它是 AGPL-3.0
协议，roc_desk 是 MIT + Commons Clause，直接搬运 AGPL 代码会把所在文件的授权状态搞复杂，
不建议照抄实现，仅可用来对照“多方言元数据归一化该长什么样”。

### 4.3 驱动落地建议

| 类型 | 首选 Rust 驱动策略 | 备注 |
|---|---|---|
| MySQL | `sqlx` MySQL | 支持 TLS、参数化、流式读取 |
| PostgreSQL | `sqlx` PostgreSQL | 可复用大部分元数据实现 |
| openGauss | PostgreSQL wire 适配器（优先 `tokio-postgres`/兼容驱动） | 增加版本探测和兼容性开关，禁止假设所有 PG 扩展存在 |
| SQL Server | `tiberius` | TDS 协议；单独实现分页、参数和执行计划 |
| Oracle | `oracle`/ODPI-C 可选特性 | 安装包需明确 OCI/Instant Client 依赖；无驱动时显示可操作的安装提示 |

驱动以 Cargo feature 或可选模块编译，避免未安装 Oracle 客户端导致其他数据库不可用。所有驱动版本、TLS 支持和许可证在发布构建清单中锁定。

### 4.4 本地目录缓存：让 AI 像改代码一样改 SQL

SQL 工作区不像现有"AI 编程助手"那样天然绑定一个磁盘项目目录——一个数据源本质上只是连接信息，
没有用户选定的本地文件夹。要让 AI 复用已经验证过的文件改动状态机（`src-tauri/src/coding/session.rs`
+ `coding/changes.rs` 的 `ChangeStore`：`stage`/`accept`/`reject`/`undo`/`redo`/`revert_turn`，
diff 用 `similar`，前端 `FileChangeCard` + `changesById` + `coding:file-change` 事件那一整套），
就必须先给每个 SQL 工作区造出一个真实存在的本地目录，把每个查询标签页落成一个真实的 `.sql` 文件。

**目录结构**（沿用现有 `.rock_desk/` 惯例和 `WorkspaceManager` 的 `cache_root` 兜底模式）：

```text
<cache_root>/sql/<data_source_id>/
├── workspace.json            # 元数据：data_source_id、显示名、创建/更新时间
├── queries/
│   ├── <tab_id>.sql          # 每个标签页对应一个真实文件；AI 的 read_file/write_file/edit_file
│   │                          # 工具直接操作这里，走已有的 ChangeStore
│   └── ...
├── schema_cache/              # 只读 DDL 快照，供 AI 当上下文用，靠 list_directory/glob/read_file
│   ├── manifest.json          # 记录抓取时间、来源连接、涉及哪些 schema/table
│   └── <schema>__<table>.ddl.sql
├── sessions/
│   └── <history_id>.json      # AI 对话历史快照，格式对齐现有 WorkspaceHistorySnapshot
└── exports/                   # 导出结果的临时落地目录，按时间定期清理
```

`<cache_root>` 直接复用 `WorkspaceManager` 已有的 cache_root（`<exe_dir>/.rock_desk/workspaces`
的兄弟目录），不新增一套缓存根目录管理逻辑。目录以 `data_source_id` 为 key 而不是"工作区窗口"，
这样同一数据源在多个窗口打开时能共享同一份文件和同一个活跃 AI 会话（对齐现有
`state.coding_sessions: HashMap<workspace_id, ...>` 的单活跃会话模型），避免出现两份不同步的
`.sql` 文件。

**标签页 = 文件，路径在创建时就定好**：新建查询标签页时立刻分配 `tab_id`（与
`sql_workspace_tabs.id` 一致）并建好 `queries/<tab_id>.sql`（哪怕内容是空的），而不是等到"保存"
才落盘。这是因为 `ChangeStore::stage()` 按路径字符串做身份匹配，从未有过确定路径的"虚拟标签页"
没法被正常 diff/accept——只要路径提前存在（哪怕是空文件），AI 第一次写入就能走"读盘失败退化为
整份新增"的既有分支生成正常 diff。相应地，第 5 节 `sql_workspace_tabs` 表的 `sql_text` 列改成
`file_path`（指向 `queries/<tab_id>.sql` 的相对路径），标签页内容的唯一真相是磁盘文件，SQLite
只存元数据（标题、光标位置、排序、更新时间）——这样 AI 编辑、Monaco 编辑器、磁盘文件三者才能对齐
同一份状态，不会出现"SQLite 里的内容"和"AI 刚改完的文件内容"谁准的歧义。

**AI 工具复用与新增**：

- `read_file`/`write_file`/`edit_file`/`list_directory`/`glob`/`search_files` 原样复用现有
  `coding/tools.rs` 定义，作用域限定在 `<cache_root>/sql/<data_source_id>/` 内。生成 SQL/优化
  SQL/修复错误这几个 AI 功能，本质上就是对 `queries/<tab_id>.sql` 做一次 `edit_file`，走
  `stage_change` 生成 diff，用户在 `FileChangeCard` 上点"应用"才真正落盘并让 Monaco buffer 走
  `syncExternalWrite` 刷新——不用另起一套"AI 直接改编辑器内容"的通道，直接搬现成 UI。"整轮撤销"
  （`turn_id` + `revert_turn`）语义也直接保留：AI 一次改了好几个标签页，用户可以按轮次一次性撤销。
- 新增一组 SQL 专属工具（`sql_run_query`/`sql_explain`/`sql_list_objects`/`sql_describe_object`/
  `sql_preview_rows`），不走文件系统，而是调用第 6 节的 IPC command。执行层面必须复用第 7 节的
  `sqlparser` AST 分类和 `start_confirmable_write` 流程——AI 发起的写操作和人手动在编辑器里执行
  遵守同一套确认闸门，不能因为是"AI 调用"就绕开确认。
- **必须补的安全边界（现有编程助手目前缺失，不能照抄现状）**：现有 `execute_tool`
  （`session.rs`）里的 `read_file`/`write_file`/`edit_file` 目前没有路径越界校验，完全靠系统
  提示词自觉，`permission.rs` 的权限引擎也明确只管 `run_command`/`webfetch`/`mcp`，不管文件读写。
  SQL 工作区的文件工具必须比照 `commands/fs.rs::guard_local_path` 的做法（`canonicalize()` 后
  校验 `starts_with(workspace_root)`）显式拒绝任何落在 `<cache_root>/sql/<data_source_id>/`
  之外的路径——这里涉及连接凭据引用、查询历史等相对敏感的数据源信息，不适合沿用现状"无边界、
  靠 UI 确认兜底"的做法。

**Schema 上下文走文件而不是专门的 AI 工具**：`schema_cache/` 在打开工作区、切换 Schema 或对象树
刷新时增量写入涉及到的表/视图 DDL（内容来自 `sql_describe_object`），AI 靠通用的
`list_directory`/`glob`/`read_file` 就能像浏览代码仓库一样浏览表结构，不需要为"根据表结构提问"
单独定制协议。`schema_cache/` 只允许存 DDL/结构信息，不允许缓存业务数据样本；如果用户显式授权
发送数据样本（第 7 节的脱敏流程），样本只能进对话上下文，不落盘到这个目录，避免在磁盘上留下明文
业务数据。`exports/` 目录同理需要定期按年龄清理，避免导出的 CSV/JSON 无限堆积。

## 5. 数据模型与持久化

在现有 SQLite migration 后追加 `0020_sql_desktop.sql`（当前仓库最新迁移是 `0019_ai_providers_reasoning_effort.sql`；实现时以届时最新编号为准，不要固定写死）：

```sql
CREATE TABLE sql_data_sources (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  db_kind TEXT NOT NULL,
  host TEXT NOT NULL,
  port INTEGER,
  database_name TEXT,
  service_name TEXT,
  default_schema TEXT,
  username TEXT,
  credential_ref TEXT,
  options_json TEXT NOT NULL DEFAULT '{}',
  environment TEXT NOT NULL DEFAULT 'dev',
  group_name TEXT,
  readonly INTEGER NOT NULL DEFAULT 1,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  last_used_at TEXT
);

CREATE TABLE sql_query_history (
  id TEXT PRIMARY KEY,
  data_source_id TEXT NOT NULL,
  title TEXT,
  sql_text TEXT NOT NULL,
  normalized_sql TEXT,
  status TEXT NOT NULL,
  duration_ms INTEGER,
  row_count INTEGER,
  error_code TEXT,
  created_at TEXT NOT NULL,
  FOREIGN KEY(data_source_id) REFERENCES sql_data_sources(id) ON DELETE CASCADE
);

CREATE TABLE sql_workspace_tabs (
  id TEXT PRIMARY KEY,
  data_source_id TEXT NOT NULL,
  title TEXT NOT NULL,
  file_path TEXT NOT NULL,       -- 相对路径，指向 <cache_root>/sql/<data_source_id>/queries/<id>.sql
  result_view_mode TEXT NOT NULL DEFAULT 'table', -- 'table' | 'text'，见 3.3 结果区
  cursor_json TEXT,
  sort_order INTEGER NOT NULL DEFAULT 0,
  updated_at TEXT NOT NULL,
  FOREIGN KEY(data_source_id) REFERENCES sql_data_sources(id) ON DELETE CASCADE
);
```

`sql_workspace_tabs` 不再存 SQL 正文：标签页内容的唯一真相是 `file_path` 指向的磁盘文件（见
[4.4](#44-本地目录缓存让-ai-像改代码一样改-sql)），SQLite 只存元数据。`sql_query_history.sql_text`
不受影响，继续原样保存——它是"某次实际执行过什么"的审计记录，跟标签页当前内容是两回事，不应该
因为标签页文件后续被覆盖或删除而丢失历史。

敏感字段只保存引用：`credential_ref` 指向 keyring 条目；日志、历史和 AI 上下文对密码、Token、连接串中的 secret 做统一脱敏。

## 6. IPC/API 设计

首批 command：

| Command | 作用 |
|---|---|
| `sql_list_data_sources` | 列出数据源（不返回密码） |
| `sql_save_data_source` | 新增/编辑配置并写入 keyring |
| `sql_delete_data_source` | 删除配置及凭据引用 |
| `sql_test_connection` | 测试连接并返回版本/延迟 |
| `sql_open_session` / `sql_close_session` | 创建/释放工作区会话 |
| `sql_list_objects` | 懒加载 Schema、表、视图等 |
| `sql_describe_object` | 获取列、索引、约束、DDL |
| `sql_execute` | 执行 SQL，返回 query_id 和流式结果事件 |
| `sql_cancel` | 取消正在执行的查询，必须真正终止服务端语句，非仅中断本地任务 |
| `sql_confirm_write` / `sql_rollback_write` | 对分类为 `Transaction` 的写操作，展示受影响行数后由用户确认提交或放弃 |
| `sql_explain` | 获取执行计划（能力支持时） |
| `sql_query_history` | 查询、收藏、删除历史 |
| `sql_export_result` | 将结果流式导出为 CSV/JSON |

长查询不通过单次 invoke 返回大对象：`sql_execute` 返回 `query_id`，通过 `sql://query/{id}/chunk`、`.../finished`、`.../error` 事件分块传递，并支持背压和取消。

## 7. 安全与治理

- 默认只读：数据源 `readonly=true` 时拒绝 `INSERT/UPDATE/DELETE/DDL`。危险 SQL 判定复用 `sqlparser`
  crate 对语句做 AST 分类（参考 [4.2.1](#421-参考实现rainfrog) rainfrog 的
  `Normal/Confirm/Transaction` 三分类，可正确识别 CTE 包裹的写操作），不用正则匹配关键字；
  解析失败或分类未知一律降级为需要确认，不放行。
- 生产环境增加"执行写操作"二次确认，可配置禁止写操作；UPDATE/DELETE 等支持事务回滚的语句先在
  事务中执行、展示数据库真实返回的受影响行数，确认后再 commit、取消则 rollback；DDL 等不支持
  事务内回滚的语句直接走确认弹窗，不进事务。
- 参数全部使用驱动参数绑定，禁止把用户输入直接拼接进连接或查询 SQL。
- 网络连接支持 TLS 证书校验、证书指纹确认和代理/SSH 隧道；禁止默认关闭证书校验。
- 查询超时、最大返回行数、最大结果字节数、并发数可按数据源配置；超限自动取消并提示。
- 审计记录保存数据源、用户、本地时间、SQL 摘要、是否写操作、结果状态；默认不记录完整敏感参数值。
- AI 发送前执行脱敏：表结构可发送，数据样本默认关闭；用户选择样本时先遮罩邮箱、手机号、身份证、Token 等字段。

## 8. AI 工具设计

右侧 AI 面板首期提供：

1. **生成 SQL**：根据自然语言和选中的表生成 SQL，结果先回填编辑器，不自动执行。
2. **解释 SQL**：说明语义、索引使用和潜在风险。
3. **优化 SQL**：结合执行计划提出建议，生成可对比的改写版本。
4. **修复错误**：将数据库错误、SQL 方言和相关片段发送给模型。
5. **表结构问答**：仅基于当前 Schema/DDL 回答，靠读取 [4.4](#44-本地目录缓存让-ai-像改代码一样改-sql)
   的 `schema_cache/` 文件获取上下文，不单独定制协议。

**交互机制**（详见 4.4，这里只讲落到 AI 面板上的行为）：

- 上面 1/3/4 三项本质上都是"AI 改一下当前标签页对应的 `.sql` 文件"，走已有编程助手的
  `stage_change` → `FileChangeCard`（应用/拒绝/整轮撤销）流程，**不会绕过确认直接覆盖编辑器
  内容**——"结果先回填编辑器"准确说是"生成一条 Pending 的文件改动，用户点应用后编辑器才更新"。
  这样"AI 生成 SQL"和"人在编辑器里手改 SQL"在撤销栈里是同一套语义，用户可以用同一个撤销入口
  处理两者。
- AI 如果需要执行 SQL 验证自己的改动（比如生成后跑一下 `EXPLAIN` 或小范围 `SELECT` 预览），
  用第 4.4 节新增的 `sql_run_query`/`sql_explain` 工具，这条路径复用第 7 节的
  `sqlparser` 分类和确认闸门，AI 发起的写操作和人手动执行遵守同一套确认流程，不允许 AI 专属
  的"自动执行"通道。
- AI 工具的文件读写作用域被硬限制在当前数据源的 `<cache_root>/sql/<data_source_id>/` 目录内
  （4.4 节里提到的安全边界），不会碰到其他数据源的缓存或工作区外的文件。

AI 工具调用统一复用现有 provider、SSE 和脱敏能力；增加 SQL 专用上下文长度上限、数据样本开关和“插入编辑器/复制/执行前确认”操作。

## 9. 实施分期与验收

### Phase 0：基础设施（1 周）

- 完成 migration、keyring 凭据引用、`DatabaseAdapter` trait、统一错误码和 command 骨架。
- 验收：可新增数据源配置、密码不出现在 SQLite 和日志中。

### Phase 1：MVP 查询工作区（2~3 周）

- 首页卡片和数据源列表。
- MySQL/PostgreSQL 连接测试、对象树、SQL 编辑器、结果表格、取消/超时、历史记录。
- 验收：100 万行级别查询不会阻塞 UI；1,000 行结果分页展示；断网和错误可恢复。

### Phase 2：多数据库与安全（2 周）

- openGauss、SQL Server、Oracle 适配器；TLS、只读策略、生产确认、审计、导出。
- 验收：五类数据库完成连接/元数据/查询核心场景；缺少 Oracle 依赖时有明确提示且不影响其他驱动。

### Phase 3：AI 与体验增强（1~2 周）

- AI 生成/解释/优化/错误修复，Schema 上下文和脱敏。
- SQL 格式化、执行计划、收藏、快捷键、布局持久化。

### 测试矩阵

- Rust：适配器、SQL policy、分页/取消、凭据引用、错误映射单测。
- 前端：向导校验、工作区状态、结果流事件、断线重连组件测试。
- 集成：Docker CI 启动 MySQL、PostgreSQL、openGauss、SQL Server；Oracle 使用可选自托管 runner 或模拟适配器。
- 安全：SQL 注入、敏感信息日志扫描、TLS 证书错误、生产写操作拦截。

## 10. 风险与决策点

| 风险 | 应对 |
|---|---|
| Oracle 原生依赖导致安装复杂 | 作为可选 feature；启动时探测并给出安装向导 |
| 各数据库元数据和分页语法差异 | 所有方言封装在 adapter，禁止 UI 直接写数据库 SQL |
| 大结果集占用内存 | 流式事件、虚拟表格、行数/字节上限、导出走临时文件 |
| AI 泄露业务数据 | 默认只传 DDL；样本显式授权并脱敏；审计 AI 请求摘要 |
| 多窗口/多会话资源泄漏 | session manager + RAII/关闭事件 + 空闲超时回收 |
| 驱动许可证和打包体积 | 在发布前建立依赖清单，按 feature 拆包并做许可证审查 |
| AI 文件工具越权读写工作区外文件（现有编程助手的已知缺口，见 4.4） | SQL 工作区的 `read_file`/`write_file`/`edit_file` 必须比照 `commands/fs.rs::guard_local_path` 做路径校验，硬限制在 `<cache_root>/sql/<data_source_id>/` 内，不能沿用现状的无边界实现 |

## 11. 首批开发任务清单

1. 确认现有 SQLite migration 最新编号并创建 SQL 桌面 migration。
2. 实现 `CredentialStore` 对数据源 secret 的保存/读取/删除封装。
3. 建立 `sql` domain、adapter registry、错误码和 Tauri commands。
4. 先接入 PostgreSQL/MySQL，打通测试连接、对象树、执行流和取消。
5. 在首页增加 SQL 桌面卡片及数据源 CRUD 向导。
6. 实现 `SqlWorkspace` 三栏布局、Monaco SQL 语言配置和虚拟结果表。
7. 接入历史记录、审计、只读/生产策略和导出。
8. 依次接入 openGauss、SQL Server、Oracle，并补齐集成测试。
9. 复用现有 AI provider，落地 4.4 节的本地目录缓存（标签页文件化、`schema_cache/`）、
   复用编程助手的 `ChangeStore` 状态机，并补齐 `read_file`/`write_file`/`edit_file` 的
   工作区路径越界校验（现有编程助手缺失，SQL 工作区不能照抄）。
10. 结果区实现表格/纯文本双模式渲染（`ResultTableView`/`ResultTextView`），按标签页记忆选择。
11. 更新用户文档、发布构建依赖说明和故障排查手册。

## 12. 完成定义（Definition of Done）

- 用户可从首页新增五类数据源并看到连接状态。
- 每个数据源可进入独立工作区，Schema/Table 懒加载、SQL 执行、停止、结果分页和历史均可用。
- 查询错误、超时、断开、取消都有可理解的中文提示，且不会阻塞其他工作区。
- 凭据不落明文；生产写操作、AI 执行、导出均有明确确认和审计。
- AI 对 SQL 标签页的改动都经过 Pending/Applied/Rejected/Undone 状态机确认，没有绕过用户确认
  直接改写编辑器内容的路径；AI 文件工具的读写范围经过校验，无法触达对应数据源缓存目录之外的文件。
- 结果区表格/纯文本两种展示模式均可用，按标签页记忆切换。
- `npm run test:web`、`npm run test:rust` 及数据库集成测试通过；文档中的驱动依赖和打包限制已验证。
