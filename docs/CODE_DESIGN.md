# roc_desk — 代码设计文档（Code Design）

> 本文档是 [../DESIGN.md](../DESIGN.md) 的实现层细化：给出完整的代码目录结构、每个文件/模块的职责、核心类型与接口签名、数据库 schema、IPC 命令与事件目录。目标是让任何一位工程师拿到本文档就能直接开始写某一个模块，而不需要先去猜接口形状。
>
> 约定：本文档中的 Rust 代码为**接口骨架**（签名 + 关键字段），不是完整实现；具体错误处理、生命周期标注等实现细节在编码阶段确定。

---

## 一、总体分层

```
┌─────────────────────────────────────────────────────────────────┐
│  src-web (React)                                                  │
│  components/ ── 纯展示 + 局部交互状态                              │
│  stores/     ── Zustand，跨组件共享状态                            │
│  services/   ── 唯一允许调用 invoke()/listen() 的地方              │
│  types/      ── 与后端共享的类型（tauri-specta 自动生成）          │
└───────────────────────────────┬───────────────────────────────────┘
                                 │ Tauri IPC (invoke / emit)
┌───────────────────────────────┴───────────────────────────────────┐
│  src-tauri (Rust)                                                  │
│  commands/   ── #[tauri::command] 薄封装，只做参数校验+调用 domain  │
│  {ssh,sftp,log,ai,coding,plugin,browser,connection}/               │
│                ── 领域逻辑（domain layer），不依赖 tauri::State     │
│                   之外的任何 Tauri 类型，便于单元测试                │
│  db/         ── SQLite 连接池 + 迁移 + 各领域的 repository          │
│  state.rs    ── AppState，聚合所有 Manager，注入到 tauri::Manager   │
│  error.rs    ── 统一错误类型，实现 serde::Serialize 供前端消费       │
└─────────────────────────────────────────────────────────────────────┘
```

**分层原则**：
- `commands/*` 是唯一暴露给前端的入口，函数体只做「取 State → 调 domain 方法 → 转换错误 → 返回」，不写业务逻辑。这样领域逻辑可以脱离 Tauri runtime 直接用 `cargo test` 测试。
- 领域模块之间通过 trait（如 `FileOps`、`CredentialStore`）解耦，`coding` 模块调用 `ssh`/`sftp` 时不直接依赖具体类型，而是依赖 `FileOps` trait，本地/远程实现可互换。
- 前端 `services/` 是 IPC 边界，`components/` 永远不直接 `import { invoke } from '@tauri-apps/api'`，便于将来做 mock 和测试。

---

## 二、完整目录结构（带职责说明）

