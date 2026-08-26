# roc_desk 远程 Agent（roc_desk_agent.exe）设计方案

> 本文档设计一个新增的 Windows 可执行程序 `roc_desk_agent.exe`：部署在**被管理的远程 Windows 服务器**上常驻运行，`roc_desk.exe`（客户端）通过自定义协议连接它，把远程 Windows 主机的文件系统、命令执行、终端会话接入现有的"工作区"模型——效果上等价于给 Windows 目标机器配了一个专用的、比 SSH+SFTP 更贴合 Windows 语义的远程后端。
>
> 定位类比：`roc_desk_agent.exe` 之于 Windows 远程工作区，相当于 `russh`/`russh-sftp` 之于当前的 SSH/SFTP 远程工作区——是**新增的第三种连接协议**，不替换、不改动现有 SSH（DESIGN.md §3.2）/RDP（`src-tauri/src/rdp/mod.rs`）能力。三者在 `ConnectionProfile.protocol` 上并列（`Ssh` / `Rdp` / 新增 `Agent`），共用同一张连接档案表、同一套分组/标签管理。
>
> 本文档只是方案设计，尚未实现；落地后的实际状态请按 REQUIREMENTS.md 的记录方式补充"已实现/部分实现/未实现"。

---

## 一、为什么需要单独做一个 Agent，而不是复用 SSH+SFTP

当前的远程工作区能力（DESIGN.md §3.1.4、§3.2、§3.3）完全建立在 SSH 协议之上：`russh` 建连、`russh-sftp` 做文件读写、shell 命令靠 `SshSession::exec`/`request_shell` 一发了之。这套方案面向 **Linux** 目标机器几乎没有摩擦（OpenSSH 是标配），但直接套到 **Windows** 目标机器上会遇到几个结构性的别扭之处，这是新增 Agent 的直接动机：

| 问题 | SSH+SFTP 方案下的表现 | Agent 方案怎么解决 |
|------|----------------------|---------------------|
| **OpenSSH Server 不是 Windows 标配** | Windows Server 2012R2/2016 及更早版本、大量被安全基线锁死的生产机器根本没装 `sshd`；装 OpenSSH for Windows 涉及改动目标机器的系统组件，运维往往不愿意 / 没有权限批准 | Agent 是一个独立小体积 exe，不依赖系统组件，管理员本地拷贝一个文件、起一个进程（或注册一个服务）即可，不改动系统配置 |
| **Shell 语义错位** | `request_shell` 起的是 `cmd.exe`/PowerShell，但 §3.2 的 `cd <dir>`"代打命令"、`log/remote.rs::shell_quote` 的单引号转义规则都是 POSIX shell 语义，直接套用在 Windows 目标上是错的（真连上 Windows sshd 时 `cd '/etc/nginx'` 这类转义对 PowerShell 无意义甚至语法错误） | Agent 协议层的命令执行原语原生按 Windows 语义设计（`cmd /c`/PowerShell 参数数组传递，不走"拼一行 shell 字符串再转义"这条容易出错的路） |
| **路径分隔符 / 盘符** | SFTP 协议内部路径约定用 `/`，`fsops::remote.rs` 里 `format!("{}/{}", ...)` 拼路径的写法处处假设 POSIX 路径；Windows 的 `C:\Users\...` 和盘符概念要在这层强行适配 | Agent 协议原生传 Windows 路径（含盘符），列目录直接返回盘符列表作为"根" |
| **远程内容搜索慢**（REQUIREMENTS.md §3.11 已记录的已知局限）| `fsops::search_stream` 对远程文件是"逐文件一次 SFTP round trip"，工作区文件数一多明显慢于本地 ripgrep | Agent 把搜索逻辑放在**目标机器本地执行**（复用同一套 `search_stream` 算法，但输入是 Agent 进程本机的 `std::fs`），只有匹配到的少量结果经网络传回，从"N 次网络请求"变成"1 次网络请求"，这是架构上的根本性提速，不是调参能解决的 |
| **文件变更感知** | 无——Explorer 树对远程目录的变化完全无感知，只能手动刷新 | Agent 常驻进程可以用 `notify` crate 包装 Windows 原生的 `ReadDirectoryChangesW` 做文件监听，未来可推送变更事件到客户端（v1 不做，见 §九 阶段划分） |
| **凭据模型** | 需要用户名+密码/私钥，貌似"标准"但对一台专用被管理服务器来说是过度设计——通常只有 1~2 个运维人员需要连接 | Agent 用一次性生成的配对令牌（pairing token），语义更贴近"给这台机器发一张邀请函"，不需要管理 Windows 系统账户密码 |

