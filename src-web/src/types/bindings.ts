// 与 src-tauri 手写同步的类型定义。
//
// CODE_DESIGN.md §八 计划用 tauri-specta 从 Rust 端自动生成本文件，避免类型漂移；
// Phase 1 先手写以加快首个可运行版本的落地，接入 tauri-specta 是后续要做的事，
// 到时候这个文件会被生成结果替换（导出的类型名/形状保持不变即可）。

/** "转到定义/声明"命中的一个候选位置（src-tauri/src/symbols/mod.rs::SymbolLocation）。*/
export interface SymbolLocation {
  path: string;
  /** 1-based 行号，和 Monaco Range 的行号约定一致。 */
  line: number;
  kind: string;
}

export type WorkspaceKind = "local" | "remote";

export interface WorkspaceProfile {
  id: string;
  kind: WorkspaceKind;
  root_path: string;
  connection_id: string | null;
  display_name: string;
  last_opened_at: string | null;
  /** SFTP/Agent 双栏浏览器最后停留的两边目录，NULL 表示还没打开过。 */
  last_sftp_local_path: string | null;
  last_sftp_remote_path: string | null;
}

export interface FileEntry {
  name: string;
  path: string;
  is_dir: boolean;
  size: number | null;
  modified: number | null;
}

export interface FileContent {
  text: string;
  encoding: string;
  mtime: number;
  /** 文件总字节数（不是 text 的长度——truncated 为 true 时 text 只是截断预览）。 */
  total_size: number;
  /** 文件过大、text 只是截断预览时为 true；此时编辑器应转只读，禁止保存。 */
  truncated: boolean;
}

export type WriteOutcome =
  | { type: "Written"; mtime: number }
  | { type: "Conflict"; current_mtime: number; current_preview: string };

/** 对应后端 `fsops::binary_info::BinaryInfo`——打开 EXE/DLL/SO 等可执行文件时展示
 * 的基本信息 + 依赖库列表（2026-08-28 需求）。 */
export interface BinaryInfo {
  format: string;
  architecture: string;
  bitness: string;
  file_kind: string;
  entry_point: string | null;
  timestamp: string | null;
  subsystem: string | null;
  dependencies: string[];
  exports: string[];
  exports_truncated: boolean;
  total_exports: number;
  sections: BinarySection[];
}

export interface BinarySection {
  name: string;
  virtual_size: number;
  raw_size: number;
}

/** 对应后端 `fsops::jar_info::JarInfo`——打开 JAR 包时展示的基本信息（manifest/
 * Main-Class/Class-Path）+ 内部条目列表（2026-08-28 需求）。 */
export interface JarInfo {
  total_entries: number;
  class_count: number;
  main_class: string | null;
  class_path: string[];
  manifest: ManifestAttribute[];
  entries: JarEntryInfo[];
  entries_truncated: boolean;
}

export interface ManifestAttribute {
  key: string;
  value: string;
}

export interface JarEntryInfo {
  path: string;
  is_dir: boolean;
  size: number;
  compressed_size: number;
}

export type AppErrorKind =
  | "Connection"
  | "Auth"
  | "HostKeyRejected"
  | "PermissionDenied"
  | "NotFound"
  | "Database"
  | "Conflict"
  | "Internal";

export interface AppError {
  kind: AppErrorKind;
  message: string;
}

export function isAppError(e: unknown): e is AppError {
  return typeof e === "object" && e !== null && "kind" in e && "message" in e;
}

export type AuthMethod = "password" | "key" | "agent";
/** "agent" 是 AGENT_DESIGN.md 的远程 Windows Agent 协议——和上面 AuthMethod 里的
 * "agent"（SSH Agent 认证）是两个不相关的概念，只是恰好同名，注意区分。 */
export type Protocol = "ssh" | "rdp" | "agent";

/** RDP 专属的少量额外字段，存在 ConnectionProfile.options 里（JSON，见后端
 * connection/profile.rs 注释）；SSH 连接的 options 一般是 null。*/
export interface RdpOptions {
  domain?: string;
  width?: number;
  height?: number;
  color_depth?: number;
}

export interface ConnectionProfile {
  id: string;
  name: string;
  host: string;
  port: number;
  username: string;
  auth_method: AuthMethod;
  credential_ref: string | null;
  group_id: string | null;
  tags: string[];
  jump_host_id: string | null;
  protocol: Protocol;
  options: RdpOptions | null;
  last_connected_at: string | null;
  created_at: string;
}