```
devhub/
├── src-tauri/
│   ├── Cargo.toml
│   ├── tauri.conf.json
│   ├── capabilities/                    # Tauri 2.0 权限模型（取代 1.x allowlist）
│   │   ├── main-window.json             # 主窗口可用的 Command 白名单
│   │   └── browser-webview.json         # 网页浏览子 WebView 的极小权限集（见 UI_DESIGN.md）
│   ├── migrations/                      # SQLite 迁移脚本（见五、数据库设计）
│   │   ├── 0001_init.sql
│   │   ├── 0002_connections.sql
│   │   ├── 0003_logs_fts.sql
│   │   ├── 0004_coding_sessions.sql
│   │   ├── 0005_audit_log.sql
│   │   └── 0006_workspaces.sql          # 工作区档案 + 最近打开列表（DESIGN.md §3.1.1）
│   ├── icons/
│   └── src/
│       ├── main.rs                      # 入口：初始化 tracing、AppState、注册 Command/插件
│       ├── lib.rs                       # mod 声明 + tauri::Builder 组装（供集成测试复用）
│       ├── state.rs                     # AppState：聚合各 Manager，Arc<Mutex/RwLock> 包裹
│       ├── error.rs                     # AppError（thiserror）+ Serialize 实现
│       ├── events.rs                    # 全局 Tauri Event 名称常量（避免字符串硬编码散落各处）
│       │
│       ├── db/
│       │   ├── mod.rs
│       │   ├── pool.rs                  # r2d2/deadpool 连接池封装
│       │   ├── migrate.rs               # 迁移执行器（启动时自动 apply migrations/*.sql）
│       │   └── repo/                    # 每张表一个 repository，封装 SQL，禁止 SQL 散落业务代码
│       │       ├── connections_repo.rs
│       │       ├── known_hosts_repo.rs
│       │       ├── logs_repo.rs
│       │       ├── chat_repo.rs
│       │       ├── coding_session_repo.rs
│       │       ├── audit_repo.rs
│       │       └── workspace_repo.rs    # WorkspaceProfile + 最近打开列表
│       │
│       ├── credential/                  # 系统密钥链封装（对应 DESIGN.md §八-1）
│       │   ├── mod.rs
│       │   └── keyring_store.rs         # CredentialStore trait 的 keyring 实现
│       │
│       ├── connection/                  # 连接档案管理（对应 DESIGN.md §3.2.2）
│       │   ├── mod.rs
│       │   ├── profile.rs               # ConnectionProfile 模型 + CRUD
│       │   └── group.rs                 # 分组/标签
│       │
│       ├── workspace/                   # 工作区模型（DESIGN.md §3.1.1，应用入口）
│       │   ├── mod.rs
│       │   ├── profile.rs               # WorkspaceProfile：Local/Remote + 根路径 + 关联连接
│       │   └── recent.rs                # 最近打开列表的排序/清理
│       │
│       ├── fsops/                       # 本地/远程统一文件操作（原属 coding/file_ops.rs，
│       │   ├── mod.rs                   # 独立出来供 Explorer、SFTP 快捷工具、Coding Agent 共用，
│       │   ├── local.rs                 # 避免三处各自实现一套本地/远程文件读写）
│       │   └── remote.rs                # 含 §3.1.4 远程文件编辑所需的 mtime 冲突检测
│       │
│       ├── ssh/
│       │   ├── mod.rs
│       │   ├── session.rs               # SshSession：单条物理连接 + 多 Channel 复用
│       │   ├── pool.rs                  # SshConnectionPool：按 host 复用连接（DESIGN.md §3.2.2）
│       │   ├── handler.rs               # russh::client::Handler 实现，含 check_server_key
│       │   ├── known_hosts.rs           # TOFU / 指纹比对逻辑
│       │   ├── reconnect.rs             # 指数退避重连策略（DESIGN.md §3.2.3）
│       │   └── sftp.rs                  # SFTP 会话封装（基于 russh-sftp）
│       │
│       ├── log/
│       │   ├── mod.rs
│       │   ├── engine.rs                # LogSearchEngine：FTS5 查询
│       │   ├── importer.rs              # 日志文件 → SQLite 增量导入（流式，避免整文件入内存）
│       │   ├── remote.rs                # 远程日志获取：SSH exec `tail -f` / rg 直传
│       │   ├── parser.rs                # 日志行解析（时间戳/级别提取，多格式适配）
│       │   └── retention.rs             # 索引磁盘配额与 LRU 清理（DESIGN.md §十-3）
│       │
│       ├── ai/
│       │   ├── mod.rs
│       │   ├── chat.rs                  # AiChat：SSE 流式对话
│       │   ├── providers.rs             # Provider 枚举与适配（OpenAI 兼容 / Ollama / 豆包）
│       │   └── redaction.rs             # 发送前敏感信息脱敏（DESIGN.md §3.6）
│       │
│       ├── coding/                      # AI 编程助手模块（依赖 fsops，不再自带 file_ops）
│       │   ├── mod.rs
│       │   ├── session.rs               # CodingSession：对话+变更历史+undo/redo，target 自动继承工作区
│       │   ├── tools.rs                 # AgentTool 定义 + 工具执行分发（依赖 fsops::FileOps，不再自带）
│       │   ├── diff.rs                  # 基于 similar crate 生成/应用 diff
│       │   ├── guard.rs                 # 高危命令黑名单 + 确认流程（DESIGN.md §3.8.2.1）
│       │   ├── mcp.rs                   # MCP 客户端管理（rmcp）
│       │   └── git_ops.rs               # 自动提交（git2）
│       │
│       ├── plugin/
│       │   ├── mod.rs
│       │   ├── lua_engine.rs            # mlua 运行时 + 受限全局表注册
│       │   └── sandbox.rs               # 移除 os.execute/io.popen，代理受控 API
│       │
│       ├── browser/
│       │   ├── mod.rs
│       │   ├── bookmark.rs
│       │   └── isolation.rs             # 子 WebView 创建 + CSP 配置（DESIGN.md §3.5）
│       │
│       └── commands/                    # #[tauri::command] 薄封装层，按领域拆分
│           ├── mod.rs                   # 统一 invoke_handler![...] 收集
│           ├── workspace.rs             # 打开/最近工作区、Explorer 树读取
│           ├── fs.rs                    # Explorer/编辑器用的文件读写（含冲突检测），复用 fsops
│           ├── connection.rs
│           ├── ssh.rs
│           ├── sftp.rs
│           ├── log_search.rs
│           ├── ai.rs
│           ├── coding.rs
│           ├── plugin.rs
│           └── browser.rs
│
├── src-web/
│   ├── package.json
│   ├── vite.config.ts
│   ├── index.html
│   ├── src/
│   │   ├── main.tsx
│   │   ├── App.tsx                      # 顶层路由：TabBar + Sidebar + 内容区 Switch
│   │   │
│   │   ├── types/
│   │   │   └── bindings.ts              # tauri-specta 自动生成，禁止手改
│   │   │
│   │   ├── services/                    # IPC 边界，一一对应 commands/*.rs
│   │   │   ├── workspaceService.ts      # 打开/最近工作区
│   │   │   ├── fsService.ts             # Explorer/编辑器读写文件（含冲突检测）
│   │   │   ├── connectionService.ts
│   │   │   ├── sshService.ts
│   │   │   ├── sftpService.ts
│   │   │   ├── logSearchService.ts
│   │   │   ├── aiService.ts
│   │   │   ├── codingService.ts
│   │   │   ├── pluginService.ts
│   │   │   └── eventBus.ts              # 统一封装 listen()，映射 events.rs 中的常量
│   │   │
│   │   ├── stores/                      # Zustand，见 六、前端状态管理
│   │   │   ├── workspaceStore.ts        # 当前工作区、最近工作区列表
│   │   │   ├── explorerStore.ts         # Explorer 树展开状态、选中项
│   │   │   ├── editorStore.ts           # 打开的编辑器标签、dirty 状态、预览态/固定态
│   │   │   ├── tabsStore.ts
│   │   │   ├── connectionStore.ts
│   │   │   ├── sshStore.ts
│   │   │   ├── sftpStore.ts
│   │   │   ├── logSearchStore.ts
│   │   │   ├── aiChatStore.ts
│   │   │   ├── codingStore.ts
│   │   │   ├── settingsStore.ts
│   │   │   └── uiStore.ts               # 主题、面板尺寸等纯 UI 状态
│   │   │
│   │   ├── hooks/
│   │   │   ├── useWorkspace.ts
│   │   │   ├── useSshSession.ts
│   │   │   ├── useFileEditor.ts         # 打开/编辑/保存/冲突处理的统一逻辑（Explorer 与 SFTP 快捷工具共用）
│   │   │   ├── useKeyboardShortcuts.ts
│   │   │   └── useTauriEvent.ts
│   │   │
│   │   ├── components/
│   │   │   ├── Layout/
│   │   │   │   ├── TabBar.tsx           # Tab 列表 + 右侧顶部快捷工具图标簇（DESIGN.md §3.1.3）
│   │   │   │   ├── QuickToolsBar.tsx    # 📂SFTP / 🌐浏览器 / 💬AI问答 / 🛠AI编程，find-or-create 打开逻辑
│   │   │   │   ├── Sidebar.tsx          # Activity Bar 容器（Explorer/Search/Servers/Settings 图标）
│   │   │   │   ├── StatusBar.tsx
│   │   │   │   └── SplitPane.tsx        # react-resizable-panels 封装
│   │   │   ├── Workspace/               # 工作区选择页（应用入口，DESIGN.md §3.1.1）
│   │   │   │   ├── WorkspacePicker.tsx  # "打开本地文件夹" / "连接远程主机并选择目录"
│   │   │   │   ├── RecentWorkspaceList.tsx
│   │   │   │   └── RemoteDirPicker.tsx  # 连接成功后选根目录的轻量远程目录浏览器
│   │   │   ├── Explorer/                # 工作区文件树（替代原侧边栏连接树，成为默认 Sidebar 内容）
│   │   │   │   ├── ExplorerTree.tsx     # VS Code 式：单击预览态/双击固定，懒加载子目录
│   │   │   │   ├── ExplorerNode.tsx
│   │   │   │   └── ExplorerContextMenu.tsx  # 新建文件/文件夹、重命名、删除、在 SFTP 中打开
│   │   │   ├── ConnectionManager/       # 连接档案 CRUD，从 WorkspacePicker 或 Activity Bar「服务器」呼出
│   │   │   │   ├── ConnectionTree.tsx   # 分组/标签树
│   │   │   │   ├── ConnectionForm.tsx   # 新建/编辑档案
│   │   │   │   └── HostKeyDialog.tsx    # TOFU / 指纹变化告警弹窗
│   │   │   ├── Terminal/
│   │   │   │   ├── TerminalView.tsx     # xterm.js 挂载（远程=SSH Channel，本地=portable-pty）
│   │   │   │   └── TerminalToolbar.tsx
│   │   │   ├── Editor/                  # 通用可编辑代码编辑器，Explorer 单击打开 与 CodingAgent 右栏共用
│   │   │   │   ├── CodeEditor.tsx       # Monaco，非只读，Ctrl+S 保存，dirty 圆点
│   │   │   │   ├── ConflictDialog.tsx   # §3.1.4 远程文件保存前的 mtime 冲突提示
│   │   │   │   └── LargeFileBanner.tsx  # 大文件分级提示条（§3.3.1）
│   │   │   ├── SftpBrowser/             # 收窄为跨目录浏览/传输管理器（DESIGN.md §3.3），非默认打开
│   │   │   │   ├── SftpBrowser.tsx
│   │   │   │   ├── FileList.tsx
│   │   │   │   └── TransferProgress.tsx
│   │   │   ├── LogSearch/
│   │   │   │   ├── LogSearchPanel.tsx
│   │   │   │   ├── SearchConditionForm.tsx
│   │   │   │   ├── ResultList.tsx       # @tanstack/react-virtual 虚拟滚动
│   │   │   │   └── LogDetailPane.tsx
│   │   │   ├── WebBrowser/
│   │   │   │   ├── WebBrowserView.tsx
│   │   │   │   └── BookmarkBar.tsx
│   │   │   ├── AiChat/
│   │   │   │   ├── ChatPanel.tsx
│   │   │   │   ├── MessageBubble.tsx
│   │   │   │   └── ProviderSelector.tsx
│   │   │   ├── CodingAgent/
│   │   │   │   ├── CodingAgent.tsx      # 三栏布局容器：左栏直接复用 <ExplorerTree>，右栏复用 <CodeEditor>
│   │   │   │   ├── TargetBadge.tsx      # 自动绑定的目标提示 + "更改"覆盖入口（DESIGN.md §3.8.1）
│   │   │   │   ├── RemoteCapabilityBadge.tsx  # "远程模式：无智能补全" 常驻徽标（§3.8.7）
│   │   │   │   ├── ToolCallProgress.tsx # 工具调用中的耗时反馈（§3.8.7 性能诚实）
│   │   │   │   ├── ChatPanel.tsx
│   │   │   │   ├── DiffViewer.tsx
│   │   │   │   ├── FileChangeCard.tsx
│   │   │   │   └── CommandConfirmDialog.tsx  # run_command 二次确认
│   │   │   ├── Settings/
│   │   │   │   ├── SettingsPanel.tsx
│   │   │   │   ├── ProviderSettings.tsx
│   │   │   │   ├── SecuritySettings.tsx # 脱敏开关、命令黑名单查看
│   │   │   │   ├── ShortcutSettings.tsx
│   │   │   │   └── PluginManager.tsx
│   │   │   └── shared/                  # 跨模块复用的基础组件（基于 shadcn/ui 二次封装）
│   │   │       ├── Toast.tsx            # success/info/error 三态（UI_DESIGN.md §十一，docs/prototypes/devhub-dialogs.html §7）
│   │   │       ├── ConfirmDialog.tsx    # 通用确认弹窗骨架，HostKeyDialog/CommandConfirmDialog/ConflictDialog 均基于它二次封装
│   │   │       ├── SkeletonText.tsx
│   │   │       ├── EmptyState.tsx
│   │   │       ├── StatusDot.tsx        # connected/connecting/disconnected/error 四态圆点（dialogs.html §6），SSH 状态/工作区列表/Tab 复用
│   │   │       ├── ToggleSwitch.tsx     # 设置页开关（dialogs.html §7），支持 disabled 态（如"高危命令黑名单拦截"不可关闭）
│   │   │       └── SegmentedControl.tsx # 日志搜索"实时/索引"、原本散落各处的二选一控件统一收口（dialogs.html §7）
│   │   └── styles/
│   │       ├── globals.css
│   │       └── theme.css                # CSS 变量，见 UI_DESIGN.md 设计令牌
│   └── public/
│
├── plugins/                             # 用户 Lua 插件目录
├── mcp-servers/
├── docs/
│   ├── CODE_DESIGN.md                   # 本文件
│   └── UI_DESIGN.md
└── DESIGN.md
```