**不做的事**（明确边界，避免范围蔓延）：Agent 不是要取代 SSH——Linux 目标机器、以及已经装好 OpenSSH 且能正常工作的 Windows 机器，继续走现有 SSH+SFTP 路径，两者长期并存。Agent 也不做远程桌面（RDP 模块已经解决这个问题）。

---

## 二、总体架构

```
┌───────────────────────────────┐          ┌──────────────────────────────────┐
│   roc_desk.exe（客户端，任意机器）│          │  roc_desk_agent.exe（被管理的远程   │
│                                 │          │  Windows 服务器，常驻运行）          │
│  ┌───────────┐  ┌─────────────┐│  TLS/TCP │  ┌───────────┐  ┌───────────────┐│
│  │ Explorer  │  │ AI 编程助手  ││ 自定义协议 │  │ RPC 分发器 │  │ Windows 原生 API││
│  │ SFTP 快捷  │  │ 全局搜索    ││<────────>│  │           │  │ std::fs        ││
│  │ 工具       │  │             ││  单条长连  │  │           │  │ ConPTY         ││
│  └─────┬─────┘  └──────┬──────┘│  接多路复用│  │           │  │ ReadDirectory-  ││
│        │               │       │          │  │           │  │  ChangesW      ││
│        └───────┬───────┘       │          │  └─────┬─────┘  └───────────────┘│
│         fsops::FileOps trait   │          │        │                          │
│                │               │          │        │                          │
│      ┌─────────┴─────────┐     │          │        │                          │
│      │  AgentFileOps      │◄────┼──────────┼────────┘                          │
│      │ (agent/ 模块)      │     │          │                                    │
│      └────────────────────┘     │          └──────────────────────────────────┘
└───────────────────────────────┘
```

关键设计决定：**Agent 的所有能力最终都要落到已有的 `fsops::FileOps` trait（`f:\code\wuyou\roc_desk\src-tauri\src\fsops\mod.rs`）上**，用一个新的 `AgentFileOps` 实现补齐第三种后端（本地 `LocalFileOps` / SSH `RemoteFileOps` / Agent `AgentFileOps`）。这是这份设计里最重要的复用杠杆：Explorer、SFTP 自由浏览、AI 编程助手工具集、全局搜索/替换——这些模块全部只依赖 `&dyn FileOps`，一行都不用改，天然获得对 Windows Agent 工作区的支持。

### 2.1 新增的代码组织

```
roc_desk/                          ← 仓库根目录（Cargo workspace 化）
├── Cargo.toml                     ← 新增，声明 workspace members
├── src-tauri/                     ← 现有客户端 crate（roc_desk_lib / roc_desk 二进制），改为 workspace member
├── agent/                         ← 新增 crate，产出 roc_desk_agent.exe
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs                ← CLI 入口（run / install-service / pair / genkey 子命令，见 §六）
│       ├── server.rs              ← TCP 监听 + TLS + 每连接分发循环
│       ├── handlers/
│       │   ├── fs.rs              ← list_dir/read_file/write_file/delete/rename/copy/create_dir
│       │   ├── exec.rs            ← 一次性命令执行（对应 SshSession::exec）
│       │   ├── shell.rs           ← ConPTY 交互式终端（Phase 2，见 §九）
│       │   ├── search.rs          ← 本机执行的内容/文件名搜索
│       │   └── watch.rs           ← 文件变更监听（Phase 3）
│       ├── auth.rs                ← 配对令牌校验
│       └── config.rs              ← agent.toml 配置加载
└── protocol/                       ← 新增 crate，客户端与 Agent 共享的协议类型定义
    ├── Cargo.toml
    └── src/lib.rs                  ← 请求/响应消息的 serde 结构体、帧编解码（见 §三）
```

