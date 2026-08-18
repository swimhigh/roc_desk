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

export interface CodingSessionInfo {
  id: string;
  mode: CodingMode;
  target: CodingTarget;
  auto_allow_readonly: boolean;
  git_repo: boolean;
  auto_git_commit: boolean;
  changes: FileChange[];
}

export interface CodingToolCallEvent {
  sessionId: string;
  tool: string;
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

export interface SearchSummary {
  files: SearchFileResult[];
  truncated: boolean;
}

export interface ReplaceSummary {
  files_changed: number;
  occurrences_replaced: number;
}