---

## 三、后端核心类型与接口

### 3.1 `state.rs` — 应用状态聚合

```rust
pub struct AppState {
    pub db: DbPool,
    pub credential_store: Arc<dyn CredentialStore>,
    pub ssh_pool: Arc<SshConnectionPool>,
    pub workspaces: Arc<RwLock<HashMap<Uuid, WorkspaceHandle>>>, // 当前窗口已打开的工作区（见 §3.8）
    pub log_engine: Arc<LogSearchEngine>,
    pub ai_providers: Arc<RwLock<ProviderRegistry>>,
    pub coding_sessions: Arc<RwLock<HashMap<Uuid, CodingSession>>>,
    pub mcp_manager: Arc<RwLock<McpManager>>,
    pub lua_engine: Arc<PluginEngine>,
}
```

所有 `#[tauri::command]` 函数通过 `State<'_, AppState>` 取用，`AppState` 在 `main.rs` 中一次性构造并 `app.manage(state)`。

### 3.2 统一错误类型 `error.rs`

```rust
#[derive(Debug, thiserror::Error, serde::Serialize)]
#[serde(tag = "kind", content = "message")]
pub enum AppError {
    #[error("connection failed: {0}")]
    Connection(String),
    #[error("authentication failed: {0}")]
    Auth(String),
    #[error("host key verification failed: {0}")]
    HostKeyRejected(String),
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("database error: {0}")]
    Database(String),
    #[error("internal error: {0}")]
    Internal(String),
}
```