`protocol/` 独立成一个不依赖 Tauri、不依赖 `tokio-native-tls`/GUI 的纯数据 crate，被 `agent/` 和 `src-tauri/` 同时依赖——这样协议改动（加字段、加方法）由编译器保证客户端和 Agent 两侧同步更新，不会出现"协议文档和两边实现各自漂移"的问题（`mcp/` 模块手写 JSON-RPC 客户端时就是因为没有共享类型定义，前后端对齐全靠人工核对）。

`src-tauri/` 侧新增镜像 `ssh/` 模块结构的 `agent/` 子模块：

```
src-tauri/src/agent/
├── mod.rs
├── session.rs      ← 单条 TLS 连接，多路复用多个逻辑 stream（对应 SshSession）
├── pool.rs          ← AgentConnectionPool，同一 Agent 目标复用一条物理连接（对应 SshConnectionPool）
├── handshake.rs     ← TLS 握手 + 证书指纹 TOFU 校验（对应 known_hosts.rs）
└── pairing.rs        ← 配对令牌的输入/存储流程
```

以及 `src-tauri/src/fsops/agent.rs`（新增第三个 `FileOps` 实现，和 `local.rs`/`remote.rs` 同级）。

---

## 三、通信协议设计

### 3.1 传输层：TLS over TCP

- Agent 启动时若本地没有证书，自动用 `rcgen` 生成一份自签名证书 + 私钥，落盘到 Agent 的数据目录（`cert.pem`/`key.pem`），此后固定复用——证书本身不需要被任何公共 CA 信任，它只是承载"这条连接是加密的，且我们要校验的是这个具体证书的指纹"这件事，模型完全类比现有 SSH 的 `known_hosts` TOFU（DESIGN.md §3.2.1）。
- 服务端用 `tokio-rustls` 监听；客户端连接时**不做标准 CA 链校验**（自签名证书天然过不了），而是在 `rustls::ClientConfig` 里装一个自定义 `ServerCertVerifier`，校验逻辑与 `SshHandler::check_server_key` 完全对称：
  - 首次连接（Unknown）：计算证书指纹（SHA-256），弹窗展示给用户确认，确认后写入本地"Agent 已知主机"表（新增 SQLite 表 `agent_known_hosts { connection_id, fingerprint, trusted_at }`，与 `known_hosts.rs` 现有表分开，因为这是完全不同的信任链条，不应该和 SSH 主机指纹混在一张表里）。
  - 再次连接：指纹匹配则放行；指纹变化则按 DESIGN.md §3.2.1 同等级别红色告警拒绝连接（可能是重装了 Agent，也可能是中间人）。
  - **不做**：证书有效期校验、吊销检查——自签名证书生态里这些机制意义不大，指纹比对已经是这套模型里唯一有意义的信任锚点，做多余的校验只会增加"过期了但其实是我自己重装的"这类误报。

### 3.2 应用层帧格式

单条 TLS 连接承载多个逻辑"流"（文件操作请求/响应、一次性命令执行、交互式终端字节流、未来的文件监听推送事件），复用 SSH 那边"一条物理连接、多个 Channel"的多路复用思路，但协议本身比 SSH 简单得多，不需要重新发明信道流控——直接设计一个简单的二进制分帧：