export interface ConnectionProfileInput {
  name: string;
  host: string;
  port: number;
  username: string;
  auth_method: AuthMethod;
  secret: string | null;
  group_id: string | null;
  tags: string[];
  jump_host_id: string | null;
  protocol: Protocol;
  options: RdpOptions | null;
}

/** 会话树的文件夹（远程工具模式，DESIGN.md §3.9）。*/
export interface ConnectionGroup {
  id: string;
  name: string;
  parent_id: string | null;
}

export interface ConnectionGroupInput {
  name: string;
  parent_id: string | null;
}

/** 远程主机资源使用率原始采样——只有累计计数器，CPU%/网速由前端拿相邻两次
 * 采样自己算差（后端 ssh/monitor.rs 顶部注释解释了为什么不在后端做）。*/
export interface HostStats {
  hostname: string;
  uptime_seconds: number;
  cpu_total: number;
  cpu_idle: number;
  mem_total_kb: number;
  mem_available_kb: number;
  net_rx_bytes: number;
  net_tx_bytes: number;
  disks: DiskUsage[];
  sampled_at_ms: number;
}

export interface DiskUsage {
  mount: string;
  total_kb: number;
  used_kb: number;
  used_percent: number;
}

export interface HostKeyPromptEvent {
  requestId: string;
  host: string;
  port: number;
  fingerprint: string;
  changed: boolean;
  oldFingerprint: string | null;
}

/** Agent TLS 证书指纹 TOFU 弹窗（AGENT_DESIGN.md §3.1），和上面的 SSH 主机指纹
 * 弹窗结构几乎一样，多一个 connectionId 字段——指纹按连接档案而不是 host/port
 * 存（见后端 `agent_known_hosts` 表的注释）。 */
export interface AgentCertPromptEvent {
  requestId: string;
  connectionId: string;
  host: string;
  port: number;
  fingerprint: string;
  changed: boolean;
  oldFingerprint: string | null;
}

export interface SshDataEvent {
  channelId: string;
  data: number[]; // 字节数组，前端转 Uint8Array 后交给 xterm.js
}

export interface SshStatusEvent {
  channelId: string;
  status: "connected" | "connecting" | "disconnected" | "error";
}

export interface LogQuery {
  query: string;
  limit?: number | null;
}

export interface LogSearchResult {
  file_path: string;
  line_number: number;
  timestamp: string | null;
  log_level: string | null;
  host_name: string;
  snippet: string;
}

export interface LiveSearchResult {
  file_path: string;
  line_number: number;
  timestamp: string | null;
  log_level: string | null;
  line: string;
}

export interface IndexStats {
  row_count: number;
  job_count: number;
}

export interface LogImportFailure {
  path: string;
  error: string;
}

export interface LogImportOutcome {
  lines_imported: number;
  files_imported: number;
  failed: LogImportFailure[];
}

export interface LogImportProgressEvent {
  requestId: string;
  path: string;
  done: number;
  total: number;
}

/** 对齐 Codex `config.toml` 的 `model_reasoning_effort`；`null`/未设置表示不传
 * 这个参数，交给服务端默认值。 */
export type ReasoningEffort = "minimal" | "low" | "medium" | "high";

export interface AiProvider {
  id: string;
  name: string;
  api_base: string;
  api_key_ref: string | null;
  model: string;
  is_local: boolean;
  wire_api: string;
  reasoning_effort: ReasoningEffort | string | null;
  /** 这个 Provider 实际能接受的上下文窗口（估算 token 数），`null` 表示不填、
   * 用 AI 编程助手的保守全局默认值（60_000）。填了之后编程助手的自动摘要/裁剪
   * 会按这个值来，不会把大窗口 Provider 也按小窗口的阈值频繁压缩上下文
   * （2026-09 需求）。 */
  context_window_tokens: number | null;
  created_at: string;
}

export interface AiProviderInput {
  name: string;
  api_base: string;
  api_key: string | null;
  model: string;
  is_local: boolean;
  wire_api: string;
  reasoning_effort: string | null;
  context_window_tokens: number | null;
}

export type ChatRole = "system" | "user" | "assistant";

export interface ChatMessage {
  role: ChatRole;
  content: string;
}

export interface AiChatChunkEvent {
  requestId: string;
  delta: string;
}

export interface AiChatDoneEvent {
  requestId: string;
}

export interface AiChatErrorEvent {
  requestId: string;
  message: string;
}