所有 domain 层方法返回 `Result<T, AppError>`；`commands/*` 直接透传（`AppError` 已实现 `Serialize`，前端拿到的是 `{ kind, message }` 而不是裸字符串，便于按错误类型分支处理，例如 `HostKeyRejected` 单独弹出 `HostKeyDialog`）。

### 3.3 `credential/mod.rs` — 凭据存储抽象

```rust
#[async_trait]
pub trait CredentialStore: Send + Sync {
    async fn set(&self, key: &str, secret: &str) -> Result<(), AppError>;
    async fn get(&self, key: &str) -> Result<Option<String>, AppError>;
    async fn delete(&self, key: &str) -> Result<(), AppError>;
}

pub struct KeyringStore { service_name: &'static str }
```

`ConnectionProfile` 表中只存 `credential_ref: String`（形如 `ssh:{profile_id}:password`），实际密文经 `CredentialStore` 存取，数据库文件本身不含任何机密。

### 3.4 `ssh/pool.rs` — 连接复用

```rust
pub struct SshConnectionPool {
    sessions: RwLock<HashMap<HostKey, Arc<SshSession>>>, // HostKey = (host, port, user)
}

impl SshConnectionPool {
    pub async fn get_or_connect(&self, profile: &ConnectionProfile) -> Result<Arc<SshSession>, AppError>;
    pub async fn open_channel(&self, session: &SshSession, kind: ChannelKind) -> Result<ChannelId, AppError>;
    pub async fn disconnect(&self, key: &HostKey);
}

pub enum ChannelKind { Shell, Sftp, Exec(String) }
```

同一主机的终端、SFTP、`run_command` 执行共用一条 `SshSession`，各自开独立 `Channel`（对应 DESIGN.md §3.2.2 的多路复用要求）。

### 3.5 `fsops/mod.rs` — 本地/远程操作统一接口

> 从早期的 `coding/file_ops.rs` 独立为顶层模块（DESIGN.md §3.1.4）：Explorer 打开文件、SFTP 快捷工具、Coding Agent 三处都要读写本地/远程文件，放在 `coding/` 下会造成 Explorer 反向依赖 `coding` 模块，违反 §九 的依赖方向约束。

```rust
#[async_trait]
pub trait FileOps: Send + Sync {
    async fn read_file(&self, path: &str) -> Result<FileContent, AppError>;
    async fn write_file(&self, path: &str, content: &str, expected_mtime: Option<i64>) -> Result<WriteOutcome, AppError>;
    async fn list_dir(&self, path: &str) -> Result<Vec<FileEntry>, AppError>;
    async fn search(&self, pattern: &str, path: &str) -> Result<Vec<SearchResult>, AppError>;
    async fn exec_command(&self, cmd: &str, cwd: Option<&str>) -> Result<CommandOutput, AppError>;
}

pub struct FileContent { pub text: String, pub mtime: i64 }

pub enum WriteOutcome {
    Written { mtime: i64 },
    Conflict { current_mtime: i64, current_preview: String }, // 见下方冲突检测
}

pub struct LocalFileOps;
pub struct RemoteFileOps { session: Arc<SshSession> }
```