```
┌──────────┬──────────┬──────────┬────────────────────┐
│ 长度(u32) │ 流ID(u32) │ 帧类型(u8)│      Payload        │
│  4 bytes │  4 bytes │  1 byte  │   (长度-9 bytes)     │
└──────────┴──────────┴──────────┴────────────────────┘
```

- **流 ID**：客户端为每个逻辑请求（一次 `read_file`、一个交互式终端会话）分配一个自增 `u32`，响应帧带回同一个流 ID，客户端按流 ID 分发到对应的等待者（`oneshot::Sender` 或流式场景下的 `mpsc::Sender`）——这一层结构在 `SshSession`（`channels: Mutex<HashMap<Uuid, ...>>`）和 `mcp/stdio.rs`（按响应 `id` 分发）里都已经用过同样的模式，是这个代码库里成熟的既有套路，不是新发明。
- **帧类型**：
  - `0x01 Control`：Payload 是一个 JSON 编码的 `protocol::Request`/`protocol::Response`（见 §3.3），用于所有"一问一答"的 RPC（`list_dir`/`read_file` 元数据/`exec` 等）。
  - `0x02 DataChunk`：Payload 是裸二进制字节，用于大文件读写、终端输出——**不经过 JSON**，避免 base64 编码带来的 33% 体积膨胀和序列化开销（当前 SFTP 路径下这个问题不存在是因为 `russh-sftp` 本身是二进制协议；如果 Agent 协议图省事把文件内容塞进 JSON 字符串字段，大文件传输会明显变慢，这是设计阶段就要避免的坑）。
  - `0x03 StreamEnd`：标记某个流的数据结束（对应 `read_file` 分块读完、终端会话关闭）。
  - `0x04 Error`：流级别错误（比 `Control` 帧里携带的业务错误更底层，比如流 ID 冲突）。

### 3.3 `protocol` crate 的核心类型

```rust
// protocol/src/lib.rs
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum Request {
    Handshake { token: String, client_version: String },
    ListDir { path: String },
    Stat { path: String },
    ReadFile { path: String },                 // 响应通过 DataChunk 帧流式返回，Control 帧只带 mtime/size 元信息
    WriteFile { path: String, expected_mtime: Option<i64> }, // 请求方随后发 DataChunk，最后发 StreamEnd
    Delete { path: String, is_dir: bool },
    Rename { from: String, to: String },
    CreateDir { path: String },
    ListRoots,                                   // 盘符列表：C:\, D:\ ...（Windows 路径的"根"概念）
    Exec { command: String, args: Vec<String>, cwd: String, timeout_secs: u32 },
    SearchContent { root: String, query: String, options: SearchOptions },
    SearchFileName { root: String, query: String },
    OpenShell { cols: u16, rows: u16, cwd: String }, // Phase 2：ConPTY 交互式终端
    ShellResize { cols: u16, rows: u16 },
    WatchStart { path: String, recursive: bool },     // Phase 3
    WatchStop,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Response {
    Ok(ResponseBody),
    Conflict { current_mtime: i64, current_preview: String }, // 与 fsops::WriteOutcome::Conflict 对齐
    Error { code: ErrorCode, message: String },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ResponseBody {
    Entries(Vec<FileEntry>),
    FileMeta { size: u64, mtime: i64 },
    Written { mtime: i64 },
    ExecResult { exit_code: Option<i32>, output: String },
    SearchResults(Vec<SearchFileResult>),
    Roots(Vec<String>),
    Empty,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum ErrorCode {
    NotFound, PermissionDenied, OutsideAllowedRoots, InvalidArgument, Internal, AuthFailed,
}

// FileEntry / SearchOptions / SearchFileResult 直接复用与
// src-tauri/src/fsops/mod.rs 里同名结构体字段一致的定义（客户端 AgentFileOps
// 做一层平凡的字段映射转换成 fsops::FileEntry，避免让 fsops 反过来依赖这个
// protocol crate——依赖方向始终是 protocol 被两边依赖，不依赖任何一边）。
```