export type CodingMode = "plan" | "build";

export type CodingTarget =
  | { kind: "Local" }
  | { kind: "Remote"; connection_id: string; host_label: string }
  | { kind: "Agent"; connection_id: string; host_label: string };

export type ChangeStatus = "pending" | "applied" | "rejected" | "undone";

export type ChatAttachment =
  | { kind: "image"; name: string; mime: string; data_base64: string }
  | { kind: "file"; name: string; content: string }
  | { kind: "pdf"; name: string; data_base64: string };

export interface DiffLine {
  sign: "+" | "-" | " ";
  content: string;
}

export interface FileChange {
  id: string;
  path: string;
  old_content: string;
  new_content: string;
  diff: DiffLine[];
  status: ChangeStatus;
  /** 产生这条变更的用户消息轮次 id——同一轮里的变更共享同一个值，前端据此
   * 分组做"这一轮全部应用/全部拒绝/整体撤销"。 */
  turn_id: string;
}

/** AI 写盘（Accept/Undo/Redo/整轮撤销）落地后的同步信息——如果这个路径当前
 * 正在编辑器里开着，前端要用它刷新对应的 buffer，否则打开的 Tab 会和磁盘内容
 * 脱节（见 `editorStore.syncExternalWrite`）。 */
export interface FileSyncInfo {
  change_id: string;
  path: string;
  content: string;
  mtime: number;
}

export type TodoStatus = "pending" | "in_progress" | "completed";

export interface TodoItem {
  id: string;
  content: string;
  status: TodoStatus;
}

export interface CodingSessionInfo {
  id: string;
  provider_id: string;
  mode: CodingMode;
  target: CodingTarget;
  auto_allow_readonly: boolean;
  git_repo: boolean;
  auto_git_commit: boolean;
  /** "完全授权模式"：开启后 AI 提出的文件改动不再生成 Diff 等 Accept，直接落盘
   * （用户反馈"一次改 20 多个文件还要逐个确认太繁琐"）。会话级开关。 */
  full_auto: boolean;
  /** 文件改动是否自动应用，默认 true——2026-09 用户要求默认不用逐个点"应用"，
   * 只需要在改错时点"撤销"。关掉退回旧的"每条手动 Accept"行为。 */
  auto_apply_changes: boolean;
  changes: FileChange[];
  todos: TodoItem[];
  project_memory_loaded: string[];
}

export type PermissionDecision = "allow" | "ask" | "deny";

export interface PermissionRule {
  id: string;
  tool: string;
  pattern: string;
  decision: PermissionDecision;
  enabled: boolean;
  created_at: string;
}

export interface PermissionRuleInput {
  tool: string;
  pattern: string;
  decision: PermissionDecision;
}

export type McpTransportKind = "stdio" | "http";

export interface McpServer {
  id: string;
  name: string;
  transport: McpTransportKind;
  command: string | null;
  args: string[];
  env: Record<string, string>;
  url: string | null;
  headers: Record<string, string>;
  auth_token_ref: string | null;
  enabled: boolean;
  created_at: string;
}

export interface McpServerInput {
  name: string;
  transport: McpTransportKind;
  command: string | null;
  args: string[];
  env: Record<string, string>;
  url: string | null;
  headers: Record<string, string>;
  auth_token: string | null;
  enabled: boolean;
}

export interface SkillMeta {
  name: string;
  description: string;
  dir: string;
}

export interface CodingTodoUpdateEvent {
  sessionId: string;
  todos: TodoItem[];
}

export interface CodingQuestionRequestEvent {
  sessionId: string;
  requestId: string;
  question: string;
  options: string[];
}

export interface CodingToolCallEvent {
  sessionId: string;
  tool: string;
  /** 这次调用在操作什么（文件路径/搜索词等），不是所有工具都有——2026-08-18
   * 真实复现"看起来在循环"的问题时，只有工具名完全看不出是不是在反复处理
   * 同一个东西，加上这个字段才能一眼确认是真循环还是正常地一个个探索。 */
  detail?: string | null;
  /** 只有 `coding:tool-call-end` 才带——这次调用实际执行完拿到的结果文本
   * （`run_command` 是标准输出/错误合并、`read_file`/`search_files` 之类是它们
   * 各自的返回内容），用户点开时间线里已完成的这一行时展示出来。 */
  output?: string | null;
}