**§3.1.4 远程文件编辑冲突检测的落地**：`write_file` 接收调用方在"打开文件那一刻"记下的 `expected_mtime`；`RemoteFileOps::write_file` 写入前先 `stat` 远程文件当前 `mtime`，与 `expected_mtime` 不一致时返回 `WriteOutcome::Conflict`（附带远程当前内容的前几行预览，供前端渲染差异提示）而不是直接覆盖，由 `commands/fs.rs::fs_write_file` 透传给前端弹出 `ConflictDialog`（DESIGN.md §3.1.4）。`LocalFileOps` 同样实现该检查（本地文件也可能被外部程序改过），保持两种实现行为一致。

`CodingSession` 与 Explorer/SFTP 的编辑器共用同一个 `Box<dyn FileOps>` 实现来源；`CodingSession` 创建时根据当前工作区（`WorkspaceProfile::kind`）自动注入，不再由用户手动选择（DESIGN.md §3.8.1）。

### 3.6 `coding/guard.rs` — 高危命令拦截

```rust
pub struct CommandGuard { blocklist: Vec<Regex>, allowlist: HashSet<String> }

pub enum GuardVerdict {
    Blocked(String),          // 命中黑名单，直接拒绝，附拒绝原因
    RequiresConfirmation,     // 默认路径：需要前端弹窗确认
    AutoAllowed,              // 命中用户自定义只读白名单
}

impl CommandGuard {
    pub fn evaluate(&self, cmd: &str) -> GuardVerdict;
}
```

`commands/coding.rs::run_command` 在真正执行前必须先调用 `CommandGuard::evaluate`；`RequiresConfirmation` 时通过 Tauri Event 通知前端弹出 `CommandConfirmDialog`，等待前端 `confirm_command` 命令回传结果后才继续（用 `tokio::sync::oneshot` 挂起等待）。

### 3.7 `ssh/handler.rs` — 主机指纹校验

```rust
pub struct SshHandler {
    host: String,
    port: u16,
    known_hosts: Arc<KnownHostsRepo>,
    trust_prompt: mpsc::Sender<TrustPromptRequest>, // 转发到前端弹窗，等待用户确认
}

impl russh::client::Handler for SshHandler {
    type Error = AppError;
    async fn check_server_key(&mut self, key: &PublicKey) -> Result<bool, Self::Error> { /* 见 DESIGN.md §3.2.1 */ }
}
```

### 3.8 `workspace/mod.rs` — 工作区模型（应用入口）

```rust
#[derive(Serialize, Deserialize, specta::Type)]
pub struct WorkspaceProfile {
    pub id: Uuid,
    pub kind: WorkspaceKind,
    pub root_path: String,             // 本地绝对路径 或 远程主机上的绝对路径
    pub connection_id: Option<Uuid>,   // kind = Remote 时必填，关联 ConnectionProfile
    pub display_name: String,
    pub last_opened_at: Option<String>,
}

#[derive(Serialize, Deserialize, specta::Type)]
pub enum WorkspaceKind { Local, Remote }

/// 当前窗口内已打开的工作区运行时句柄（不落库，进程内状态）
pub struct WorkspaceHandle {
    pub profile: WorkspaceProfile,
    pub file_ops: Arc<dyn FileOps>,          // 打开时根据 kind 注入 LocalFileOps / RemoteFileOps
    pub default_terminal_channel: Option<ChannelId>, // §3.1.2 默认自动打开的终端
}

pub struct WorkspaceManager { repo: Arc<WorkspaceRepo> }

impl WorkspaceManager {
    pub async fn open_local(&self, path: &str) -> Result<WorkspaceHandle, AppError>;
    pub async fn open_remote(&self, connection_id: Uuid, remote_path: &str) -> Result<WorkspaceHandle, AppError>;
    pub async fn list_recent(&self, limit: usize) -> Result<Vec<WorkspaceProfile>, AppError>;
    pub async fn remove_from_recent(&self, id: Uuid) -> Result<(), AppError>;
}
```

`open_remote` 内部先经 `SshConnectionPool::get_or_connect`（复用 §3.4 的连接池，走 §3.2.1 的指纹校验），成功后用该 `SshSession` 构造 `RemoteFileOps` 并打开一个默认 Shell `Channel`；`AppState.workspaces` 以 `WorkspaceProfile.id` 为 key 缓存 `WorkspaceHandle`，供该工作区下所有 Tab（Explorer、终端、SFTP、Coding Agent）共享同一份文件操作句柄和连接，不重复握手。

---

## 四、Tauri Command 目录

> 所有命令统一放在 `commands/` 下按领域文件组织；下表为**接口契约**，前端 `services/*.ts` 与之一一对应，字段变更需同步更新 `types/bindings.ts`（tauri-specta 自动生成，禁止手改）。

### 4.0 workspace / fs（应用入口，DESIGN.md §3.1）