`Handshake` 请求-响应必须是每条物理连接建立后的第一次交互；`token` 校验失败直接断开连接并在 Agent 侧记审计日志（见 §五）。

---

## 四、客户端集成方案

### 4.1 `ConnectionProfile` 扩展

```rust
// src-tauri/src/connection/profile.rs
pub enum Protocol {
    Ssh,
    Rdp,
    Agent,   // 新增
}
```

`options: Option<serde_json::Value>` 字段（当前只有 RDP 在用）新增 Agent 专属的 JSON 子结构：

```json
{ "pairing_token_ref": "agent:{connection_id}:token" }
```

复用与 SSH 密码/RDP 选项相同的模式——非敏感的 host/port 存 `ConnectionProfile` 本体，配对令牌走 `credential_ref` 一样的系统密钥链（`CredentialStore`），不落库明文。`auth_method` 字段对 Agent 协议不适用（没有用户名/密码的概念），UI 层在 `protocol == Agent` 时隐藏用户名/认证方式选择，只保留主机、端口、"配对令牌"一个输入框。

### 4.2 `AgentConnectionPool` 与 `WorkspaceManager` 改造

`WorkspaceManager::open_remote`（`src-tauri/src/workspace/mod.rs:115`）目前硬编码走 `self.ssh_pool.get_or_connect(...)` 构造 `RemoteFileOps`。改造成按 `connection.protocol` 分支：

```rust
pub async fn open_remote(&self, connection_id: Uuid, remote_path: &str) -> Result<WorkspaceHandle, AppError> {
    let connection = self.connection_manager.get(connection_id)?.ok_or(...)?;
    let file_ops: Arc<dyn FileOps> = match connection.protocol {
        Protocol::Ssh => {
            let session = self.ssh_pool.get_or_connect(connection_id).await?;
            Arc::new(RemoteFileOps::new(session))
        }
        Protocol::Agent => {
            let session = self.agent_pool.get_or_connect(connection_id).await?;
            Arc::new(AgentFileOps::new(session))
        }
        Protocol::Rdp => return Err(AppError::Internal("RDP 连接不能作为文件工作区".into())),
    };
    // 以下 profile/metadata 落库逻辑与协议无关，原样复用
    ...
}
```

`AgentConnectionPool` 结构上与 `SshConnectionPool` 同构（`HashMap<Uuid, Arc<AgentSession>>` + 懒建连 + 复用），`AgentSession` 内部管理一条 TLS 连接上的多个逻辑流（§3.2 的流 ID 分发），对外暴露 `async fn request(&self, req: protocol::Request) -> Result<protocol::Response, AppError>` 以及流式版本 `async fn request_streamed(&self, req) -> impl Stream<Item = Vec<u8>>`（给 `read_file`/终端输出用）。

### 4.3 `AgentFileOps` 实现骨架

```rust
// src-tauri/src/fsops/agent.rs
pub struct AgentFileOps {
    session: Arc<AgentSession>,
}

#[async_trait]
impl FileOps for AgentFileOps {
    async fn list_dir(&self, path: &str) -> Result<Vec<FileEntry>, AppError> {
        match self.session.request(protocol::Request::ListDir { path: path.into() }).await? {
            protocol::Response::Ok(protocol::ResponseBody::Entries(entries)) =>
                Ok(entries.into_iter().map(Into::into).collect()),
            protocol::Response::Error { message, .. } => Err(AppError::Internal(message)),
            _ => Err(AppError::Internal("unexpected response".into())),
        }
    }

    async fn read_file_raw(&self, path: &str) -> Result<(Vec<u8>, i64), AppError> {
        let mut stream = self.session.request_streamed(protocol::Request::ReadFile { path: path.into() }).await?;
        let mut buf = Vec::new();
        let mut mtime = 0i64;
        while let Some(chunk) = stream.next().await {
            match chunk {
                StreamItem::Meta { mtime: m, .. } => mtime = m,
                StreamItem::Data(bytes) => buf.extend_from_slice(&bytes),
                StreamItem::End => break,
            }
        }
        Ok((buf, mtime))
    }

    async fn write_file_bytes(&self, path: &str, bytes: &[u8], expected_mtime: Option<i64>) -> Result<WriteOutcome, AppError> {
        self.session.write_stream(protocol::Request::WriteFile { path: path.into(), expected_mtime }, bytes).await
    }

    // delete/rename/create_dir：与上面同样"发 Request、匹配 Response"的直译模式，略。
    // copy/replace_text：不单独实现，沿用 FileOps trait 的默认实现（基于 list_dir/read_file_raw/
    // write_file_bytes 组合而成），和 RemoteFileOps 对 replace_text 的处理方式一致。
}
```

