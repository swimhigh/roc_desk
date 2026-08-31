//! `roc_desk.exe`（客户端）与 `roc_desk_agent.exe`（远程 Windows Agent）之间的共享协议
//! （AGENT_DESIGN.md §三）。这个 crate 不依赖 Tauri、不依赖 GUI/系统密钥链——纯数据
//! 定义 + 帧编解码，被 `agent/` 和 `src-tauri/` 同时依赖，协议改动由编译器保证两侧同步。

pub mod frame;

pub use frame::{read_frame, write_frame, Frame, FrameType, DATA_CHUNK_SIZE, FRAME_HEADER_LEN, MAX_FRAME_PAYLOAD_LEN};

use serde::{Deserialize, Serialize};

/// 协议版本号：客户端和 Agent 分开升级是必然场景（AGENT_DESIGN.md §九-4），这里选择
/// 最简单的策略——`Handshake` 双方交换版本号，不一致就直接报错提示升级，不做协议协商。
pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum Request {
    Handshake { token: String, protocol_version: u32, client_version: String },
    ListDir { path: String },
    Stat { path: String },
    /// 响应通过 `DataChunk`/`StreamEnd` 帧流式返回文件内容，Control 帧只带
    /// `FileMeta` 元信息（mtime/size），避免大文件走 JSON 造成 33% 体积膨胀。
    ReadFile { path: String },
    ReadFileBounded { path: String, max_bytes: u64 },
    /// 请求方发完这个 Control 帧后，紧接着在同一 stream_id 上发 `DataChunk` 帧
    /// （可以多个）传输文件内容，最后发 `StreamEnd`。
    WriteFile { path: String, expected_mtime: Option<i64> },
    Delete { path: String, is_dir: bool },
    Rename { from: String, to: String },
    CreateDir { path: String },
    /// 盘符列表：`C:\`、`D:\` ...（Windows 路径的"根"概念，对应 Explorer 顶层）。
    ListRoots,
    Exec { command: String, args: Vec<String>, cwd: String, timeout_secs: u32 },
    SearchContent { root: String, query: String, options: SearchOptions },
    SearchFileName { root: String, query: String },
    /// 交互式终端（AGENT_DESIGN.md §四.4 Phase 2）：这个请求打开的流是长期双向的——
    /// 响应 `Ok(Empty)` 之后，同一个 stream_id 上会持续收发 `DataChunk`（分别是
    /// 键盘输入/PTY 输出）和 `ShellResize`（Control 帧），直到某一端发 `StreamEnd`
    /// （客户端发＝用户关闭终端；Agent 发＝远端 shell 进程退出）。
    OpenShell { cols: u16, rows: u16, cwd: String },
    /// 只能发在一个已经 `OpenShell` 成功的 stream_id 上，不是独立请求。
    ShellResize { cols: u16, rows: u16 },
}

// `content = "data"`（邻接标签）而不是只给 `tag`（内部标签）：内部标签要求每个
// 变体的内容都能序列化成 JSON 对象，好把 "status" 字段插进去——`ResponseBody`
// 里 `Entries`/`Roots`/`SearchResults` 这几个变体的内容是数组（`Vec<...>`），
// 序列化成 JSON 数组而不是对象，用内部标签会在运行时序列化失败（`encode_json`
// 拿到 Err，调用方 `unwrap_or_default()` 把它悄悄吞成一个空 payload），
// 具体表现是客户端收到一个空字节的 Control 帧，JSON 解码报
// "EOF while parsing a value"——`list_dir`/`list_roots`/`search_*` 全部中招，
// 只有内容本身就是对象的 `Handshake`/`FileMeta`/`Written`/`ExecResult` 侥幸能用。
// 邻接标签把内容包进独立的 "data" 字段，不管内容是对象还是数组都能用。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", content = "data", rename_all = "snake_case")]
pub enum Response {
    Ok(ResponseBody),
    /// 与 `fsops::WriteOutcome::Conflict` 对齐：写入前发现远端 mtime 与调用方预期
    /// 的不一致，返回当前 mtime + 前几行预览，交给调用方决定是否强制覆盖。
    Conflict { current_mtime: i64, current_preview: String },
    Error { code: ErrorCode, message: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ResponseBody {
    Entries(Vec<FileEntry>),
    FileMeta { size: u64, mtime: i64 },
    Written { mtime: i64 },
    ExecResult { exit_code: Option<i32>, output: String },
    SearchResults(Vec<SearchFileResult>),
    Roots(Vec<String>),
    Handshake { server_version: String, hostname: String },
    Empty,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    NotFound,
    PermissionDenied,
    OutsideAllowedRoots,
    InvalidArgument,
    Internal,
    AuthFailed,
    ProtocolVersionMismatch,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: Option<u64>,
    pub modified: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SearchOptions {
    #[serde(default)]
    pub case_sensitive: bool,
    #[serde(default)]
    pub whole_word: bool,
    #[serde(default)]
    pub use_regex: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchMatch {
    pub line_number: usize,
    pub line_text: String,
    pub match_start: usize,
    pub match_end: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchFileResult {
    pub path: String,
    pub matches: Vec<SearchMatch>,
}

/// Control 帧 payload 编解码——固定用 JSON（DataChunk 帧才是裸字节，见 `frame` 模块）。
pub fn encode_json<T: Serialize>(value: &T) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(value)
}

pub fn decode_json<'a, T: Deserialize<'a>>(bytes: &'a [u8]) -> Result<T, serde_json::Error> {
    serde_json::from_slice(bytes)
}