| Command | 参数 | 返回 | 说明 |
|---------|------|------|------|
| `workspace_list_recent` | `limit?: number` | `WorkspaceProfile[]` | 工作区选择页的"最近打开"列表 |
| `workspace_open_local` | `path` | `WorkspaceHandle` | 原生目录选择器选中后调用 |
| `workspace_browse_remote_dir` | `connection_id, path` | `RemoteDirEntry[]` | 供 `RemoteDirPicker` 选根目录时逐层浏览 |
| `workspace_open_remote` | `connection_id, remote_path` | `WorkspaceHandle` | 确认远程根目录后打开工作区，内部建连+指纹校验 |
| `workspace_remove_recent` | `id` | `()` | 从最近列表移除（不影响已打开的其他窗口） |
| `workspace_close` | `id` | `()` | 关闭工作区，释放 `WorkspaceHandle`（是否断开底层 SSH 连接取决于是否还有其他工作区/Tab 在用） |
| `fs_list_dir` | `workspace_id, path` | `FileEntry[]` | Explorer 懒加载子目录，内部路由到该工作区的 `FileOps::list_dir` |
| `fs_read_file` | `workspace_id, path` | `FileContent` | 打开编辑器时读取，返回内容+`mtime` |
| `fs_write_file` | `workspace_id, path, content, expected_mtime` | `WriteOutcome` | 保存；`Conflict` 时前端弹 `ConflictDialog`（§3.1.4） |
| `fs_search` | `workspace_id, pattern, path?` | `SearchResult[]` | `Ctrl+Shift+F` 全工作区搜索，复用 §3.4 日志搜索引擎的同一套本地/远程双模式 |

### 4.1 connection（连接档案）

| Command | 参数 | 返回 | 说明 |
|---------|------|------|------|
| `connection_list` | `group_id?: string` | `ConnectionProfile[]` | 列出档案（可按分组过滤） |
| `connection_create` | `ConnectionProfileInput` | `ConnectionProfile` | 创建档案，敏感字段写入 keyring |
| `connection_update` | `id, ConnectionProfileInput` | `ConnectionProfile` | — |
| `connection_delete` | `id` | `()` | 同时清理 keyring 中的凭据 |
| `connection_test` | `id` | `ConnectionTestResult` | 试连接，不落地会话 |

### 4.2 ssh / sftp

| Command | 参数 | 返回 | 说明 |
|---------|------|------|------|
| `ssh_connect` | `profile_id` | `session_id` | 建连或复用连接池中的连接 |
| `ssh_open_shell` | `session_id, rows, cols` | `channel_id` | 打开终端 Channel |
| `ssh_write` | `channel_id, data: bytes` | `()` | 键盘输入写入远端 |
| `ssh_resize` | `channel_id, rows, cols` | `()` | 终端 resize |
| `ssh_disconnect` | `session_id` | `()` | — |
| `ssh_confirm_host_key` | `request_id, trust: bool` | `()` | 响应 TOFU / 指纹变化弹窗 |
| `sftp_list_dir` | `session_id, path` | `FileEntry[]` | — |
| `sftp_read_file_stream` | `session_id, path` | `()`（结果走事件） | 见 §5 事件目录 |
| `sftp_read_file_paged` | `session_id, path, offset, length` | `string` | 大文件分页读取 |
| `sftp_upload` | `session_id, local_path, remote_path` | `()`（进度走事件） | — |
| `sftp_download` | `session_id, remote_path, local_path` | `()`（进度走事件） | — |
| `sftp_delete` | `session_id, path` | `()` | 需前端二次确认 |

### 4.3 log_search

| Command | 参数 | 返回 | 说明 |
|---------|------|------|------|
| `log_import_file` | `session_id, remote_path` | `import_job_id` | 远程日志下载+导入，走事件报进度 |
| `log_search_index` | `LogQuery` | `SearchResult[]` | 模式 B：本地 FTS5 索引搜索 |
| `log_search_live` | `session_id, LogQuery, path` | `()`（结果走事件） | 模式 A：远程 ripgrep 实时搜索 |
| `log_index_stats` | `—` | `IndexStats` | 索引磁盘占用，供设置页展示与清理 |
| `log_index_clear` | `older_than?: string` | `()` | LRU / 手动清理 |

### 4.4 ai

| Command | 参数 | 返回 | 说明 |
|---------|------|------|------|
| `ai_list_providers` | `—` | `ProviderConfig[]` | — |
| `ai_save_provider` | `ProviderConfigInput` | `ProviderConfig` | API Key 写入 keyring |
| `ai_chat_stream` | `provider_id, messages` | `()`（内容走事件 SSE） | — |
| `ai_cancel_stream` | `stream_id` | `()` | — |

### 4.5 coding

| Command | 参数 | 返回 | 说明 |
|---------|------|------|------|
| `coding_create_session` | `workspace_id, target_override?: connection_id` | `session_id` | 默认 target 取自 `workspace_id` 对应的 `WorkspaceHandle`（DESIGN.md §3.8.1），`target_override` 仅供「更改目标」高级入口使用 |
| `coding_send_message` | `session_id, text, attachments?` | `()`（回复走事件流） | — |
| `coding_set_mode` | `session_id, mode: plan\|build` | `()` | — |
| `coding_accept_change` | `session_id, change_id` | `()` | — |
| `coding_reject_change` | `session_id, change_id` | `()` | — |
| `coding_undo` | `session_id` | `()` | — |
| `coding_redo` | `session_id` | `()` | — |
| `coding_confirm_command` | `request_id, approved: bool` | `()` | 响应 `CommandGuard::RequiresConfirmation` |

### 4.6 plugin / browser

| Command | 参数 | 返回 | 说明 |
|---------|------|------|------|
| `plugin_list` | `—` | `PluginMeta[]` | — |
| `plugin_load` | `path` | `()` | 沙箱内加载并注册命令 |
| `plugin_run_command` | `name, args` | `serde_json::Value` | — |
| `browser_navigate` | `webview_id, url` | `()` | — |
| `browser_bookmark_add` | `title, url` | `Bookmark` | — |

---

## 五、数据库设计（SQLite）

单文件 `devhub.db`，通过 `migrations/*.sql` 顺序管理 schema 版本。