/** 模型在同一条消息里，工具调用之外顺带写的说明性文字（2026-08-18 需求："编程
 * 助手的思考过程没有展示出来"）——之前直接丢弃，现在广播出来在时间线里展示。 */
export interface CodingAssistantNoteEvent {
  sessionId: string;
  text: string;
  kind?: "model" | "status";
}

/** 每次向模型发起请求后拿到的 token 消耗量（chat/completions 的 `usage` 或
 * Responses API 的 `usage`，两种协议统一成同一个形状再广播），前端渲染成时间线
 * 里的一条小字提示。 */
export interface CodingTokenUsageEvent {
  sessionId: string;
  promptTokens: number;
  completionTokens: number;
  totalTokens: number;
}

export interface CodingHistorySummary {
  id: string;
  title: string;
  provider_id: string;
  provider_label: string;
  model: string;
  mode: string;
  created_at: string;
  updated_at: string;
}

export interface CodingHistoryDetail extends CodingHistorySummary {
  workspace_id: string;
  provider_id: string;
  timeline: unknown;
  changes: unknown;
}

export interface CodingFileChangeEvent {
  sessionId: string;
  change: FileChange;
  /** "完全授权模式"下这条改动已经直接落盘时才有——前端据此刷新对应路径可能
   * 已经打开的编辑器 buffer，否则磁盘内容变了、编辑器里显示的还是旧内容。 */
  sync?: FileSyncInfo | null;
}

export interface CodingCommandBlockedEvent {
  sessionId: string;
  command: string;
}

export interface CodingCommandConfirmRequestEvent {
  sessionId: string;
  requestId: string;
  command: string;
  host: string | null;
  /** "mcp" 表示这是一次 MCP 工具调用确认，不是本地/远程 Shell 命令——弹窗文案
   * 据此区分（`CommandConfirmDialog.tsx`）。旧事件没有这个字段时按 "command" 处理。 */
  kind?: "command" | "mcp";
  /** 仅 kind === "mcp" 时存在：`"<server>:<tool>"`，权限规则引擎按这个字符串
   * 做通配匹配（不是展示用的 `command` 文本，那个还带着调用参数）。 */
  matchKey?: string;
}

export interface CodingGitCommitResultEvent {
  sessionId: string;
  path: string;
  output: string;
}

/** 用户点了文件改动卡片的"应用/拒绝"、这一轮提议的改动全部处理完之后，后端
 * 自动帮用户把对话续上——这一对事件通知前端"这个后台发起的续跑轮次开始/
 * 结束了"，让 UI 表现得和手动发消息完全一致（时间线气泡、输入框禁用状态、
 * 历史落盘），不需要用户手动再发一条消息才能感知到 AI 已经继续（2026-09
 * 用户反馈：点了应用后 AI 什么反应都没有，必须等下一轮会话）。 */
export interface CodingAutoContinueStartEvent {
  sessionId: string;
  note: string;
}

export interface CodingAutoContinueDoneEvent {
  sessionId: string;
  reply: string | null;
  error: string | null;
}

export interface SftpTransferProgressEvent {
  requestId: string;
  path: string;
  bytes?: number;
  totalBytes?: number;
}

/** 传输历史一条记录（`transfer_log_list`），SFTP/Agent 双栏浏览器共用同一张表。
 * 字段名和 Rust 的 `TransferLogEntry` 保持原样的 snake_case——这个结构体没有
 * `#[serde(rename_all = "camelCase")]`，序列化出来的 JSON key 就是 Rust 字段名
 * 本身（和 `FileEntry` 的 `is_dir` 是同一个既有约定）。*/
export interface TransferLogEntry {
  id: string;
  protocol: "sftp" | "agent";
  direction: "upload" | "download";
  profile_id: string | null;
  profile_name: string;
  local_path: string;
  remote_path: string;
  is_dir: boolean;
  file_count: number;
  status: "completed" | "cancelled" | "failed";
  error_message: string | null;
  started_at: string;
  finished_at: string;
}

export interface BrowserHistoryEntry {
  id: string;
  url: string;
  title: string | null;
  visited_at: string;
}

export type SearchMode = "content" | "file_name";

export interface SearchOptions {
  case_sensitive: boolean;
  whole_word: boolean;
  use_regex: boolean;
}

export interface SearchMatch {
  line_number: number;
  line_text: string;
  /** 字符下标（不是字节下标），可直接配合 Array.from(line) 做高亮切片。 */
  match_start: number;
  match_end: number;
}