至此 Explorer 的目录树浏览、点击编辑+`Ctrl+S`保存+mtime 冲突检测、SFTP 快捷工具的跨目录浏览、AI 编程助手的 `read_file`/`write_file`/`edit_file`/`list_directory` 工具、全局搜索/替换——全部通过 `&dyn FileOps` 这一层自动获得 Agent 支持，不需要在这些模块里写任何 `if protocol == Agent` 分支。

### 4.4 命令执行与终端

`coding/session.rs` 的 `CodingTarget`（AI 编程助手 `run_command` 工具的执行目标）目前只有 `Local`/`Ssh` 两态，比照 §4.2 的做法扩展一个 `Agent(Arc<AgentSession>)` 分支，执行时发送 `protocol::Request::Exec`——**注意命令语法**：Agent 侧 `Exec` 请求带的是 `command + args: Vec<String>` 而不是"拼成一行字符串再转义"，直接对应 Windows `CreateProcess`/`std::process::Command` 的参数数组语义，天然避免 `log/remote.rs::shell_quote` 那种针对 POSIX shell 设计的转义规则在 Windows 目标上失配的问题（§一 表格已指出这一点）。§3.8.2.1 的黑名单硬拦截/只读白名单/逐条确认/审计日志机制原样复用，只是黑名单模式库需要补一套 Windows 专属的危险命令（`Remove-Item -Recurse -Force C:\`、`Format-Volume`、`Stop-Computer`/`Restart-Computer`、`reg delete`、修改防火墙规则等），不能直接照搬现有的 `rm -rf /` 这套 Linux 模式列表。

交互式终端（`TerminalPanel` 新增 `kind: 'agent'`）是 Phase 2 工作（见 §九），落地时后端用 Agent 侧的 `ConPTY`（`CreatePseudoConsole` Win32 API）起一个真正的 `powershell.exe`/`cmd.exe`，字节流通过 §3.2 的 `DataChunk` 帧双向传输，客户端 `TerminalView`/`terminalStore` 的渲染逻辑不需要改（和现有 `ssh`/`local` 两态一样，只是多一种数据来源）。

---

## 五、安全设计

- **配对令牌（pairing token）**：Agent 首次启动（或执行 `roc_desk_agent.exe pair` 子命令）生成一个高熵随机令牌（32 字节，Base32 编码展示，形如 `roc-agent-XXXX-XXXX-XXXX-XXXX`），打印到控制台并写入本地配置文件的哈希（`argon2` 或 `sha256`，配置文件本身不存明文令牌）。运维人员把这串令牌通过一条可信信道（内部 IM、口头、密码管理器分享）交给需要连接的 roc_desk 用户；用户在"新建连接"对话框里粘贴这个令牌，客户端立即用它换取一次 `Handshake`，握手成功后令牌本体存入 `CredentialStore`（系统密钥链），后续复用，不需要每次连接都重新输入。
  - 令牌是**一次配对多次使用**，不是一次性——这与 TLS 指纹 TOFU 共同构成两层信任：指纹保证"连的是同一台机器"，令牌保证"这个客户端被这台机器的管理员授权过"。
  - **未做**：令牌轮换/过期策略、多用户各自独立令牌（当前设计是"一台 Agent 一个令牌"，多个操作者共享同一令牌；如果需要按人审计，需要在协议里加一个"令牌标签"字段，v1 不做，留作后续项）。
- **路径访问范围限制（allowed_roots）**：Agent 配置文件可选配置一个允许访问的根路径白名单（如 `["D:\\projects", "E:\\www"]`）；配置了白名单时，所有涉及路径的请求先校验目标路径是否落在白名单某一项之内（规范化后比较，防止 `..` 或符号链接绕过），命中范围外直接返回 `OutsideAllowedRoots` 错误。不配置（留空）则默认允许访问整机（等价于当前 SSH 场景下"能连上就能读写这个账户权限范围内的任何文件"的行为），由管理员按实际风险承受能力决定，UI 上明确提示两种模式的区别。
- **审计日志**：Agent 侧独立维护一份轻量审计日志（本地 SQLite 或滚动文本文件，不依赖客户端），记录每次 `Exec` 请求、每次握手成功/失败（含来源 IP）、路径越界拒绝事件；这是防御纵深的第二层——即便客户端本身的 `command_audit_log`（`coding/` 模块已有）被篡改或客户端主机被攻陷，服务端这份日志仍然是可信来源。
- **网络暴露面**：Agent 默认只监听指定端口（不做端口转发/穿透），使用场景假定客户端与 Agent 在同一内网或已建立 VPN/跳板——**不设计公网直接暴露场景**，文档里需要明确提示用户不要把 Agent 端口暴露到公网，必要时结合 Windows 防火墙规则限制来源 IP。这与现有 SSH 场景的风险模型一致（SSH 端口暴露公网同样不推荐），不是 Agent 独有的新风险，但因为协议是自定义的、没有经过像 SSH 那样多年的公开审计，更应该保守。

---

## 六、Agent 部署与生命周期管理

### 6.1 CLI 子命令

```
roc_desk_agent.exe run                  # 前台运行，日志打印到控制台（测试/临时场景）
roc_desk_agent.exe pair                 # 生成/重新生成配对令牌，打印给管理员
roc_desk_agent.exe install-service      # 注册为 Windows 服务（开机自启、后台常驻）
roc_desk_agent.exe uninstall-service
roc_desk_agent.exe status               # 查看当前监听地址、已配对状态、最近连接
```

`install-service` 基于 `windows-service` crate 实现（新依赖，仅 `agent/` crate 引入，不污染 `src-tauri/`），服务账户默认使用运行安装命令时的账户权限；文档需要提示"Agent 进程的文件访问权限 = 运行它的 Windows 账户权限"，管理员应按最小权限原则创建一个专用服务账户，而不是图省事用本机管理员账户跑。

### 6.2 配置文件（`agent.toml`，与 exe 同目录，对齐 REQUIREMENTS.md §3.9"可变内容不打包进 exe、放同级目录"的既有约定）

```toml
[server]
listen_addr = "0.0.0.0"
port = 7879                     # 与现有 RDP/SSH 端口不冲突的默认值，可改