```sql
-- 0001_init.sql
CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);

-- 0002_connections.sql
CREATE TABLE connection_groups (
    id TEXT PRIMARY KEY, name TEXT NOT NULL, parent_id TEXT REFERENCES connection_groups(id)
);
CREATE TABLE connections (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    host TEXT NOT NULL,
    port INTEGER NOT NULL DEFAULT 22,
    username TEXT NOT NULL,
    auth_method TEXT NOT NULL,          -- 'password' | 'key' | 'agent'
    credential_ref TEXT,                -- keyring 条目引用，机密不落库
    group_id TEXT REFERENCES connection_groups(id),
    tags TEXT,                          -- JSON array
    jump_host_id TEXT REFERENCES connections(id),
    last_connected_at TEXT,
    created_at TEXT NOT NULL
);
CREATE TABLE known_hosts (
    host TEXT NOT NULL, port INTEGER NOT NULL,
    fingerprint TEXT NOT NULL, trusted_at TEXT NOT NULL,
    PRIMARY KEY (host, port)
);

-- 0003_logs_fts.sql（字段设计见 DESIGN.md §3.4.2，非文本字段标 UNINDEXED）
CREATE VIRTUAL TABLE logs USING fts5(
    content, file_path UNINDEXED, line_number UNINDEXED,
    timestamp UNINDEXED, log_level UNINDEXED, host_name UNINDEXED,
    tokenize = 'unicode61'
);
CREATE TABLE log_import_jobs (
    id TEXT PRIMARY KEY, host_name TEXT, file_path TEXT,
    status TEXT, bytes_total INTEGER, bytes_done INTEGER, created_at TEXT
);

-- 0004_coding_sessions.sql
CREATE TABLE coding_sessions (
    id TEXT PRIMARY KEY, target_type TEXT, project_root TEXT,
    mode TEXT DEFAULT 'plan', created_at TEXT
);
CREATE TABLE coding_messages (
    id TEXT PRIMARY KEY, session_id TEXT REFERENCES coding_sessions(id),
    role TEXT, content TEXT, created_at TEXT
);
CREATE TABLE file_changes (
    id TEXT PRIMARY KEY, session_id TEXT REFERENCES coding_sessions(id),
    path TEXT, old_content TEXT, new_content TEXT, diff TEXT,
    status TEXT,                        -- pending | applied | rejected | undone
    created_at TEXT
);

-- 0005_audit_log.sql（DESIGN.md §八-8）
CREATE TABLE audit_log (
    id TEXT PRIMARY KEY, occurred_at TEXT NOT NULL,
    actor_session_id TEXT, target_host TEXT,
    action TEXT NOT NULL,               -- e.g. 'run_command' | 'sftp_delete' | 'ssh_connect'
    detail TEXT,                        -- JSON，如命令原文、被拒绝原因
    result TEXT                         -- 'allowed' | 'blocked' | 'error'
);

-- 0006_workspaces.sql（DESIGN.md §3.1.1，应用入口）
CREATE TABLE workspaces (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,                 -- 'local' | 'remote'
    root_path TEXT NOT NULL,
    connection_id TEXT REFERENCES connections(id),  -- kind='remote' 时必填
    display_name TEXT NOT NULL,
    last_opened_at TEXT,
    created_at TEXT NOT NULL
);
```

**索引与容量注意**（对应 DESIGN.md §十-3）：`logs` FTS5 表按需为每个 `import_job` 生成独立的逻辑分区（`file_path` 前缀过滤即可，暂不拆分物理表），`log_index_clear` 命令基于 `log_import_jobs.created_at` 做 LRU 清理，避免无限增长。

---

## 六、前端状态管理（Zustand stores）

| Store | 职责 | 关键字段 |
|-------|------|----------|
| `workspaceStore` | 当前打开的工作区、最近工作区列表 | `current: WorkspaceProfile \| null`, `recent: WorkspaceProfile[]` |
| `explorerStore` | Explorer 树的展开/加载状态、选中项 | `expandedPaths: Set<string>`, `loadingPaths: Set<string>`, `selection` |
| `editorStore` | 打开的编辑器标签、内容缓冲、dirty 状态、预览态/固定态 | `buffers: Map<tabId, { content, mtime, dirty, isPreview }>` |
| `tabsStore` | 打开的标签页列表与激活项（含 find-or-create 逻辑，见 DESIGN.md §3.1.3） | `tabs: Tab[]`, `activeTabId` |
| `connectionStore` | 连接档案树 + 各会话连接状态 | `profiles`, `sessionStatus: Map<sessionId, 'connecting'\|'connected'\|'reconnecting'\|'disconnected'>` |
| `sshStore` | 每个终端 Channel 的缓冲区引用（xterm 实例由组件自持，store 只存元数据） | `channels: Map<channelId, ChannelMeta>` |
| `sftpStore` | SFTP 快捷工具当前目录、选中项、传输队列（不再是 Explorer 的数据源） | `cwd`, `selection`, `transfers: TransferTask[]` |
| `logSearchStore` | 搜索条件、结果、分页游标 | `query`, `results`, `mode: 'live'\|'index'` |
| `aiChatStore` | 各 Provider 的会话历史 | `sessions: Map<providerId, ChatSession>` |
| `codingStore` | Coding Agent 的 mode/messages/changes/undo 栈/当前目标 | 与后端 `CodingSession` 字段对齐，前端只读，写操作一律经 `codingService` |
| `settingsStore` | 主题、快捷键、脱敏开关等本地偏好 | 持久化到 `tauri-plugin-store`（非敏感数据） |
| `uiStore` | 面板宽度、侧边栏折叠等纯 UI 状态 | 不落库，随窗口生命周期 |

