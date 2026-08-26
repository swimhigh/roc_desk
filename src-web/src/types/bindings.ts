// 与 src-tauri 手写同步的类型定义。
//
// CODE_DESIGN.md §八 计划用 tauri-specta 从 Rust 端自动生成本文件，避免类型漂移；
// Phase 1 先手写以加快首个可运行版本的落地，接入 tauri-specta 是后续要做的事，
// 到时候这个文件会被生成结果替换（导出的类型名/形状保持不变即可）。

export type WorkspaceKind = "local" | "remote";

export interface WorkspaceProfile {
  id: string;
  kind: WorkspaceKind;
  root_path: string;
  connection_id: string | null;
  display_name: string;
  last_opened_at: string | null;
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
}

export type WriteOutcome =
  | { type: "Written"; mtime: number }
  | { type: "Conflict"; current_mtime: number; current_preview: string };

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
export type Protocol = "ssh" | "rdp";

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

export interface AiProvider {
  id: string;
  name: string;
  api_base: string;
  api_key_ref: string | null;
  model: string;
  is_local: boolean;
  created_at: string;
}

export interface AiProviderInput {
  name: string;
  api_base: string;
  api_key: string | null;
  model: string;
  is_local: boolean;
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
  | { kind: "Remote"; connection_id: string; host_label: string };

export type ChangeStatus = "pending" | "applied" | "rejected" | "undone";

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
}

/** 模型在同一条消息里，工具调用之外顺带写的说明性文字（2026-08-18 需求："编程
 * 助手的思考过程没有展示出来"）——之前直接丢弃，现在广播出来在时间线里展示。 */
export interface CodingAssistantNoteEvent {
  sessionId: string;
  text: string;
  kind?: "model" | "status";
}

export interface CodingHistorySummary {
  id: string;
  title: string;
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

export interface SftpTransferProgressEvent {
  requestId: string;
  path: string;
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