[security]
allowed_roots = []               # 空 = 不限制；示例：["D:\\projects", "E:\\logs"]
token_hash = "argon2id$..."      # pair 子命令生成，不存明文

[limits]
max_concurrent_connections = 8
exec_timeout_secs = 120
```

---

## 七、与现有方案的选型对照（给用户/运维的决策参考）

| 场景 | 推荐方案 |
|------|----------|
| 目标是 Linux 服务器 | 继续用现有 SSH+SFTP，无需 Agent |
| 目标 Windows 已装好 OpenSSH Server 且能正常连接 | 两者都可用；已有 SSH 习惯（如已经维护了大量连接档案、习惯 known_hosts 模型）可以继续用 SSH，不强制迁移 |
| 目标 Windows 没有/不方便装 OpenSSH（老版本 Server、锁死的安全基线） | 用 Agent——这是本方案要解决的主要场景 |
| 需要频繁做工作区级全文搜索、目标机器文件数量大 | 优先 Agent（§一表格已说明搜索性能是架构级优势） |
| 只需要图形远程桌面 | 用 RDP 模块，与本方案无关 |

---

## 八、实现阶段划分

- **Phase 1（MVP，文件系统 + 一次性命令执行）**：`protocol`/`agent` crate 骨架、TLS 握手 + 指纹 TOFU、配对令牌、`AgentFileOps` 全量实现（覆盖 `FileOps` trait 的基础原语）、`Exec` 一次性命令执行接入 AI 编程助手的 `run_command` 工具。**不做**交互式终端、不做文件监听推送。此阶段即可让 Explorer/SFTP 快捷工具/AI 编程助手/全局搜索四大模块对 Windows Agent 工作区可用，是性价比最高的一步。
- **Phase 2（交互式终端）**：Agent 侧 ConPTY 集成，`OpenShell`/`ShellResize` 协议方法，客户端 `TerminalPanel` 新增 `kind: 'agent'`，复用现有断线可见性 + 手动重连 UI（DESIGN.md §3.2.3 / REQUIREMENTS.md §3.2 已有的模式）。
- **Phase 3（文件变更推送）**：`WatchStart`/`WatchStop` + Agent 侧 `notify` 监听，Explorer 树对 Agent 工作区实现自动刷新（目前本地/SSH 工作区都没有这个能力，属于借这次机会做的增量提升，不是回填缺口）。
- **暂不规划**：Agent 转发/跳板链式连接（类比 SSH 的 `jump_host_id`）、多令牌分用户审计、公网穿透——这些在明确出现真实需求前不做，避免过度设计。

---

## 九、开放问题（需要在详细设计/实现前拍板）

1. **Windows 服务账户的权限边界**：默认用哪个账户跑服务、文档要不要强制引导创建专用最小权限账户，还是把这个决策完全留给运维。
2. **`allowed_roots` 是否要做成必填**：完全放开访问整机的默认值是否风险过高，是否应该改成"首次运行强制要求配置至少一个根路径，留空需要显式二次确认"。
3. **配对令牌是否需要支持吊销**：当前设计里丢失/泄露的令牌只能靠重新 `pair`（生成新令牌，旧令牌连带整个 Agent 的信任状态一起失效）来处理，是否需要更细粒度的"撤销单个客户端"能力。
4. **`protocol` crate 版本兼容策略**：客户端和 Agent 分开升级是必然场景（Agent 部署在远程服务器上，不会和客户端同步升级），`Handshake` 里的 `client_version`/未来可能需要的 `agent_version` 字段之间要不要做协议版本协商，还是简单地"不兼容就报错，提示用户升级 Agent"。

---

## 十、验收标准（Phase 1 完成的判定依据）

- 在一台没有 OpenSSH Server 的干净 Windows Server 虚拟机上，仅拷贝 `roc_desk_agent.exe` + 运行 `pair` + `run`，即可从 roc_desk.exe 新建一条 `Protocol::Agent` 连接档案并成功打开远程工作区。
- 首次连接必须弹出证书指纹确认对话框；篡改/替换 Agent 后重连必须触发红色告警，不能静默放行。
- Explorer 对该工作区的浏览/点击编辑/`Ctrl+S`保存/mtime 冲突检测四个行为与本地工作区、SSH 远程工作区表现一致（同一套前端组件，理论上零改动即可通过）。
- AI 编程助手在该工作区下能正常调用 `read_file`/`write_file`/`edit_file`/`list_directory`/`search_files`/`run_command` 六个工具，`run_command` 的黑名单拦截对 Windows 专属危险命令生效。
- 全局搜索（`Ctrl+Shift+F`）对一个几千文件规模的远程目录，端到端耗时明显低于同等规模下 SSH+SFTP 路径的表现（验证 §一表格里"搜索性能"这条设计收益是否真实兑现）。