**约定**：所有 store 的写操作只能来自 `services/*` 或 `eventBus` 的回调，组件内不允许直接 `set()` 领域数据（`uiStore` 除外），保证「后端是唯一真相源」。`editorStore` 的 `buffers` 是本地编辑缓冲区（DESIGN.md §3.1.4），只有显式保存才会调用 `fsService.writeFile` 同步到后端，编辑过程中不产生 IPC 调用。

---

## 七、Tauri Event 目录

| Event | Payload | 触发方 | 消费方 |
|-------|---------|--------|--------|
| `workspace:opened` | `WorkspaceHandle` 摘要 | 工作区打开完成（含远程建连+指纹校验后） | `App.tsx`，驱动布局从「工作区选择页」切到 IDE 布局 |
| `ssh:data` | `{ channelId, data: bytes }` | 后端读到远端输出 | `TerminalView` |
| `ssh:status` | `{ sessionId, status }` | 连接状态变化（含重连） | `connectionStore` |
| `ssh:host-key-prompt` | `{ requestId, host, fingerprint, changed: bool }` | TOFU / 指纹变化 | `HostKeyDialog` |
| `sftp:file-chunk` / `sftp:file-progress` / `sftp:file-complete` | 见 DESIGN.md §3.3.1 | 流式读取远程文件（Explorer 打开文件 与 SFTP 快捷工具共用） | `CodeEditor` |
| `sftp:transfer-progress` | `{ taskId, transferred, total }` | 上传/下载 | `TransferProgress` |
| `log:import-progress` | `{ jobId, bytesDone, bytesTotal }` | 日志导入 | `LogSearchPanel` |
| `log:live-result` | `{ queryId, line }` | ripgrep 实时搜索 | `ResultList` |
| `ai:chat-delta` | `{ streamId, delta }` | SSE 流式回复 | `ChatPanel` |
| `coding:agent-message` | `{ sessionId, message }` | Agent 回复/思考过程 | `CodingAgent` |
| `coding:file-change` | `{ sessionId, change: FileChange }` | 新增/更新变更 | `FileChangeCard` |
| `coding:tool-call-progress` | `{ sessionId, tool, startedAt, elapsedMs }` | 每次工具调用开始/耗时更新（DESIGN.md §3.8.7 性能诚实） | `ToolCallProgress` |
| `coding:command-confirm-request` | `{ requestId, sessionId, host?, command }` | `CommandGuard::RequiresConfirmation` | `CommandConfirmDialog` |
| `audit:new-entry` | `AuditLogEntry` | 任意敏感操作发生后 | `SecuritySettings`（若打开审计面板） |

事件名统一定义在后端 `events.rs` 常量与前端 `services/eventBus.ts` 的映射表中，禁止在业务代码里手写字符串字面量。

---

## 八、跨端类型同步策略

- 使用 [`tauri-specta`](https://github.com/oscartbeaumont/tauri-specta) 从 Rust 的 `#[derive(specta::Type)]` 结构体自动生成 `src-web/src/types/bindings.ts`，包括所有 Command 的入参/返回类型与 Event payload 类型。
- CI 中加一步 `cargo run --bin export-bindings`（或等价 build script）并 `git diff --exit-code` 校验生成结果是否已提交，防止前后端类型漂移。
- `AppError` 的 `kind` 字段用 TS 的联合类型消费，前端可以 `switch (err.kind)` 做分支处理（如 `HostKeyRejected` → 打开 `HostKeyDialog`）。

---

## 九、模块依赖关系（避免循环依赖）

```
commands/*  ──depends on──▶  domain modules (workspace/ssh/log/ai/coding/plugin/browser/connection/fsops)
domain modules  ──depends on──▶  db/repo, credential, error
workspace  ──depends on──▶  fsops, ssh::SshConnectionPool（打开工作区时按需建连）
coding     ──depends on (trait only)──▶  fsops::FileOps（不再直接依赖 ssh，也不自带 file_ops）
fsops      ──depends on (trait only)──▶  ssh::SshSession（RemoteFileOps 实现）
plugin     ──depends on (trait only)──▶  log::LogSearchEngine, ssh::SshSession（通过受控 API 代理）
```

`ssh`、`log`、`db`、`credential`、`fsops` 是最底层，禁止反向依赖 `workspace`/`coding`/`plugin`/`commands`；这样保证核心连接、文件读写与检索能力可以脱离 AI/插件功能独立编译测试。`workspace` 是新的编排层——它组合 `fsops` 和 `ssh` 但不被两者依赖，这条单向关系是 Explorer、SFTP、Coding Agent 能共享同一个 `WorkspaceHandle` 而不互相耦合的关键。

---

## 十、测试策略（对应 DESIGN.md §十-4）

| 层级 | 工具 | 覆盖重点 |
|------|------|----------|
| Rust 单元测试 | `cargo test` | `CommandGuard::evaluate`、`KnownHosts` 比对逻辑、FTS5 查询构造、Diff 生成/应用 |
| Rust 集成测试 | `cargo test --test integration` + 本地 sshd 容器 | SSH 连接池复用、SFTP 大文件分页 |
| 前端单元测试 | Vitest + Testing Library | store reducer、组件纯逻辑（如 `isTextViewable`） |
| 端到端 | `tauri-driver` + WebDriver | 关键路径：打开远程工作区（含指纹校验）→ 默认终端 Tab → Explorer 打开并编辑保存远程文件（含冲突场景）→ AI 编程助手 Apply/Reject |

CI 建议从 Phase 1 起就跑 `cargo test` + `npm run test`（Lint + 单测），端到端在 Phase 3 落地 Coding Agent 后补齐。