export interface SearchFileResult {
  path: string;
  matches: SearchMatch[];
}

/** `fs_search_stream` 命令本身不返回结果，结果通过下面这三个事件流式推送
 * （2026-08-18 需求："能否一个一个目录搜，搜到一部分先展示一部分"）。 */
export interface SearchFileResultEvent {
  requestId: string;
  file: SearchFileResult;
}

export interface SearchDoneEvent {
  requestId: string;
  truncated: boolean;
}

export interface SearchErrorEvent {
  requestId: string;
  message: string;
}

export interface ReplaceSummary {
  files_changed: number;
  occurrences_replaced: number;
}

// ---------------------------------------------------------------------------
// SQL 桌面模块（docs/SQL_DESKTOP_PLAN.md，src-tauri/src/sql/model.rs）
// ---------------------------------------------------------------------------

export type DbKind = "mysql" | "tdsql" | "postgres" | "opengauss" | "sql_server" | "oracle";

export interface DataSourceProfile {
  id: string;
  name: string;
  db_kind: DbKind;
  host: string;
  port: number | null;
  database_name: string | null;
  default_schema: string | null;
  username: string | null;
  credential_ref: string | null;
  environment: string;
  group_name: string | null;
  readonly: boolean;
  ssl_required: boolean;
  created_at: string;
  updated_at: string;
  last_used_at: string | null;
}

/** `password` 传空字符串表示"不修改密码"（对齐 AiProviderInput 的既有约定）。*/
export interface DataSourceInput {
  name: string;
  db_kind: DbKind;
  host: string;
  port: number | null;
  database_name: string | null;
  default_schema: string | null;
  username: string | null;
  password: string | null;
  environment: string;
  group_name: string | null;
  readonly: boolean;
  ssl_required: boolean;
}

export interface DbInfo {
  version: string;
  latency_ms: number;
}

export type ObjectKind = "table" | "view" | "materialized_view" | "function" | "procedure";

export interface ObjectRef {
  schema: string;
  name: string;
  kind: ObjectKind;
}

export interface ObjectPage {
  objects: ObjectRef[];
}

export interface ColumnDef {
  name: string;
  data_type: string;
  nullable: boolean;
  default_value: string | null;
  is_primary_key: boolean;
  comment: string | null;
}

export interface IndexDef {
  name: string;
  definition: string;
}

export interface ObjectDefinition {
  object: ObjectRef;
  columns: ColumnDef[];
  indexes: IndexDef[];
  ddl: string | null;
  comment: string | null;
}

/** 结果集单元格——统一转成字符串展示，`is_binary` 时 `text` 是十六进制串
 * （参考 rainfrog 的展示方式，见方案 §4.2.1）。 */
export interface Cell {
  text: string;
  is_null: boolean;
  is_binary: boolean;
}

export interface ColumnInfo {
  name: string;
  type_name: string;
}

export interface ExecuteResult {
  columns: ColumnInfo[];
  rows: Cell[][];
  rows_affected: number | null;
  truncated: boolean;
  duration_ms: number;
}

export interface PendingWrite {
  pending_id: string;
  rows_affected: number | null;
  preview: ExecuteResult;
}

export type QueryStatus = "running" | "finished" | "error" | "cancelled";

export interface QueryPoll {
  status: QueryStatus;
  result: ExecuteResult | null;
  error: string | null;
}

/** `sql_execute` 的返回形状（src-tauri/src/commands/sql.rs::ExecuteOutcome）。*/
export type ExecuteOutcome =
  | { kind: "Started"; query_id: string }
  | { kind: "NeedsConfirmation" }
  | ({ kind: "PendingWrite" } & PendingWrite);

export interface QueryHistoryEntry {
  id: string;
  data_source_id: string;
  title: string | null;
  sql_text: string;
  status: string;
  duration_ms: number | null;
  row_count: number | null;
  error_message: string | null;
  created_at: string;
}

export type ResultViewMode = "table" | "text";

/** 标签页内容的唯一真相是 `file_path` 指向的磁盘文件
 * （docs/SQL_DESKTOP_PLAN.md §4.4），这里只是元数据。 */
export interface WorkspaceTab {
  id: string;
  data_source_id: string;
  title: string;
  file_path: string;
  result_view_mode: ResultViewMode;
  cursor_json: string | null;
  sort_order: number;
  updated_at: string;
}

// ---------------------------------------------------------------------------
// 表数据编辑 / 导出导入（src-tauri/src/sql/data_editor.rs、sql/transfer.rs）
// ---------------------------------------------------------------------------

export interface CellInput {
  is_null: boolean;
  text: string;
}

export interface NamedCell {
  name: string;
  value: CellInput;
}

export type AlterOp =
  | { op: "add_column"; name: string; data_type: string; nullable: boolean }
  | { op: "drop_column"; name: string }
  | { op: "rename_column"; old_name: string; new_name: string };

export type TransferFormat = "csv" | "json";

export interface TransferProgress {
  rows_done: number;
  done: boolean;
  cancelled: boolean;
  error: string | null;
}

// --- SQL Agent（2026-09：AI 工具要和"工作区"编程助手一样是真正的多轮 Agent，
// 见 sql::agent 模块文档）。会话信息/历史类型结构上和 CodingSessionInfo/
// CodingHistorySummary 对应，事件 payload 形状直接复用 Coding*Event 系列
// （字段完全一样，只是从 "coding:" 换成 "sqlagent:" 事件前缀）。

export interface SqlAgentSessionInfo {
  id: string;
  provider_id: string;
  todos: TodoItem[];
}

export interface SqlAgentHistorySummary {
  id: string;
  title: string;
  provider_id: string;
  provider_label: string;
  model: string;
  created_at: string;
  updated_at: string;
}

export interface SqlAgentHistoryDetail {
  id: string;
  title: string;
  provider_id: string;
  provider_label: string;
  model: string;
  created_at: string;
  updated_at: string;
  data_source_id: string;
  timeline: unknown;
}

export interface SqlAgentConfirmRequestEvent {
  sessionId: string;
  requestId: string;
  sql: string;
}

// --- HTTP 桌面（docs/HTTP_DESKTOP_PLAN.md）。没有独立的"集合"/"环境" SQLite
// 表——这些类型描述的是落在工作区目录 `.rock_desk/http/` 下的 YAML 文件内容
// （src-tauri/src/http_desk/model.rs），只有 HttpRequestHistory*/HttpWorkspaceTab
// 这几个是真正的数据库行。

export interface KeyValueItem {
  key: string;
  value: string;
  enabled: boolean;
}

export type ApiKeyLocation = "header" | "query";

export type AuthConfig =
  | { type: "none" }
  | { type: "bearer"; token: string }
  | { type: "basic"; username: string; password: string }
  | { type: "api_key"; key: string; value: string; add_to: ApiKeyLocation };

export type RequestBody =
  | { type: "none" }
  | { type: "json"; content: string }
  | { type: "raw"; content: string; content_type: string }
  | { type: "form_url_encoded"; items: KeyValueItem[] }
  | { type: "form_data"; items: KeyValueItem[] };

export interface RequestDef {
  id: string;
  name: string;
  method: string;
  url: string;
  params: KeyValueItem[];
  headers: KeyValueItem[];
  auth: AuthConfig;
  body: RequestBody;
}

export interface RequestSummary {
  id: string;
  name: string;
  method: string;
  folder: string[];
}

export interface EnvVar {
  key: string;
  value: string;
  secret: boolean;
  enabled: boolean;
}

export interface EnvironmentDef {
  id: string;
  name: string;
  variables: EnvVar[];
}

export interface HttpCollectionMeta {
  name: string;
  description: string | null;
  auth: AuthConfig;
  variables: EnvVar[];
}

export interface HttpCollectionSummary {
  slug: string;
  name: string;
  description: string | null;
  request_count: number;
}

export interface HttpExecuteResult {
  status: number;
  status_text: string;
  headers: [string, string][];
  body: string;
  body_is_text: boolean;
  body_base64: string | null;
  duration_ms: number;
  size_bytes: number;
  resolved_url: string;
  truncated: boolean;
}

export interface HttpRequestHistoryEntry {
  id: string;
  workspace_id: string;
  collection_slug: string;
  request_id: string | null;
  environment_id: string | null;
  method: string;
  url: string;
  status_code: number | null;
  duration_ms: number | null;
  response_size_bytes: number | null;
  error_message: string | null;
  created_at: string;
}

export interface HttpRequestHistoryDetail {
  id: string;
  request_snapshot: string;
  response_snapshot: string | null;
}

export interface HttpWorkspaceTab {
  id: string;
  workspace_id: string;
  collection_slug: string;
  request_id: string;
  title: string;
  sort_order: number;
  updated_at: string;
}
