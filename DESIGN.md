# roc_desk — 集成开发者客户端工具设计方案

> 一个基于浏览器式界面的集成开发工具，集 SSH 终端、SFTP、日志搜索、网页浏览、统一 AI工具于一体。
> 目标平台：Windows（兼容 macOS / Linux）

> **本文档是架构/技术方案，记录"打算怎么做"；逐项功能"实际做到什么程度、用户提过哪些具体要求、踩过哪些真实 bug"见 [REQUIREMENTS.md](REQUIREMENTS.md)，也可以直接跳到本文末尾的[十二、实现状态](#十二实现状态)速览。两份文档职责不同，请勿把实现细节同步写重复。

---

## 一、总体架构选型：Tauri 2.0

### 为什么选择 Tauri 2.0

| 对比维度 | Electron | Qt | Tauri 2.0 |
|---------|----------|-----|-----------|
| 打包体积 | ~150MB | ~80MB | **~5–10MB** |
| 内存占用 | 高 | 中 | **低** |
| 后端语言 | Node.js (JS) | C++ | **Rust** |
| 前端语言 | JS/TS | C++ (QML) | **JS/TS** |
| WebView | 内置 Chromium | 自有渲染 | **系统 WebView2** |
| 跨平台 | ✅ | ✅ | ✅ |
| 安全性 | 一般 | 高 | **高（Rust 安全模型）** |
| 学习曲线 | 低 | 高 | 中 |

**结论**：Tauri 2.0 后端使用 Rust（用户熟悉的语言），前端使用 JS/TS，利用 Windows 系统自带的 WebView2，安装包极小，性能优秀，是最优选择。

### 技术栈总览

```
┌─────────────────────────────────────────────────────┐
│                    Frontend (WebView2)               │
│  React 18 + TypeScript + Vite                       │
│  ┌──────────┬──────────┬──────────┬───────────────┐  │
│  │ xterm.js │ Monaco   │ SFTP     │ AI Chat UI    │  │
│  │ Terminal │ Editor   │ Browser  │ (Markdown)    │  │
│  └────┬─────┴────┬─────┴────┬─────┴───────┬───────┘  │
│       │ WebSocket│ IPC     │ IPC         │ IPC     │
├───────┼──────────┼─────────┼─────────────┼─────────┤
│                    Backend (Rust)                     │
│  ┌──────────┬──────────┬──────────┬───────────────┐  │
│  │ russh    │ russh-   │ SQLite   │ reqwest       │  │
│  │ SSH客户端│ sftp     │ FTS5     │ HTTP/AI API   │  │
│  └──────────┴──────────┴──────────┴───────────────┘  │
│  ┌──────────┬──────────┬──────────┐                  │
│  │ mlua     │ tokio    │ Tauri    │                  │
│  │ Lua脚本  │ 异步运行时│ Plugin   │                  │
│  └──────────┴──────────┴──────────┘                  │
└─────────────────────────────────────────────────────┘
```

---

## 二、核心开源组件清单

### 2.1 后端（Rust）

| 组件 | 仓库 | 协议 | 用途 | Stars |
|------|------|------|------|-------|
| **Tauri 2.0** | [tauri-apps/tauri](https://github.com/tauri-apps/tauri) | MIT/Apache-2.0 | 应用框架 | 88k+ |
| **russh** | [Eugeny/russh](https://github.com/Eugeny/russh) | Apache-2.0 | SSH 客户端库 | 300+ |
| **russh-sftp** | [crates.io/crates/russh-sftp](https://crates.io/crates/russh-sftp) | Apache-2.0 | SFTP 子系统 | — |
| **rusqlite** | [rusqlite](https://crates.io/crates/rusqlite) | MIT | SQLite + FTS5 | — |
| **reqwest** | [seanmonstar/reqwest](https://github.com/seanmonstar/reqwest) | MIT/Apache-2.0 | HTTP 客户端（AI API） | 10k+ |
| **tokio** | [tokio-rs/tokio](https://github.com/tokio-rs/tokio) | MIT | 异步运行时 | 28k+ |
| **mlua** | [khvzak/mlua](https://github.com/khvzak/mlua) | MIT | Lua 脚本引擎 | 1k+ |
| **serde** / **serde_json** | [serde-rs/serde](https://github.com/serde-rs/serde) | MIT/Apache-2.0 | 序列化 | 9k+ |
| **tracing** | [tokio-rs/tracing](https://github.com/tokio-rs/tracing) | MIT | 日志框架 | 5k+ |
| **tauri-plugin-fs** | Tauri 官方 | MIT/Apache-2.0 | 文件系统操作 | — |
| **tauri-plugin-shell** | Tauri 官方 | MIT/Apache-2.0 | 进程调用 | — |
| **tauri-plugin-store** | Tauri 官方 | MIT/Apache-2.0 | KV 存储 | — |
| **notify** | [notify-rs/notify](https://github.com/notify-rs/notify) | CC0-1.0 | 文件变更监控 | 3k+ |
| **portable-pty** | [wez/wezterm (pty)](https://github.com/wez/wezterm) | MIT | 本地 PTY（可选） | 8k+ |

### 2.2 前端（TypeScript / React）

| 组件 | 仓库 | 协议 | 用途 | Stars |
|------|------|------|------|-------|
| **React 18** | [facebook/react](https://github.com/facebook/react) | MIT | UI 框架 | 235k+ |
| **Vite** | [vitejs/vite](https://github.com/vitejs/vite) | MIT | 构建工具 | 72k+ |
| **xterm.js** | [xtermjs/xterm.js](https://github.com/xtermjs/xterm.js) | MIT | 终端模拟器前端 | 18k+ |
| **Monaco Editor** | [microsoft/monaco-editor](https://github.com/microsoft/monaco-editor) | MIT | 代码/文本编辑器 | 40k+ |
| **react-markdown** | [remarkjs/react-markdown](https://github.com/remarkjs/react-markdown) | MIT | AI 回答渲染 | 13k+ |
| **shadcn/ui** | [shadcn-ui/ui](https://github.com/shadcn-ui/ui) | MIT | UI 组件库 | 80k+ |
| **lucide-react** | [lucide-icons/lucide](https://github.com/lucide-icons/lucide) | ISC | 图标库 | 17k+ |
| **TailwindCSS** | [tailwindlabs/tailwindcss](https://github.com/tailwindlabs/tailwindcss) | MIT | CSS 框架 | 86k+ |
| **zustand** | [pmndrs/zustand](https://github.com/pmndrs/zustand) | MIT | 状态管理 | 50k+ |
| **react-arborist** | [brimdata/react-arborist](https://github.com/brimdata/react-arborist) | MIT | 树形侧边栏 | 3k+ |
| **react-resizable-panels** | [bvaughn/react-resizable-panels](https://github.com/bvaughn/react-resizable-panels) | MIT | 可拖拽面板 | 4k+ |
| **@tanstack/react-virtual** | [TanStack/virtual](https://github.com/TanStack/virtual) | MIT | 虚拟列表（日志） | 6k+ |

### 2.3 可选扩展组件

| 组件 | 仓库 | 协议 | 用途 |
|------|------|------|------|
| **Code Server** | [coder/code-server](https://github.com/coder/code-server) | MIT | 嵌入式 VS Code 编码环境 |
| **Open WebUI** | [open-webui/open-webui](https://github.com/open-webui/open-webui) | MIT | 多模型 AI 对话前端参考 |
| **Lobe Chat** | [lobehub/lobe-chat](https://github.com/lobehub/lobe-chat) | Apache-2.0 | AI 聊天 UI 参考 |
| **Headlines** | [ripienaar/gping](https://github.com/orf/gping) | MIT | 可视化工具参考 |
| **ripgrep** | [BurntSushi/ripgrep](https://github.com/BurntSushi/ripgrep) | MIT/Unlicense | 高性能文本搜索（可作为 Rust 库嵌入） |

---

## 三、功能模块设计

### 3.1 工作区与整体 UI 布局（VS Code 式）

> 本节替换了早期"浏览器式、模块独立标签、侧边栏=连接树"的设计。新模型以**工作区（Workspace）**为核心入口，交互心智对齐 VS Code：先选目录，再在这个目录的上下文里做所有事情。

#### 3.1.1 工作区模型（Workspace）

工作区是一个「本地目录」或「远程主机上的某个目录」的绑定，是应用的第一入口，而不是先连接主机再决定看什么。

```
┌─────────────────────────────────────────────────────────┐
│                      roc_desk                              │
│                                                             │
│   [📁 打开本地文件夹]      [🖥 连接远程主机并选择目录]      │
│                                                             │
│   最近打开的工作区                                          │
│   ┌───────────────────────────────────────────────────┐   │
│   │ 💻  my-local-app          F:\code\my-local-app      │   │
│   │ 🖥  web-01 · nginx-conf   /etc/nginx  (root@web-01)  │   │
│   │ 🖥  web-01 · api-service  ~/app  (root@web-01)       │   │
│   │ 💻  scratch                D:\scratch                │   │
│   └───────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────┘
```

- **数据模型**：`WorkspaceProfile { id, kind: Local|Remote, root_path, connection_id?, display_name, last_opened_at }`；`kind = Remote` 时关联一个 `ConnectionProfile`（见 §3.2.2），凭据仍走系统密钥链，工作区表本身不存机密。
- **打开本地文件夹**：走系统原生目录选择器（`tauri-plugin-dialog`），选中后直接作为工作区根目录。
- **连接远程主机并选择目录**：先从已保存的连接档案选择主机（或新建一个），连接成功后弹出一个轻量的远程目录选择器（复用 SFTP 的目录浏览能力，仅用于"选根目录"这一步，不承担完整文件管理），确认后作为工作区根目录。
- **最近打开的工作区**：本地/远程用不同图标区分（💻/🖥），显示根路径和（远程时）主机名；点击直接重新打开——远程工作区会自动触发 SSH 重连（复用 §3.2.3 的重连策略）。右键可"从列表移除""在新窗口打开"。
- **切换工作区**：`Ctrl+Shift+O` 或菜单"文件 → 打开工作区"随时可以回到这个选择页；不强制退出应用重来，也支持在新窗口打开第二个工作区（多工作区并行，互不干扰）。
- **无工作区兜底**：应用支持不打开任何工作区、只用终端/AI工具等独立工具的场景（比如只是想连一下服务器执行命令），此时顶部快捷工具依然可用，但左侧 Explorer 为空态，提示"打开一个工作区以浏览文件"。

#### 3.1.2 进入工作区后的整体布局

```
┌──────────────────────────────────────────────────────────────────────┐
│ [my-app ▾]  [🖥 终端] [📄 main.rs]                 [📂][🌐][✨] [+] │  ← Tab 栏 + 顶部快捷工具
├──────────────┬───────────────────────────────────────────────────────┤
│  Explorer    │                                                        │
│  ▾ src/      │              内容区域（按当前 Tab 切换）                │
│    main.rs ● │                                                        │
│    api.rs    │   - 终端 (xterm.js)                                   │
│  ▾ tests/    │   - 代码编辑器 (Monaco，可编辑保存，非只读)             │
│  Cargo.toml  │   - SFTP 传输管理器 / 网页浏览；右侧停靠 AI工具       │
│  README.md   │                                                        │
├──────────────┴───────────────────────────────────────────────────────┤
│  🖥 web-01 · 24ms   Explorer: /home/user/app   UTF-8   Ln 12, Col 4  │
└──────────────────────────────────────────────────────────────────────┘
```

- **左侧 Sidebar 从"连接树"变为"Explorer 文件树"**：根目录固定为当前工作区根目录，本地工作区走文件系统 API，远程工作区走 SFTP，均懒加载子目录（不会一次性拉全树）。原「连接管理」侧边栏（§3.2.2 的档案 CRUD）改为通过工作区选择页或 Activity Bar 的「服务器」图标单独呼出，不再占用默认侧边栏位置。
- **Explorer 点击行为对齐 VS Code**：单击文件 → 以"预览标签"（斜体文件名、颜色浅一档）打开，再单击别的文件会复用/替换这个预览标签，避免手滑点一圈文件后开一堆标签；双击 → 转为固定标签。点击的文件默认进入**可编辑**状态（不是早期设计里"仅浏览用只读 Monaco"），支持 `Ctrl+S` 保存（本地直接写文件系统，远程走 SFTP 写回并给出成功/失败反馈）、未保存时标签上出现圆点，关闭前二次确认。
- **搜索**：`Ctrl+F` 为当前文件内搜索；`Ctrl+Shift+F` 打开"在工作区中搜索"侧栏，本地/远程分别复用 §3.4 日志搜索模块的两套引擎（本地 ripgrep 直接跑，远程 `rg` over SSH），但这是面向代码/文本的通用搜索，与 §3.4 面向日志（时间范围、级别、FTS5 索引）的专门工具是两个独立入口，服用同一套底层搜索能力。
- **默认打开的 Tab**：进入工作区后自动打开一个终端标签，并展开底部面板——远程工作区打开该主机的 SSH 终端（自动连接，走 §3.2.1 的指纹校验流程），本地工作区打开本机 Shell（PowerShell/bash，`portable-pty`）。
- **状态栏**：随当前工作区与激活 Tab 变化，左侧显示连接状态（远程）或本地路径（本地），右侧显示当前编辑器的编码/光标位置等上下文信息。

#### 3.1.3 顶部快捷工具与 Tab 复用

Tab 栏右侧固定一排图标（SFTP 传输 📂 / 网页浏览 🌐 / AI工具 ✨），点击 AI工具后在工作区右侧打开停靠栏：

1. 若当前工作区**已经**打开过该工具的 Tab → 直接激活已有 Tab（不新开、不丢失该 Tab 内的状态，如对话历史、SFTP 当前目录）。
2. 若没有 → 在当前工作区下新开一个绑定该工作区的 Tab。
3. 工具 Tab 与终端/编辑器 Tab 是同级关系，共用同一个 Tab 栏、支持拖拽排序和 `Ctrl+Tab` 切换。

**各工具与工作区的关系**：

| 工具 | 与工作区的关系 |
|------|----------------|
| SFTP 传输管理器 | 不再承担"浏览工作区目录"的职责（已由 Explorer 覆盖），定位收窄为**跨目录自由浏览与批量传输**：查看工作区之外的路径（如 `/etc/nginx`）、大批量上传下载、查看文件权限。远程工作区下默认复用工作区所在主机的连接；本地工作区下若要用 SFTP 需先选择一个已保存的远程连接。 |
| 网页浏览 | 与工作区类型无关，始终可用。 |
| AI工具 | 统一承载普通问答与编程任务，停靠在工作区右侧；自动绑定当前工作区，Plan 只读分析、Build 可操作文件；两种模式都提供 `web_search` 互联网搜索工具，见 §3.6 与 §3.8。 |

AI工具与中央编辑区之间设置可左右拖动的分隔条，面板宽度持久化；拖动边界同时保证 AI工具和编辑区均保留可操作的最小宽度。

#### 3.1.4 远程文件编辑模型（本地化编辑体验）

这是工作区模型能否成立的关键一环：**远程文件必须能像本地文件一样直接编辑**，而不是"只能看、改不了，要改就下载—改—上传"。设计如下：

- **编辑发生在本地内存缓冲区**：点击 Explorer 中的远程文件时，通过 SFTP 把内容整体（或按 §3.3.1 的分级策略分页）读入前端的编辑器缓冲区；之后所有键入、格式化、查找替换等操作都在这个本地缓冲区里进行，**不产生任何网络往返**，输入手感与编辑本地文件完全一致，不会因为网络延迟而卡顿或丢字符。
- **保存时才写回**：`Ctrl+S` 触发一次性 SFTP 写回（`sftp_write_file` Command），成功后标签圆点消失；失败（断网、权限不足、磁盘满）时**保留本地缓冲区内容不丢失**，标签圆点保持，并在状态栏/Toast 明确提示失败原因和"重试保存"，绝不能出现"保存失败但编辑器内容也没了"的情况。
- **保存前的冲突检测**：打开文件时记录远程文件的 `mtime`（可选再加内容哈希）；保存前先比对远程当前 `mtime` 是否与打开时一致——一致则直接写入；不一致（文件在编辑期间被别的进程/用户改过）则弹窗提示"远程文件已被修改"，提供「查看差异」「仍要覆盖」「另存为副本」三个选项，不静默覆盖别人的修改。
- **断线容忍**：编辑过程中 SSH 断开不影响本地缓冲区（缓冲区完全独立于连接状态），依赖 §3.2.3 的重连策略恢复连接后即可正常保存；若用户在断线期间尝试保存，提示"连接已断开，无法保存，内容已保留在本地"并允许稍后重试，不阻塞继续编辑。
- **与大文件策略的边界**：沿用 §3.3.1 的大文件分级——超过分页阈值（当前定义 50MB）的远程文件默认走"分页只读"模式，不支持整篇本地缓冲编辑（避免对一个尚未完整加载的缓冲区做保存产生数据丢失风险）；用户可选择"下载到本地编辑后再上传覆盖"作为大文件的编辑路径，UI 上需明确提示这一限制而不是让编辑器悄悄变只读。

### 3.2 SSH 终端模块

**数据流**：
```
用户键盘 → xterm.js(前端渲染) → WebSocket → Rust后端 → russh SSH连接 → 远程服务器
远程输出 → russh → Rust后端 → WebSocket → xterm.js → 屏幕渲染
```

**关键实现**：
- 前端：`@xterm/xterm` + `@xterm/addon-fit`（自适应窗口大小）
- 后端：`russh` 建立 SSH 连接，`russh::Channel` 管理会话
- 通信：Tauri Event 系统（WebSocket 或直接 IPC）传输终端数据
- 支持：密码/密钥/SSH Agent 认证、端口转发、Keep-Alive

**Rust 核心代码骨架**：
```rust
use russh::{client, ChannelId};
use russh_keys::key::KeyPair;

pub struct SshSession {
    session: russh::client::Handle<SshHandler>,
    channel: ChannelId,
}

impl SshSession {
    pub async fn connect(host: &str, port: u16, user: &str, auth: AuthMethod) -> Result<Self> {
        let config = russh::client::Config::default();
        // SshHandler::check_server_key 中实现 known_hosts 校验（见下文）
        let sh = SshHandler::new(host, port);
        let mut session = russh::client::connect(Arc::new(config), (host, port), sh).await?;
        // 认证...
        Ok(Self { session, channel })
    }

    pub async fn send_data(&self, data: &[u8]) -> Result<()> {
        self.session.data(self.channel, CryptoVec::from(data)).await
    }
}
```

#### 3.2.1 主机指纹校验（known_hosts / TOFU）

`russh::client::Handler::check_server_key` 默认不做任何校验，**必须显式实现**，否则整个连接对中间人攻击（MITM）无防护：

```rust
impl client::Handler for SshHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &russh_keys::key::PublicKey,
    ) -> Result<bool, Self::Error> {
        let fingerprint = server_public_key.fingerprint();
        match self.known_hosts.lookup(&self.host, self.port) {
            KnownHostStatus::Match => Ok(true),
            KnownHostStatus::Mismatch(old_fp) => {
                // 指纹变化：必须弹窗警告用户，默认拒绝连接，而不是静默通过
                self.emit_host_key_changed(old_fp, fingerprint);
                Ok(false)
            }
            KnownHostStatus::Unknown => {
                // 首次连接（TOFU）：弹窗展示指纹，用户确认后写入本地 known_hosts 存储
                let trusted = self.prompt_trust_on_first_use(fingerprint).await;
                if trusted {
                    self.known_hosts.save(&self.host, self.port, fingerprint)?;
                }
                Ok(trusted)
            }
        }
    }
}
```

- 本地 `known_hosts` 记录建议存于 SQLite（host, port, 公钥指纹, 首次信任时间），前端在 TOFU / 指纹变化时必须弹出**非默认确认**的对话框（不能是"直接连接"一个按钮），避免用户习惯性点掉。
- 指纹变化（Mismatch）场景应比首次连接（Unknown）更醒目地告警（红色警示，而非普通提示）。

#### 3.2.2 连接管理（Connection Profiles）

主界面侧边栏的"连接1/连接2"需要背后的档案管理能力，而不仅是已打开的标签：

- **数据模型**：`ConnectionProfile { id, name, host, port, username, auth_method, group, tags[], last_connected_at, jump_host? }`，持久化于 SQLite（非 Tauri Store，便于按分组/标签查询）。
- **凭据分离存储**：档案本身（host/port/用户名/分组等非敏感信息）存 SQLite；密码/私钥口令等敏感字段单独走系统密钥链（见「安全性设计」），profile 表中只保存密钥链条目的引用 ID，不落地明文。
- **功能**：分组/文件夹、标签筛选、搜索、导入导出（不含凭据，或导出时需二次确认+加密）、快速连接（历史最近使用）、跳板机（ProxyJump）链式连接。
- **多标签复用连接**：同一主机的多个标签页（终端 + SFTP + 日志）应复用同一条 SSH 物理连接，通过 `russh` 的多路复用在其上开多个 `Channel`，而不是每个标签各自握手一次连接，减少握手开销和主机审计噪音。

#### 3.2.3 断线重连策略

长时间保持的 SSH 会话必然会遇到网络抖动，需要显式的重连设计：

- 检测到 `Channel`/`Session` 断开后，状态栏显示"已断开 · 重连中"，而不是让终端静默卡死。
- 指数退避重试（如 1s → 2s → 4s → 8s，封顶 30s，可配置最大重试次数或允许用户手动取消）。
- 终端场景重连成功后不尝试恢复远端 shell 状态（PTY 内的进程通常已丢失），但应保留本地滚动缓冲区，并在终端内插入一条系统消息提示"连接已恢复"。
- SFTP/日志同步等操作性会话，重连失败应保留失败前进度（如已下载的文件分片、已导入的日志偏移量），避免整体重做。

### 3.3 SFTP 文件浏览模块

> 定位调整（见 §3.1.3）：工作区目录的浏览与编辑已由 Explorer 承担，本模块专注**跨目录自由浏览**（工作区之外的路径）与**批量传输/权限管理**，作为顶部快捷工具打开，不再是进入远程主机的默认入口。其目录浏览、大文件分级加载、文本查看器无缝跳转能力（§3.3.1）是共享的底层能力——Explorer 点击文件时走的是同一套后端 Command 和前端过渡动画，只是不需要先经过文件列表这一层。

**实现方案**：
- 后端：`russh-sftp` crate 实现 SFTP 子系统
- 前端：React 树形组件 + 文件列表双面板
- 功能：目录浏览、文件上传/下载、权限查看、在线编辑

**Rust 核心代码骨架**：
```rust
use russh_sftp::client::SftpSession;

pub struct SftpBrowser {
    sftp: SftpSession,
}

impl SftpBrowser {
    pub async fn list_dir(&self, path: &str) -> Result<Vec<FileEntry>> {
        let mut entries = Vec::new();
        let mut dir = self.sftp.open_dir(path).await?;
        while let Some(entry) = dir.read_dir().await? {
            entries.push(FileEntry {
                name: entry.filename().to_string(),
                size: entry.attrs().size,
                modified: entry.attrs().mtime,
                is_dir: entry.attrs().is_dir(),
            });
        }
        Ok(entries)
    }

    pub async fn download_file(&self, remote: &str, local: &str) -> Result<()> {
        let mut file = self.sftp.open(remote).await?;
        let mut local_file = tokio::fs::File::create(local).await?;
        // 流式下载...
        Ok(())
    }
}
```

#### 3.3.1 SFTP → 文本查看器 无缝跳转设计

当用户在 SFTP 文件列表中点击一个文本类文件时，右侧内容区平滑过渡为文本查看器，支持搜索、定位、跳转回目录。

**交互流程**：
```
┌─────────────────────────────────────────────────────────────┐
│  阶段 1：SFTP 文件列表                                       │
│  ┌────────────┬──────────────────────────────────────────┐  │
│  │  侧边栏    │  /var/log/nginx/                         │  │
│  │  SFTP      │  ┌────────────────────────────────────┐  │  │
│  │            │  │ 📁 archive/          dir           │  │  │
│  │            │  │ 📄 access.log    12.4MB  08-17     │  │  │
│  │            │  │ 📄 error.log      3.2MB  08-17  ← 点击│  │
│  │            │  │ 📄 nginx.conf     4.1KB  08-10     │  │  │
│  │            │  └────────────────────────────────────┘  │  │
│  └────────────┴──────────────────────────────────────────┘  │
├─────────────────────────────────────────────────────────────┤
│  阶段 2：过渡动画（~200ms）                                   │
│  ┌────────────┬──────────────────────────────────────────┐  │
│  │  侧边栏    │  /var/log/nginx/ > error.log             │  │ ← 面包屑路径
│  │  (保持)    │  ┌────────────────────────────────────┐  │  │
│  │            │  │ ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ │  │  │ ← 骨架屏加载
│  │            │  │ ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ │  │  │   + 进度条
│  │            │  │ ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ │  │  │
│  │            │  └────────────────────────────────────┘  │  │
│  └────────────┴──────────────────────────────────────────┘  │
├─────────────────────────────────────────────────────────────┤
│  阶段 3：文本查看器就绪                                       │
│  ┌────────────┬──────────────────────────────────────────┐  │
│  │  侧边栏    │  /var/log/nginx/ > error.log    [🔍][↩] │  │ ← 面包屑+工具栏
│  │  (保持)    │  ┌────────────────────────────────────┐  │  │
│  │            │  │ 1 │ 2026/08/17 03:21:05 [error]   │  │  │ ← Monaco Editor
│  │            │  │ 2 │ upstream timed out (110:...)  │  │  │   只读+语法高亮
│  │            │  │ 3 │ client 10.0.0.5, request...   │  │  │   + 搜索面板
│  │            │  │ 4 │ 2026/08/17 03:21:08 [warn]    │  │  │
│  │            │  │ ...                                │  │  │
│  │            │  └────────────────────────────────────┘  │  │
│  │            │  行 3/24580 | UTF-8 | 3.2MB              │  │ ← 状态信息
│  └────────────┴──────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────┘
```

**过渡动画细节**：
| 步骤 | 时长 | 效果 |
|------|------|------|
| 1. 点击响应 | 0ms | 文件行高亮 + 立即更新面包屑路径 |
| 2. 骨架屏 | 0–50ms | 内容区淡出 → 骨架屏淡入（CSS transition） |
| 3. 流式加载 | 50–Nms | SFTP 流式读取，进度条实时更新 |
| 4. 渲染完成 | N+0ms | Monaco Editor 淡入，骨架屏淡出，光标定位到第 1 行 |
| 5. 就绪 | N+100ms | 搜索栏、工具栏可用，状态栏更新 |

**前端核心代码骨架**：
```tsx
// SftpBrowser.tsx — 文件列表中的点击处理
const handleFileClick = async (file: FileEntry) => {
  if (file.is_dir) {
    navigateTo(file.path);
    return;
  }

  // 判断是否为可查看的文本文件
  if (isTextViewable(file.name)) {
    openFileViewer({
      host: currentHost,
      remotePath: file.path,
      fileName: file.name,
      fileSize: file.size,
    });
  } else {
    // 非文本文件：提示下载
    confirmDownload(file);
  }
};

// 文本文件类型检测
const TEXT_EXTENSIONS = new Set([
  '.log', '.txt', '.conf', '.cfg', '.ini', '.yaml', '.yml',
  '.json', '.xml', '.html', '.css', '.js', '.ts', '.py',
  '.sh', '.bash', '.env', '.properties', '.csv', '.sql',
  '.md', '.toml', '.lua', '.rs', '.go', '.java', '.c',
  '.cpp', '.h', '.hpp', '.nginx', '.htaccess', '.rules',
]);

function isTextViewable(filename: string): boolean {
  const ext = filename.substring(filename.lastIndexOf('.'));
  return TEXT_EXTENSIONS.has(ext.toLowerCase()) || !ext.includes('.');
}
```

```tsx
// FileViewer.tsx — 文本查看器组件
const FileViewer: React.FC<FileViewerProps> = ({ host, remotePath, fileName, fileSize }) => {
  const [content, setContent] = useState<string>('');
  const [loading, setLoading] = useState(true);
  const [progress, setProgress] = useState(0);
  const [visible, setVisible] = useState(false);

  useEffect(() => {
    let cancelled = false;

    const loadFile = async () => {
      setLoading(true);
      setVisible(false);

      // 调用 Rust 后端流式读取
      const chunks: string[] = [];
      await invoke('sftp_read_file_stream', {
        host, path: remotePath,
        onProgress: (received: number, total: number) => {
          setProgress(Math.round((received / total) * 100));
        },
        onChunk: (chunk: string) => {
          if (!cancelled) chunks.push(chunk);
        },
      });

      if (!cancelled) {
        setContent(chunks.join(''));
        setLoading(false);
        // 延迟一帧让 Monaco 挂载后再显示，避免闪烁
        requestAnimationFrame(() => setVisible(true));
      }
    };

    loadFile();
    return () => { cancelled = true; };
  }, [host, remotePath]);

  return (
    <div className="file-viewer">
      {/* 面包屑导航 */}
      <Breadcrumb>
        <BreadcrumbItem onClick={goBackToSftp}>{currentDir}</BreadcrumbItem>
        <BreadcrumbSeparator />
        <BreadcrumbItem active>{fileName}</BreadcrumbItem>
      </Breadcrumb>

      {/* 工具栏 */}
      <Toolbar>
        <SearchToggle />           {/* Ctrl+F 搜索 */}
        <GoToLineInput />          {/* Ctrl+G 跳转行号 */}
        <EncodingSelector />       {/* UTF-8 / GBK 切换 */}
        <DownloadButton />         {/* 下载到本地 */}
        <OpenInNewTabButton />     {/* 在新标签中打开 */}
        <BackToSftpButton onClick={goBackToSftp} /> {/* 返回 SFTP */}
      </Toolbar>

      {/* 内容区域：骨架屏 ↔ Monaco 交叉淡入淡出 */}
      <div className="content-area relative">
        {loading && (
          <div className={cn('absolute inset-0 transition-opacity', visible ? 'opacity-0' : 'opacity-100')}>
            <SkeletonText lines={40} />
            <ProgressBar value={progress} />
          </div>
        )}
        <div className={cn('transition-opacity duration-200', visible ? 'opacity-100' : 'opacity-0')}>
          <MonacoEditor
            value={content}
            language={detectLanguage(fileName)}
            options={{
              // 经 Explorer 打开：readOnly=false，可编辑+Ctrl+S 保存（见 §3.1.2）
              // 经 SFTP 快捷工具打开工作区之外的文件：默认 readOnly=true，需用户显式点击"编辑"再转可写，
              // 避免误改一个只是"顺手看看"的系统文件
              readOnly: openedFrom === 'sftp' && !editMode,
              minimap: { enabled: true },
              find: { addExtraSpaceOnTop: false, autoFindInSelection: 'always' },
              fontSize: 13,
              wordWrap: 'on',
            }}
            onChange={handleContentChange}  // 标记 dirty，驱动标签圆点与 Ctrl+S 保存
          />
        </div>
      </div>

      {/* 底部状态栏 */}
      <StatusBar>
        <span>行 {cursorLine}/{totalLines}</span>
        <span>{encoding}</span>
        <span>{formatBytes(fileSize)}</span>
      </StatusBar>
    </div>
  );
};
```

**Rust 后端：流式读取远程文件**：
```rust
/// Tauri Command: 流式读取远程文件，分块推送到前端
#[tauri::command]
async fn sftp_read_file_stream(
    state: State<'_, SftpManager>,
    host: String,
    path: String,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let sftp = state.get_session(&host).await?;

    // 获取文件大小
    let attrs = sftp.metadata(&path).await.map_err(|e| e.to_string())?;
    let total_size = attrs.size.unwrap_or(0);

    let mut file = sftp.open(&path).await.map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; 64 * 1024]; // 64KB 分块
    let mut received: u64 = 0;

    loop {
        let n = file.read(&mut buf).await.map_err(|e| e.to_string())?;
        if n == 0 { break; }
        received += n as u64;

        let chunk = String::from_utf8_lossy(&buf[..n]).to_string();

        // 推送数据块到前端
        app_handle.emit("sftp-file-chunk", &chunk).map_err(|e| e.to_string())?;
        // 推送进度
        app_handle.emit("sftp-file-progress", serde_json::json!({
            "received": received,
            "total": total_size,
        })).map_err(|e| e.to_string())?;
    }

    app_handle.emit("sftp-file-complete", ()).map_err(|e| e.to_string())?;
    Ok(())
}

/// 大文件策略：超过阈值时只加载头部 + 尾部，或启用按需分页
const LARGE_FILE_THRESHOLD: u64 = 50 * 1024 * 1024; // 50MB

#[tauri::command]
async fn sftp_read_file_paged(
    state: State<'_, SftpManager>,
    host: String,
    path: String,
    offset: u64,
    length: u64,
) -> Result<String, String> {
    let sftp = state.get_session(&host).await?;
    let mut file = sftp.open(&path).await.map_err(|e| e.to_string())?;
    file.set_offset(offset).await.map_err(|e| e.to_string())?;

    let mut buf = vec![0u8; length as usize];
    let n = file.read(&mut buf).await.map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&buf[..n]).to_string())
}
```

**大文件处理策略**：

| 文件大小 | 策略 | 用户体验 |
|----------|------|----------|
| < 1MB | 全量流式加载 | 即时显示，无感知延迟 |
| 1MB – 50MB | 流式加载 + 进度条 | 渐进显示，可边加载边搜索已加载部分 |
| 50MB – 500MB | 分页加载（Monaco 虚拟滚动） | 只加载可视区域 ± 缓冲区，滚动时按需请求 |
| > 500MB | 下载到本地 + SQLite FTS5 索引 | 提示用户下载到本地，导入搜索引擎后全文检索 |

**返回 SFTP 的平顺过渡**：
```tsx
// 从文本查看器返回 SFTP 文件列表
const goBackToSftp = () => {
  // 1. 内容区淡出
  setVisible(false);
  // 2. 200ms 后切换视图
  setTimeout(() => {
    setCurrentView('sftp');          // 切换回文件列表
    // 3. 恢复之前的目录位置和滚动位置
    restoreDirectoryState(savedState);
    // 4. 文件列表淡入
    requestAnimationFrame(() => setListVisible(true));
  }, 200);
};
```

### 3.4 日志搜索模块（核心亮点）

这是本工具最重要的差异化功能，替代 `grep/cat/sed` 的可视化方案。

#### 3.4.1 架构设计

```
┌─────────────────────────────────────────────────┐
│              日志搜索前端 (React)                 │
│  ┌─────────────┬──────────────┬───────────────┐ │
│  │ 搜索条件面板 │ 结果列表     │ 日志详情面板  │ │
│  │ - 关键词    │ (虚拟滚动)   │ (Monaco高亮)  │ │
│  │ - 正则      │ - 匹配行    │ - 上下文行    │ │
│  │ - 时间范围  │ - 文件名    │ - 语法高亮    │ │
│  │ - 文件过滤  │ - 时间戳    │ - 跳转定位    │ │
│  └─────────────┴──────────────┴───────────────┘ │
└────────────────────┬────────────────────────────┘
                     │ Tauri IPC
┌────────────────────┼────────────────────────────┐
│              Rust 后端                           │
│  ┌─────────────────┴──────────────────────────┐ │
│  │            搜索引擎核心                       │ │
│  │  ┌────────────┐  ┌──────────────────────┐  │ │
│  │  │ ripgrep    │  │ SQLite FTS5 全文索引  │  │ │
│  │  │ 实时搜索   │  │ 离线索引搜索          │  │ │
│  │  └────────────┘  └──────────────────────┘  │ │
│  └────────────────────────────────────────────┘ │
│  ┌────────────────────────────────────────────┐ │
│  │         远程日志获取                         │ │
│  │  SSH执行远程命令 → 流式传输 → 本地缓存/索引 │ │
│  └────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────┘
```

#### 3.4.2 双模式搜索

**模式 A：远程实时搜索（ripgrep over SSH）**
- 通过 SSH 在远程 Linux 上执行 `rg` (ripgrep) 命令
- 结果流式传回，前端虚拟滚动展示
- 适合临时搜索、小范围文件

**模式 B：本地索引搜索（SQLite FTS5）**
- 将远程日志目录下载/同步到本地
- 导入 SQLite FTS5 虚拟表建立全文索引
- 支持复杂的布尔查询、模糊匹配、中文分词
- 适合频繁搜索、历史日志分析

**SQLite FTS5 表结构**：

> 注意：`line_number` / `timestamp` / `log_level` / `host_name` 不需要参与全文分词匹配，应标记为 `UNINDEXED`，否则 FTS5 会把数字/时间戳当文本分词，既浪费索引空间也拖慢写入速度；`log_level`、`host_name`、`timestamp` 的过滤更适合走 `WHERE` 后过滤或单独建普通索引表。

```sql
CREATE VIRTUAL TABLE logs USING fts5(
    content,                       -- 日志内容（参与分词）
    file_path UNINDEXED,           -- 来源文件路径
    line_number UNINDEXED,         -- 行号
    timestamp UNINDEXED,           -- 时间戳
    log_level UNINDEXED,           -- 日志级别 (INFO/WARN/ERROR)
    host_name UNINDEXED,           -- 主机名
    tokenize = 'unicode61'
);

-- 搜索示例
SELECT file_path, line_number, snippet(logs, 0, '<b>', '</b>', '...', 20)
FROM logs
WHERE logs MATCH 'error AND timeout'
ORDER BY rank
LIMIT 100;
```

**Rust 核心代码骨架**：
```rust
use rusqlite::{Connection, params};

pub struct LogSearchEngine {
    db: Connection,
}

impl LogSearchEngine {
    pub fn new(db_path: &str) -> Result<Self> {
        let db = Connection::open(db_path)?;
        db.execute_batch("
            CREATE VIRTUAL TABLE IF NOT EXISTS logs USING fts5(
                content, file_path, line_number, timestamp, log_level, host_name,
                tokenize = 'unicode61'
            );
        ")?;
        Ok(Self { db })
    }

    pub fn import_log_file(&self, path: &str, host: &str) -> Result<usize> {
        let mut count = 0;
        let tx = self.db.unchecked_transaction()?;
        for (line_num, line) in std::fs::read_to_string(path)?.lines().enumerate() {
            let (ts, level) = parse_log_line(line);
            tx.execute("INSERT INTO logs VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![line, path, line_num, ts, level, host])?;
            count += 1;
        }
        tx.commit()?;
        Ok(count)
    }

    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>> {
        let mut stmt = self.db.prepare("
            SELECT file_path, line_number, snippet(logs, 0, '<mark>', '</mark>', '...', 30)
            FROM logs WHERE logs MATCH ?1 ORDER BY rank LIMIT ?2
        ")?;
        let results = stmt.query_map(params![query, limit], |row| {
            Ok(SearchResult {
                file: row.get(0)?,
                line: row.get(1)?,
                snippet: row.get(2)?,
            })
        })?.collect::<Result<Vec<_>, _>>()?;
        Ok(results)
    }
}
```

### 3.5 网页浏览模块

- 使用 `<iframe>` 嵌入网页（简单场景）
- 或使用 Tauri 的 `webview` 创建独立 WebView 窗口（复杂场景）
- 支持书签管理、历史记录

**隔离要求（安全关键）**：无论选择哪种方案，加载的第三方网页**绝不能**与承载应用主界面的 WebView 共享 Tauri IPC 上下文——否则一个恶意/被攻陷的网页可以直接调用 `invoke()` 拿到本地文件系统、SSH 凭据等权限。

- 优先使用 Tauri 2.0 的多 `webview` / 子窗口方案，为网页浏览分配独立 WebView 实例，其 `tauri.conf.json` 中不注入 `withGlobalTauri` 且不暴露任何 Command allowlist。
- 若使用 `<iframe>` 内嵌在主 WebView 内，必须设置 `sandbox` 属性（禁用 `allow-same-origin` 与主应用同源）并配置严格的 `Content-Security-Policy`（`frame-src` 白名单），防止内嵌页面通过同源脚本访问宿主页面的 `window.__TAURI__`。
- 两种方案都应默认拦截页面内的下载/协议跳转（如 `file://`、自定义协议）请求，避免被用作本地文件读取的跳板。

### 3.6 AI工具的问答能力

统一 AI工具不依赖模型“自带联网”。当问题涉及最新新闻、实时事实或外部资料时，Agent 必须调用 `web_search`：后端通过 Bing RSS 获取标题、摘要、发布时间和 URL，作为工具结果追加到对话上下文；模型回答必须优先使用这些结果并引用 URL。搜索失败会明确返回工具错误，禁止模型把未搜索到的内容伪装成实时事实。

**支持的模型/API**：
- 豆包（字节跳动）API
- OpenAI / 通义千问 / DeepSeek 等兼容 OpenAI 格式的 API
- 本地 Ollama 模型

**实现**：
- 前端：React 对话界面 + `react-markdown` + `react-syntax-highlighter`（代码高亮）
- 后端：`reqwest` 调用 API，支持 SSE 流式输出
- 可选：嵌入 `Code Server`（VS Code Web）提供编码环境

**数据出境风险（安全关键）**：本工具的核心场景是把生产服务器的日志、配置、代码发给 AI 分析，这些内容大概率包含密码、Token、内网 IP、API Key 等敏感信息。若用户配置的是第三方云端 API（非本地 Ollama），需要：

- 首次向某个云端 Provider 发送"来自 SFTP/日志/终端"的内容时，弹出一次性提示，明确告知数据会发送到该 Provider（可关闭）。
- 内置一个可选的轻量脱敏 pass（正则匹配常见密钥模式：`AKIA[0-9A-Z]{16}`、`-----BEGIN.*PRIVATE KEY-----`、`password\s*=\s*\S+` 等），发送前对高置信度命中项做遮蔽，供用户在设置中开启。
- 每个 Provider 配置需标注"本地"（Ollama）还是"云端"，UI 上用图标做视觉区分，避免用户混淆。

**与工作区类型的关系**：AI 问答模块与工作区是本地还是远程完全无关——它消费的是已经被读进前端内存的文本（打开的文件内容、终端选中输出、日志片段等），这些内容在到达 AI 问答模块之前，读取方式的差异已经被 Explorer/终端/日志搜索模块屏蔽掉了。因此不需要为"远程工作区下的 AI 问答"做任何特殊设计或限制，这与 §3.8.7 中 AI 编程助手需要专门讨论远程能力边界是不同的——AI 问答不主动读写文件系统，AI 编程助手会。

**Rust 核心代码骨架**：
```rust
use reqwest::Client;
use serde_json::json;

pub struct AiChat {
    client: Client,
    api_base: String,
    api_key: String,
    model: String,
}

impl AiChat {
    pub async fn chat_stream(&self, messages: Vec<Message>) -> impl Stream<Item = String> {
        let body = json!({
            "model": self.model,
            "messages": messages,
            "stream": true,
        });
        let resp = self.client.post(format!("{}/chat/completions", self.api_base))
            .bearer_auth(&self.api_key)
            .json(&body)
            .send().await;
        // SSE 流解析...
    }
}
```

### 3.7 Lua 脚本扩展模块

通过 `mlua` crate 嵌入 Lua 5.4 运行时，提供插件扩展能力：

```rust
use mlua::prelude::*;

pub struct PluginEngine {
    lua: Lua,
}

impl PluginEngine {
    pub fn new() -> Result<Self> {
        let lua = Lua::new();
        // 注册 API 到 Lua 环境
        let globals = lua.globals();
        globals.set("search_logs", lua.create_async_function(|_, query: String| async move {
            // 调用日志搜索...
            Ok(results)
        })?)?;
        globals.set("ssh_exec", lua.create_async_function(|_, (host, cmd): (String, String)| async move {
            // 执行 SSH 命令...
            Ok(output)
        })?)?;
        Ok(Self { lua })
    }

    pub fn load_plugin(&self, script: &str) -> Result<()> {
        self.lua.load(script).exec()?;
        Ok(())
    }
}
```

**Lua 插件示例**：
```lua
-- 自定义日志分析插件
local function analyze_errors(host, log_path)
    local results = search_logs(host, "level:ERROR AND timeout", 100)
    local stats = {}
    for _, r in ipairs(results) do
        stats[r.file] = (stats[r.file] or 0) + 1
    end
    return stats
end

-- 注册为工具命令
register_command("error_stats", analyze_errors)
```

### 3.8 AI工具的编程能力（OpenCode 风格）

集成类似 OpenCode / Aider / Codex 的 AI 编程能力，提供可视化 UI 界面，支持本地和远程项目的代码阅读、生成、修改、调试。作为 §3.1.3 定义的顶部快捷工具之一打开，**自动绑定当前工作区**（根目录、本地/远程、连接会话），不再需要用户在打开时手动选择目标。

#### 3.8.1 整体 UI 布局

```
┌───────────────────────────────────────────────────────────────────────┐
│  [🖥 终端] [📄 main.rs]                                [📂][🌐][✨]│
├─────────────┬─────────────────────────────────────────────────────────┤
│             │  ┌─ Plan ─ Build ─┐  目标: web-01(远程) [更改▾]  [模型▼] │
│  Explorer   │  ├───────────────────────┼─────────────────────────────┤
│（工作区树， │  │                       │                             │
│ 与左侧全局  │  │   AI 对话区域          │   代码编辑器（Monaco）      │
│ Explorer    │  │                       │                             │
│ 是同一份）  │  │  🤖 我来分析这个项目…  │   // 实时显示AI正在修改     │
│  📁 src/   │  │  📎 main.rs [已修改] │   fn main() {               │
│  ├ 📄 main │  │                       │       println!("Hello");    │
│  ├ 📄 api  │  │  👤 添加一个API路由   │   }                         │
│  └ 📄 util │  │                       │                             │
│             │  │  🤖 我来添加路由…     │   [Diff视图: +/- 对比]      │
│  变更文件    │  │     [应用✓] [拒绝✗]  │                             │
│  ─────────  │  │                       │                             │
│  ● main.rs │  ├───────────────────────┴─────────────────────────────┤
│  + api.rs  │  │  [📎 添加文件] [📷 截图]  输入消息…         [发送▶] │
│             │  └─────────────────────────────────────────────────────┘
├─────────────┴─────────────────────────────────────────────────────────┤
│  状态: 模型 Claude-3.5 | Token: 2.3k/128k | 已修改 2 个文件 | 远程模式：无智能补全 │
└───────────────────────────────────────────────────────────────────────┘
```

**三栏布局说明**：
| 区域 | 功能 |
|------|------|
| 左栏：项目文件树 | 就是当前工作区的 Explorer 树（§3.1.2），与左侧全局 Sidebar 共享同一份状态，只是额外标注被 AI 修改的文件；不再是"AI 编程助手专属的另一棵树" |
| 中栏：AI 对话 | Plan/Build 模式切换、对话历史、文件变更预览、Apply/Reject 按钮 |
| 右栏：代码编辑器 | Monaco Editor，实时显示AI正在编辑的文件，支持 Diff 对比视图 |

**"目标"从手动选择变为自动绑定+可覆盖**：默认目标就是当前工作区（本地或远程主机+根目录），顶部只显示一行只读的目标提示（远程时用 `--warning` 强调，见 UI_DESIGN.md），不再提供默认展开的下拉选择器；只有点击"更改▾"才能临时把目标切到工作区之外的其他已保存连接（用于"顺手改一下另一台机器"的少数场景），切换后当前对话的后续消息都会明确带上目标提醒，防止误操作。

#### 3.8.2 核心功能设计

**双模式工作流（参考 OpenCode）**：
```
┌─────────────────────────────────────────────────────────┐
│                    Plan 模式                             │
│  • AI 只分析和建议，不修改任何文件                         │
│  • 输出实现方案、文件清单、代码示例                        │
│  • 用户可以反复讨论、调整方案                             │
│  • 按 Tab 切换到 Build 模式                              │
└────────────────────────┬────────────────────────────────┘
                         │ 确认方案后切换
┌────────────────────────┴────────────────────────────────┐
│                    Build 模式                            │
│  • AI 执行文件读写、创建、修改                            │
│  • 每次修改生成 Diff，用户可逐条 Accept / Reject          │
│  • 支持 Undo / Redo（/undo, /redo）                     │
│  • 可执行 Shell 命令（本地或远程SSH）                     │
│  • Git 自动提交变更                                      │
└─────────────────────────────────────────────────────────┘
```

**AI 工具集（Tool Calling）**：

| 工具 | 功能 | 作用范围 |
|------|------|----------|
| `read_file` | 读取文件内容 | 本地 / 远程(SFTP) |
| `write_file` | 创建或覆盖文件 | 本地 / 远程(SFTP) |
| `edit_file` | 精确编辑文件（search/replace） | 本地 / 远程(SFTP) |
| `list_directory` | 列出目录结构 | 本地 / 远程(SFTP) |
| `search_files` | 搜索文件内容（ripgrep） | 本地 / 远程(SSH) |
| `run_command` | 执行 Shell 命令 | 本地 / 远程(SSH) |
| `create_diff` | 生成 Diff 预览 | — |
| `undo_change` | 撤销上一步修改 | — |
| `web_fetch` | 获取网页内容 | — |

#### 3.8.2.1 高危命令确认机制（安全关键）

`run_command` 允许 AI 在本地或远程主机上执行任意 shell 命令，是全工具集中风险最高的一项，不能像 `read_file` 一样静默自动执行：

- **默认策略**：`run_command` 始终需要用户逐条确认（即便在 Build 模式下），除非用户显式为该会话开启"自动执行只读命令"白名单模式。
- **黑名单硬拦截**：无论何种模式，命中破坏性模式（如 `rm -rf /`、`mkfs`、`dd if=... of=/dev/*`、`:(){ :|:& };:`、覆盖 `/etc/passwd` 等）的命令直接拒绝执行并高亮告警，不提供"仍要执行"的一键绕过，只允许用户到终端模块里手动敲。
- **只读白名单可选自动放行**：`ls` / `cat` / `grep` / `git status` 等公认只读命令，用户可选择加入自动放行列表，减少高频确认打断。
- **审计日志**：所有 `run_command` 调用（含被拒绝的）记录到本地 SQLite 审计表（时间、目标主机、命令、执行结果、AI 会话 ID），可在设置中查看/导出，便于事后追责。
- **远程场景加权**：目标为远程主机时确认弹窗需明确展示主机名（防止用户误以为在本地执行），且不提供"记住此主机 24 小时内不再询问"这类会放大风险的选项。

#### 3.8.3 代码骨架

**Rust 后端：Coding Agent 核心**
```rust
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// AI 工具定义（对应 LLM Function Calling）
#[derive(Serialize)]
pub enum AgentTool {
    ReadFile { path: String },
    WriteFile { path: String, content: String },
    EditFile { path: String, old_text: String, new_text: String },
    ListDirectory { path: String },
    SearchFiles { pattern: String, path: String },
    RunCommand { command: String, cwd: Option<String> },
}

/// 文件变更记录
pub struct FileChange {
    pub path: String,
    pub old_content: String,
    pub new_content: String,
    pub diff: String,
    pub status: ChangeStatus,
}

pub enum ChangeStatus {
    Pending,    // 等待用户确认
    Applied,    // 已应用
    Rejected,   // 已拒绝
    Undone,     // 已撤销
}

/// Coding Agent 会话管理
pub struct CodingSession {
    client: Client,
    api_base: String,
    model: String,
    messages: Vec<ChatMessage>,
    changes: Vec<FileChange>,          // 变更历史
    undo_stack: Vec<FileChange>,       // 撤销栈
    project_root: String,              // 项目根目录
    target: SessionTarget,             // 本地 or 远程SSH
}

pub enum SessionTarget {
    Local,
    Remote { host: String, sftp: SftpSession, ssh: SshSession },
}

impl CodingSession {
    /// 处理用户消息，AI 可能会调用工具
    pub async fn chat(&mut self, user_msg: &str) -> Result<AgentResponse> {
        self.messages.push(ChatMessage::user(user_msg));

        loop {
            let response = self.call_llm_with_tools().await?;

            if let Some(tool_calls) = response.tool_calls {
                // 执行工具调用
                for call in tool_calls {
                    let result = self.execute_tool(call).await?;
                    self.messages.push(ChatMessage::tool_result(result));
                }
                // 继续循环，让 AI 看到工具结果
            } else {
                // AI 直接回复，结束
                return Ok(response);
            }
        }
    }

    /// 执行文件编辑并记录变更
    async fn apply_edit(&mut self, path: &str, old: &str, new: &str) -> Result<FileChange> {
        // 1. 读取原文件
        let original = self.read_file(path).await?;

        // 2. 执行替换
        let updated = original.replace(old, new);
        if updated == original {
            return Err("Edit target not found".into());
        }

        // 3. 生成 Diff
        let diff = generate_diff(&original, &updated, path);

        // 4. 写入文件
        self.write_file(path, &updated).await?;

        // 5. 记录变更
        let change = FileChange {
            path: path.to_string(),
            old_content: original,
            new_content: updated,
            diff,
            status: ChangeStatus::Applied,
        };
        self.changes.push(change.clone());
        Ok(change)
    }

    /// 撤销上次修改
    pub async fn undo(&mut self) -> Result<()> {
        if let Some(mut change) = self.changes.pop() {
            self.write_file(&change.path, &change.old_content).await?;
            change.status = ChangeStatus::Undone;
            self.undo_stack.push(change);
        }
        Ok(())
    }

    /// 重做上次被撤销的修改
    pub async fn redo(&mut self) -> Result<()> {
        if let Some(mut change) = self.undo_stack.pop() {
            self.write_file(&change.path, &change.new_content).await?;
            change.status = ChangeStatus::Applied;
            self.changes.push(change);
        }
        Ok(())
    }
}
```

> 注：`undo_stack` 在新的手动编辑发生时应清空（标准编辑器行为——一旦在撤销点之后产生新修改，原有的"重做"分支即失效），否则 `redo()` 可能把已经被覆盖的旧内容写回。

**Rust 后端：MCP（Model Context Protocol）支持**
```rust
use rmcp::{Client as McpClient, tool::Tool};

/// MCP 客户端：连接外部 MCP 服务器获取更多工具
pub struct McpManager {
    clients: HashMap<String, McpClient>,
}

impl McpManager {
    /// 连接一个 MCP 服务器（如 filesystem, database, custom）
    pub async fn connect(&mut self, name: &str, command: &str, args: &[&str]) -> Result<()> {
        let client = McpClient::builder()
            .command(command, args)
            .connect()
            .await?;
        self.clients.insert(name.to_string(), client);
        Ok(())
    }

    /// 列出所有已连接的 MCP 工具
    pub async fn list_all_tools(&self) -> Vec<Tool> {
        let mut tools = Vec::new();
        for client in self.clients.values() {
            if let Ok(t) = client.list_tools().await {
                tools.extend(t);
            }
        }
        tools
    }

    /// 调用 MCP 工具
    pub async fn call_tool(&self, server: &str, tool: &str, args: serde_json::Value) -> Result<String> {
        let client = self.clients.get(server).ok_or("Server not found")?;
        let result = client.call_tool(tool, args).await?;
        Ok(result)
    }
}
```

**前端核心组件**：
```tsx
// CodingAgent.tsx — 编程助手主面板
const CodingAgent: React.FC = () => {
  const [mode, setMode] = useState<'plan' | 'build'>('plan');
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [activeFile, setActiveFile] = useState<string | null>(null);
  const [changes, setChanges] = useState<FileChange[]>([]);
  const [splitView, setSplitView] = useState(false); // Diff 视图开关

  return (
    <div className="coding-agent flex h-full">
      {/* 左栏：项目文件树 */}
      <ProjectTree
        root={projectRoot}
        changes={changes}           // 标注修改过的文件
        onFileClick={setActiveFile}
      />

      {/* 中栏：AI 对话 */}
      <div className="chat-panel flex flex-col flex-1">
        {/* 模式切换 */}
        <div className="mode-tabs">
          <Tab active={mode === 'plan'} onClick={() => setMode('plan')}>
            📝 Plan
          </Tab>
          <Tab active={mode === 'build'} onClick={() => setMode('build')}>
            🔨 Build
          </Tab>
          <ModelSelector />
          <TargetSelector /> {/* 本地 / SSH主机 */}
        </div>

        {/* 对话列表 */}
        <ChatMessages messages={messages}>
          {/* 每条 AI 回复中的文件变更卡片 */}
          {msg.fileChanges?.map(change => (
            <FileChangeCard
              key={change.path}
              change={change}
              onAccept={() => applyChange(change)}
              onReject={() => rejectChange(change)}
              onViewDiff={() => { setActiveFile(change.path); setSplitView(true); }}
            />
          ))}
        </ChatMessages>

        {/* 输入框 */}
        <ChatInput
          onSubmit={handleSend}
          onAttachFile={handleAttachFile}
          onAttachImage={handleAttachImage}
        />
      </div>

      {/* 右栏：代码编辑器 + Diff 视图 */}
      {activeFile && (
        <div className="editor-panel">
          <div className="editor-header">
            <span>{activeFile}</span>
            <DiffToggle active={splitView} onChange={setSplitView} />
            <UndoRedoButtons />
          </div>
          {splitView ? (
            <MonacoDiffEditor
              original={getOriginal(activeFile)}
              modified={getModified(activeFile)}
              language={detectLanguage(activeFile)}
            />
          ) : (
            <MonacoEditor
              value={getFileContent(activeFile)}
              language={detectLanguage(activeFile)}
              options={{ readOnly: mode === 'plan' }}
            />
          )}
        </div>
      )}
    </div>
  );
};
```

```tsx
// FileChangeCard.tsx — AI 修改的文件变更卡片（嵌入对话流中）
const FileChangeCard: React.FC<Props> = ({ change, onAccept, onReject, onViewDiff }) => (
  <div className={cn(
    'file-change-card border rounded-lg my-2',
    change.status === 'applied' && 'border-green-500 bg-green-50',
    change.status === 'rejected' && 'border-red-500 bg-red-50',
    change.status === 'pending'  && 'border-yellow-500 bg-yellow-50',
  )}>
    <div className="flex items-center justify-between p-2">
      <div className="flex items-center gap-2">
        <FileIcon name={change.path} />
        <span className="font-mono text-sm">{change.path}</span>
        <Badge>{change.status === 'applied' ? '✓ 已应用' : change.status}</Badge>
      </div>
      <div className="flex gap-1">
        <Button size="sm" variant="outline" onClick={onViewDiff}>查看 Diff</Button>
        {change.status === 'pending' && (
          <>
            <Button size="sm" variant="success" onClick={onAccept}>✓ 应用</Button>
            <Button size="sm" variant="destructive" onClick={onReject}>✗ 拒绝</Button>
          </>
        )}
        {change.status === 'applied' && (
          <Button size="sm" variant="outline" onClick={onUndo}>↩ 撤销</Button>
        )}
      </div>
    </div>
    {/* 内联 Diff 预览（可折叠） */}
    <Collapsible>
      <DiffPreview diff={change.diff} />
    </Collapsible>
  </div>
);
```

#### 3.8.4 远程项目支持（SSH + SFTP）

当目标为远程 SSH 主机时，所有文件操作自动路由到 SFTP，命令执行路由到 SSH：

```
用户 → AI Agent → Tool Router
                    ├─ 本地模式 → 直接读写本地文件系统
                    └─ 远程模式 → SFTP 读写文件
                                → SSH  执行命令
```

**Rust 后端：统一文件操作接口**
```rust
pub trait FileOps {
    async fn read_file(&self, path: &str) -> Result<String>;
    async fn write_file(&self, path: &str, content: &str) -> Result<()>;
    async fn list_dir(&self, path: &str) -> Result<Vec<FileEntry>>;
    async fn search(&self, pattern: &str, path: &str) -> Result<Vec<SearchResult>>;
    async fn exec_command(&self, cmd: &str, cwd: Option<&str>) -> Result<String>;
}

/// 本地实现
pub struct LocalFileOps;

/// 远程实现（通过 SSH/SFTP）
pub struct RemoteFileOps {
    sftp: SftpSession,
    ssh: SshSession,
}

impl FileOps for RemoteFileOps {
    async fn read_file(&self, path: &str) -> Result<String> {
        let mut file = self.sftp.open(path).await?;
        let mut buf = Vec::new();
        file.read_to_end(&mut buf).await?;
        Ok(String::from_utf8_lossy(&buf).to_string())
    }

    async fn exec_command(&self, cmd: &str, cwd: Option<&str>) -> Result<String> {
        let full_cmd = match cwd {
            Some(dir) => format!("cd {} && {}", dir, cmd),
            None => cmd.to_string(),
        };
        let output = self.ssh.exec(&full_cmd).await?;
        Ok(output)
    }
    // ... 其他方法类似
}
```

#### 3.8.5 新增依赖

**Cargo.toml 新增**：
```toml
# MCP (Model Context Protocol)
rmcp = { version = "0.2", features = ["client", "transport-child-process"] }

# Diff 生成
similar = "2"

# Tree-sitter 语法分析（可选，用于智能代码理解）
tree-sitter = "0.24"

# Git 操作
 git2 = "0.19"
```

**npm 新增**：
```json
{
  "@monaco-editor/react": "^4.6",
  "react-diff-viewer-continued": "^3.4",
  "react-markdown": "^9.0",
  "react-syntax-highlighter": "^15.5"
}
```

#### 3.8.6 参考开源项目

| 项目 | 仓库 | Stars | 参考价值 |
|------|------|-------|----------|
| **OpenCode** | [opencode-ai/opencode](https://github.com/opencode-ai/opencode) | 12k+ | 整体架构、Plan/Build 模式、MCP |
| **Aider** | [paul-gauthier/aider](https://github.com/paul-gauthier/aider) | 42k+ | Diff 编辑策略、多模型适配 |
| **Continue** | [continuedev/continue](https://github.com/continuedev/continue) | 25k+ | IDE 插件 UI、代码上下文管理 |
| **MCP Rust SDK** | [modelcontextprotocol/rust-sdk](https://github.com/modelcontextprotocol/rust-sdk) | — | MCP 协议 Rust 实现 |
| **Lobe Chat** | [lobehub/lobe-chat](https://github.com/lobehub/lobe-chat) | 60k+ | AI 对话 UI、插件系统 |

#### 3.8.7 远程工作区下的能力边界

这是"远程工作区能否用 AI 编程助手"这个问题的完整答案，而不是简单的"支持"或"不支持"：

**完整支持的部分**（得益于 §3.8.4 已有的 `FileOps` trait 抽象，`LocalFileOps`/`RemoteFileOps` 对上层 Agent 逻辑透明）：

| 能力 | 远程工作区下的行为 |
|------|------|
| 读/写/搜索文件 | 通过 SFTP，与 §3.1.4 的本地化编辑模型是同一套读写路径 |
| 执行命令（`run_command`） | 通过 SSH exec，只要远程主机装了对应工具链（如 `cargo`/`pytest`），构建、测试、lint 等反馈闭环都能正常工作 |
| `web_fetch` | 由 Rust 后端直接发起 HTTP 请求，与工作区类型无关 |
| Diff 生成/应用、Undo/Redo | 纯本地内存操作（对比的是已经读入内存的新旧内容），不依赖工作区类型 |

**明确不支持、需要在 UI 上诚实告知的部分**（而不是让用户以为是 bug）：

| 能力 | 为什么远程不支持 | UI 处理 |
|------|------|------|
| 语言服务器级智能感知（跳转定义、实时诊断、语义补全） | 需要在目标机器上运行 LSP 或把整个项目镜像到本地并保持同步，MVP 阶段成本和复杂度过高，不在 Phase 3 范围内 | 编辑器状态栏常驻徽标"远程模式：无智能补全"（见 UI_DESIGN.md），而不是默默没反应；本地工作区不受影响，正常提供 Monaco 内置的语言服务 |
| 未来可能的本地语义/向量检索式代码上下文 | 需要本地索引基础设施，当前方案未包含 | 若后续版本加入，此能力应设计为"本地工作区专属"，远程工作区点击时给出"该功能仅支持本地工作区"的空态提示，而不是报错 |

**性能诚实**：远程模式下每次工具调用（`read_file`/`list_directory`/`run_command`）都有一次 SSH 往返延迟，高延迟链路下一次多步骤 Agent 会话可能明显慢于本地。UI 需要在对话流中显式展示"正在执行工具调用：read_file(src/main.rs) · 已耗时 1.2s"这类进度反馈（对应 UI_DESIGN.md「状态诚实」设计原则），管理用户预期，而不是让界面看起来"卡住了"。工具调用应尽量批量化（如一次 `list_directory` 覆盖后续多个 `read_file` 的需要），减少往返次数。

---

### 3.9 远程工具模式（首页会话树 + RDP）

**背景**：早期版本必须先"打开一个工作区"才能连远程主机，SSH/SFTP 能力完全挂在工作区生命周期下。用户希望 roc_desk 同时能当一个 FinalShell/MobaXterm 式的独立远程工具用——保存一批服务器会话，点开就是终端/文件浏览/远程桌面，不需要先设定"工作区"这个概念。

**首页布局**（`components/RemoteTool/HomeShell.tsx`）：用户原话——"首页应该是左边列出所有会话（可以新建），右边和原版本一样列出所有工作区（可以新建）。不需要去选会话模式和工作区模式后再展现"。因此没有走"模式切换开关"的设计：没有工作区打开时（`workspaceStore.current == null`），首页固定是左侧会话树（`SessionTree`）+ 右侧原样的 `WorkspacePicker`，两者一直同时存在；点开一个会话就在标签栏多一个可关闭标签（"工作区"固定占首位、不可关闭），打开工作区后才切到原有的 IDE 布局（§3.1），和会话标签的状态互不影响、互不清空。

**数据模型**：复用现有 `connections`/`connection_groups` 表，不建平行表——`connections` 加两列 `protocol`（`ssh` | `rdp`）和 `options`（JSON，协议相关的少量额外字段：RDP 是 `domain`/`width`/`height`）。一条 SSH 记录既能"打开终端"也能"打开 SFTP"，RDP 记录只能"打开 RDP"。分组（`connection_groups`）支持任意深度嵌套，树在渲染期从扁平列表按 `parent_id`/`group_id` 现算，不单独维护嵌套 state。

**会话标签**（`stores/remoteSessionStore.ts`，仿 `terminalStore.ts` 的"拍平数组 + 全挂载靠 display 切换"结构）：

| Tab 种类 | 复用的组件 | 说明 |
|------|------|------|
| `ssh-terminal` | `Terminal/TerminalView.tsx` | 与工作区模式下的终端是同一套渲染逻辑，`sshService.openShell` 只要 `profile_id`，从不要求工作区存在 |
| `sftp` | `SftpBrowser/SftpBrowser.tsx` | 同样只依赖 `profileId`，`workspaceId` 参数在这里只是本地记忆 key，传连接自己的 `id` 即可 |
| `rdp` | `RemoteTool/RdpView.tsx` | 见下 |

会话内提供"终端"/"SFTP"快捷跳转按钮（同一 profile 已开的标签直接激活，没有才新开，不会每点一次堆一个新标签）。

**MultiExec 广播输入**（参考 MobaXterm 的 MultiExec）：打开时，任意一个未被排除的 SSH 终端标签收到键盘输入，会广播到其余所有未排除的 SSH 终端标签（`remoteSessionStore.broadcastInput`），用于同时对多台主机敲同一批命令；单个标签可以临时排除在广播范围之外。

#### 3.9.1 RDP 远程桌面

**不自己实现协议**——完整的坑与最终方案见 `src-tauri/src/rdp/mod.rs` 顶部注释，这里只记结论。方向调整的根因依次是：①`rustls` 只支持现代 TLS1.2/1.3 前向保密套件，遇到 SChannel 策略较老的服务器会被直接 RST；②当时的 `IronRDP` 协议库没有实现”并发会话数超限时弹出选择/踢人界面”这个能力（查过源码，`ironrdp-session` 收到 `ServerSetErrorInfo` 就直接断线，没有渲染成 UI 给用户选的路径）；③以 ActiveX 就地激活方式承载系统自带的 `mstscax.dll` 控件——手抄 COM 接口（逐槽位比对类型库确认无误）、免注册 COM 激活上下文、两阶段就地激活、`OnFrameWindowActivate`/`OnDocWindowActivate`、`IOleControlSite`，逐项对照 Devolutions MsRdpEx（一份真实在用的原生 C++ 容器，同一控件）的源码把能做的全做了，`Connect()` 依然同步返回 `S_FALSE` 并立刻自我断开，且这个行为 100% 发生在触碰网络之前（用不存在的服务器地址测试，结果和真实服务器完全一样）。花了一天多用直接调用内部代码的探针排除了 vtable 错位/coclass 选错/容器接口缺失/环境属性/密码设置接口等可能性，仍未定位根因——判断是需要 API Monitor/WinDbg 之类工具实时看 MSTSCAX 内部检查了什么，不是继续”改一处、跑一次、看结果”能猜出来的，用户原话”这个功能搞了一天多了”，决定止损退回验证过的技术路线。

**方案④（曾经的方案）**：拉起系统自带的 `mstsc.exe`，用 Win32 API（`SetParent`/`SetWindowLongPtrW` 摘掉标题栏/边框/系统菜单）把它的窗口嵌进 roc_desk 主窗口——密码经 `cmdkey` 短暂写入 Windows 凭据管理器（DPAPI 加密、绑定当前用户）让 mstsc 免密登录，断开时删除。这条”内嵌外部进程窗口”的路径本身在早期测试中渲染过证书警告对话框（机制可行），修掉了早就诊断出来的一个 bug（见下”持续监视”一节）之后，真机联调复现的现象是：内嵌窗口的句柄/位置/尺寸/可见性全部正常，但内容是纯黑。查证是跨进程 `SetParent` 之后硬件加速渲染窗口的通病：现代 mstsc.exe 用 DirectX/DXGI 画桌面（不是老式 GDI），窗口被”过继”给别的进程时，窗口本身这些”窗口管理器属性”（DWM/USER32 层面）不受影响，但 GPU 合成表面没有正确重新挂到新宿主上。试过`SetParent`后把窗口尺寸”抖动”一下强制触发真正的 `WM_SIZE`（很多 DXGI 应用只在尺寸真的变化时才重建交换链）+ `RedrawWindow` 强制整个窗口树立即重绘，均无效——这类问题的正规解法（Windows”不允许硬件加速解码”策略）要写 `HKLM` 且要管理员权限改全机策略，对场景太重，且从现象看更可能是 DWM 对子窗口和顶层窗口合成 DirectX 内容方式的结构性差异，不是时序问题，判断继续在这条路上试大概率是白费。

**最终方案**：换成 FreeRDP 的 `wfreerdp.exe`。用户参考 Remmina（基于 FreeRDP 内核的开源远程管理器）后原话”参考下这个代码换成FREERDP试下吧”。选它不只是”换一个能连的客户端”：`wfreerdp.exe` 是 FreeRDP 官方已标记为 deprecated 的旧式 Windows 客户端（替代品是新的 SDL 客户端），渲染管线比现代 mstsc.exe 简单、老，大概率不是 DXGI 合成表面那一套，从根上不会撞见上面诊断出的那类跨进程子窗口合成失败。二进制来自 Chocolatey 的 `freerdp.portable` 包（`vendor/wfreerdp.exe`，SHA256 校验过，可追溯到 FreeRDP 官方 CI 按 GitHub 正式发布 tag 打的构建），随应用一起分发（`scripts/build-portable.ps1` 复制进 `bin/`）。密码/用户名直接通过 `/u:`/`/p:` 命令行参数传给它——FreeRDP 不对接 Windows 凭据管理器，不需要 `cmdkey` 那套预置/清理流程，代价是密码会短暂以明文形式出现在这个进程的命令行里（本机其它有权限的进程/管理员能看到），这是接受的已知取舍。窗口查找/内嵌/持续监视这套基础设施和换哪个客户端无关，原样复用。

**持续监视（修的是早就诊断出来的 bug）**：RDP 客户端一个进程生命周期里会先后弹出好几个独立的顶层窗口（证书警告 → 可能的会话选择 → 真正桌面），早期实现只在连接发起那一刻 `EnumWindows` 探测一次，抓到第一个窗口嵌进来后不再追踪——真机实测复现过”证书警告能看到、消失后画面没跟着嵌进来”。现在改成后台线程持续轮询这个 PID 当前的顶层窗口（`watch_and_reembed`，400ms 间隔），一旦和已内嵌的不一样就重新摘边框/`SetParent`/定位。挑”当前该嵌哪个窗口”最初按”有没有标题”过滤，真机联调发现真正渲染桌面的那个窗口标题可能是空的，按标题过滤会永远找不到它、卡在之前嵌入的小窗口上——改成按”这个 PID 名下面积最大的可见窗口”挑，不依赖标题内容，桌面渲染窗口天然比证书警告/连接状态这类小对话框大。

**面板体检**：`rdp_status` 命令报告”眼下嵌的是哪个窗口、是否可见、多大”，前端据此在 connecting/connected/error 之间切换，且 connected 之后诊断信息不整个消失（改成一条细状态条常驻）——嵌入的是外部进程窗口，拿不到 RDP 协议层面的连接状态，真机联调就出现过”状态显示已连接、诊断信息也显示窗口尺寸/可见性都正常，但嵌入区域是一片纯黑”这种情况，如果诊断跟着 status 一起隐藏就只能靠截图来回猜。

**截至本轮会话结束仍未做真机验证**——`wfreerdp.exe` 是否真的避开了 mstsc.exe 那个 DXGI 合成问题，下一次应优先验证。

---

## 四、项目目录结构

> 本节是概览；完整目录树、每个文件的职责、类型签名、Command/Event 目录见 [docs/CODE_DESIGN.md](docs/CODE_DESIGN.md)（随 §3.1 工作区模型改版同步更新，新增 `workspace/`、`fsops/` 等模块）。

```
devhub/
├── src-tauri/                  # Rust 后端
│   ├── Cargo.toml
│   ├── tauri.conf.json
│   ├── src/
│   │   ├── main.rs             # 入口
│   │   ├── lib.rs              # 模块注册
│   │   ├── workspace/          # 工作区模型（§3.1）：档案 CRUD、最近打开列表
│   │   │   ├── mod.rs
│   │   │   └── profile.rs      # WorkspaceProfile
│   │   ├── fsops/               # 本地/远程统一文件操作（原 coding/file_ops.rs 独立出来，
│   │   │   ├── mod.rs           # 供 Explorer、SFTP、AI 编程助手共用，避免各自实现一套）
│   │   │   ├── local.rs
│   │   │   └── remote.rs
│   │   ├── ssh/
│   │   │   ├── mod.rs
│   │   │   ├── session.rs      # SSH 会话管理
│   │   │   └── sftp.rs         # SFTP 操作
│   │   ├── log/
│   │   │   ├── mod.rs
│   │   │   ├── engine.rs       # 搜索引擎核心
│   │   │   ├── importer.rs     # 日志导入
│   │   │   └── remote.rs       # 远程日志获取
│   │   ├── ai/
│   │   │   ├── mod.rs
│   │   │   ├── chat.rs         # AI 对话
│   │   │   └── providers.rs    # 多模型适配
│   │   ├── coding/             # AI 编程助手模块（依赖 fsops，不再自带 file_ops）
│   │   │   ├── mod.rs
│   │   │   ├── session.rs      # Coding 会话管理
│   │   │   ├── tools.rs        # Agent 工具集
│   │   │   ├── diff.rs         # Diff 生成与应用
│   │   │   ├── mcp.rs          # MCP 客户端管理
│   │   │   └── git_ops.rs      # Git 操作
│   │   ├── plugin/
│   │   │   ├── mod.rs
│   │   │   └── lua_engine.rs   # Lua 脚本引擎
│   │   ├── browser/
│   │   │   ├── mod.rs
│   │   │   └── bookmark.rs     # 书签管理
│   │   └── commands/           # Tauri Command 定义
│   │       ├── workspace.rs    # 打开/最近工作区、Explorer 树读写
│   │       ├── ssh.rs
│   │       ├── sftp.rs
│   │       ├── log_search.rs
│   │       ├── ai.rs
│   │       ├── coding.rs       # 编程助手 Commands
│   │       └── browser.rs
│   └── icons/
├── src-web/                    # 前端 (React + TS)
│   ├── package.json
│   ├── vite.config.ts
│   ├── index.html
│   ├── src/
│   │   ├── main.tsx
│   │   ├── App.tsx
│   │   ├── components/
│   │   │   ├── Layout/         # 整体布局（Tab栏+快捷工具+Explorer+内容区）
│   │   │   ├── Workspace/      # 工作区选择页、最近工作区列表
│   │   │   ├── Explorer/       # 工作区文件树（VS Code 式，替代原 Sidebar 连接树）
│   │   │   ├── ConnectionManager/  # 连接档案 CRUD（从工作区选择页/Activity Bar 呼出）
│   │   │   ├── Terminal/       # SSH/本地终端 (xterm.js)
│   │   │   ├── SftpBrowser/    # SFTP 传输管理器（跨目录浏览，见 §3.3）
│   │   │   ├── Editor/         # 通用代码编辑器 (Monaco，可编辑保存，Explorer/AI 共用)
│   │   │   ├── LogSearch/      # 日志搜索面板
│   │   │   ├── WebBrowser/     # 网页浏览
│   │   │   ├── AiChat/         # AI 对话
│   │   │   ├── CodingAgent/    # AI 编程助手（三栏布局，左栏复用 Explorer）
│   │   │   │   ├── ChatPanel.tsx       # AI 对话 + Plan/Build
│   │   │   │   ├── DiffViewer.tsx      # Diff 对比视图
│   │   │   │   └── FileChangeCard.tsx  # 文件变更卡片
│   │   │   └── shared/
│   │   ├── stores/             # Zustand 状态管理（含 workspaceStore, editorStore）
│   │   ├── hooks/              # 自定义 hooks
│   │   ├── services/           # Tauri IPC 封装
│   │   └── styles/             # TailwindCSS 样式
│   └── public/
├── plugins/                    # Lua 插件目录
│   ├── log_analyzer.lua
│   └── auto_backup.lua
├── mcp-servers/                # MCP 服务器配置
│   └── mcp-config.json
├── docs/                       # 文档
└── DESIGN.md                   # 本文件
```

---

## 五、开发阶段规划

### Phase 1：基础框架（3-4 周，比原计划多一周用于工作区模型）
1. 初始化 Tauri 2.0 项目
2. **工作区选择页**（打开本地文件夹 / 连接远程主机选目录 / 最近工作区列表）
3. 搭建前端布局框架（Tab 栏 + 顶部快捷工具 + Explorer + 内容区）
4. **Explorer 文件树**（本地 fs + 远程 SFTP 懒加载，VS Code 式单击预览/双击固定）
5. 实现基础 SSH 连接 + xterm.js 终端（工作区默认打开）
6. **远程文件本地化编辑**（§3.1.4：本地缓冲区编辑 + 保存回写 + 基本冲突检测）
7. SFTP 传输管理器（收窄为跨目录浏览+传输，见 §3.3）

### Phase 2：日志搜索（2-3 周）
1. 实现 SQLite FTS5 日志索引引擎
2. 实现远程日志获取（SSH 执行命令 + 下载）
3. 日志查看器（Monaco Editor 集成）
4. 高级搜索 UI（正则、时间范围、多文件）

### Phase 3：AI 与编程助手（3 周）
1. AI 对话模块（OpenAI 兼容 API）
2. 流式输出 + Markdown 渲染
3. **AI 编程助手三栏布局（左栏复用 Explorer + 对话 + 编辑器），目标自动绑定当前工作区**
4. **Agent 工具集实现（read/write/edit/search/exec），统一走 fsops 而非自建 file_ops**
5. **Plan/Build 双模式 + Diff 预览 + Apply/Reject**
6. **远程项目支持（SFTP + SSH 路由）+ §3.8.7 能力边界的 UI 落地（远程模式徽标、工具调用耗时反馈）**
7. **MCP 协议集成**
8. Lua 插件引擎
9. 网页浏览模块

### Phase 4：打磨（1-2 周）
1. 主题系统（深色/浅色）
2. 配置持久化
3. 快捷键系统
4. 性能优化

---

## 六、构建与运行命令

```bash
# 前置依赖
# 1. 安装 Rust: https://rustup.rs
# 2. 安装 Node.js 18+: https://nodejs.org
# 3. Windows: 确保已安装 WebView2（Win10/11 自带）

# 初始化项目
cargo install create-tauri-app
cargo create-tauri-app devhub --template react-ts

# 开发模式
cd devhub
npm install                    # 安装前端依赖
cargo tauri dev                # 启动开发（前端+后端热重载）

# 构建 Release
cargo tauri build              # 生成 Windows 安装包 (.msi / .exe)

# 运行测试
cargo test                     # Rust 后端测试
npm run test                   # 前端测试
```

---

## 七、跨平台兼容策略

| 平台 | WebView | SSH | 特殊处理 |
|------|---------|-----|---------|
| Windows | WebView2（系统自带） | russh | 无需额外依赖 |
| macOS | WebKit（系统自带） | russh | 需签名/公证 |
| Linux | WebKitGTK | russh | 需安装 webkit2gtk |

Tauri 2.0 已内置跨平台支持，`cargo tauri build` 可自动适配目标平台。
通过 `#[cfg(target_os = "...")]` 处理平台差异代码。

---

## 八、安全性设计

1. **凭据存储**：SSH 密码/私钥口令、AI API Key 等一切机密**必须**经由系统密钥链存储——Windows 走 Credential Manager，macOS 走 Keychain，Linux 走 Secret Service，统一通过 [`keyring`](https://crates.io/crates/keyring) crate 封装，而不是明文写入 `tauri-plugin-store`。
   - 纠正常见误解：`tauri-plugin-store` 本质是磁盘上的明文 JSON 文件，**不提供加密**，不能用于存储密码/Key；它只适合存 UI 偏好、窗口位置等非敏感配置。若需要跨平台一致的"应用级加密保险箱"（而非依赖系统密钥链），可评估 `tauri-plugin-stronghold`。
2. **SSH 主机指纹校验**：见 [3.2.1](#321-主机指纹校验knownhosts--tofu)，禁止无条件信任远程主机公钥，需实现 known_hosts / TOFU 机制。
3. **IPC 安全**：Tauri 2.0 的 Command 需在 `tauri.conf.json` 的 `capabilities` 中显式声明权限范围（Tauri 2.0 已用 capabilities/permissions 模型取代 1.x 的 allowlist），前端 WebView 默认不具备任何原生能力。
4. **WebView 隔离**：主应用 UI 无法直接访问文件系统，必须通过 Tauri Command；第三方网页浏览模块须与主 IPC 上下文物理隔离，见 [3.5](#35-网页浏览模块)。
5. **Lua 沙箱**：`mlua` 默认可访问 `io`/`os` 等标准库，**必须显式裁剪全局表**（移除 `os.execute`、`io.popen`、任意文件读写），仅暴露受控的 `search_logs` / `ssh_exec` 等白名单 API；网络与文件访问一律经宿主 Rust 函数代理并复用同一套权限确认流程。
6. **高危命令确认**：AI 编程助手的 `run_command` 工具需要二次确认与黑名单拦截，见 [3.8.2.1](#3821-高危命令确认机制安全关键)。
7. **AI 数据出境**：向云端 AI Provider 发送的日志/代码/终端内容可能包含密钥、密码等敏感信息，需要脱敏与提示机制，见 [3.6](#36-ai-问答模块)。
8. **审计与可追溯**：远程命令执行、文件写入、连接建立等敏感操作应写入本地审计日志（时间、目标、操作者会话、结果），方便事后排查誤操作或异常行为。

---

## 九、关键依赖版本（Cargo.toml 参考）

```toml
[dependencies]
tauri = { version = "2", features = ["tray-icon"] }
tauri-plugin-fs = "2"
tauri-plugin-shell = "2"
tauri-plugin-store = "2"

# SSH / SFTP
russh = "0.46"
russh-keys = "0.46"
russh-sftp = "0.20"

# 数据库
rusqlite = { version = "0.32", features = ["bundled", "fts5"] }

# 异步
tokio = { version = "1", features = ["full"] }

# HTTP / AI
reqwest = { version = "0.12", features = ["json", "stream"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"

# Lua 脚本
mlua = { version = "0.10", features = ["lua54", "async"] }

# 搜索（可选，嵌入 ripgrep 逻辑）
grep = "0.3"
grep-searcher = "0.1"
grep-regex = "0.1"

# Diff 生成
similar = "2"

# MCP (Model Context Protocol)
rmcp = { version = "0.2", features = ["client", "transport-child-process"] }

# Git 操作
git2 = "0.19"

# Tree-sitter（可选，智能代码分析）
tree-sitter = "0.24"

# 日志
tracing = "0.1"
tracing-subscriber = "0.3"

# 其他
notify = "7"
chrono = "0.4"
uuid = "1"

# 凭据存储（系统密钥链，见「安全性设计」）
keyring = "3"

# 错误处理
anyhow = "1"
thiserror = "2"
```

> **版本提醒**：以上版本号为方案编写时的参考值，实现阶段务必用 `cargo add <crate>` / `cargo search` 复核最新可用版本与 API 是否有破坏性变更（尤其 `russh`、`russh-sftp`、`rmcp` 这类仍在快速演进的 crate），不要直接照抄。

---

## 十、开放风险与待办事项

以下问题在当前方案中尚未有定论，建议在进入 Phase 1 之前明确，避免返工：

1. **单用户 vs 多用户**：文档全篇按单用户桌面场景设计（无登录/权限体系）。如果未来要给团队共享连接档案或做集中审计，数据模型和凭据存储方案需要重新评估。
2. **崩溃恢复**：应用崩溃或被强制关闭后，进行中的 SFTP 传输、日志导入、AI 编程会话的变更历史如何恢复到一致状态，目前未设计（建议：关键写操作前落地 WAL/journal，重启时检测未完成任务并提示用户）。
3. **大文件全文索引的磁盘成本**：FTS5 索引通常是原文大小的 1.5–2 倍，`> 500MB` 策略提到"下载到本地 + SQLite FTS5 索引"，需要补充索引空间预估与自动清理策略（如 LRU 清理最久未查询的日志索引），避免磁盘无限增长。
4. **测试与 CI 策略**：文档未涉及测试金字塔（Rust 单元测试覆盖 SSH/SFTP/搜索引擎核心逻辑、前端组件测试、端到端的 Tauri 集成测试）与 CI 流水线，建议在 Phase 1 就把 `cargo test` / `npm run test` 接入 CI，而不是留到 Phase 4。
5. **自动更新机制**：作为长期运行的运维工具，建议引入 `tauri-plugin-updater` 做签名校验的自动更新，当前方案未提及。
6. **Lua 插件的信任模型**：`plugins/` 目录下的脚本从哪里来（用户手写 / 社区分享）？如果支持导入第三方 Lua 插件，需要明确签名/审核机制，否则「Lua 沙箱」的边界很容易被社会工程绕过（例如诱导用户手动放宽沙箱权限）。
7. **多工作区并行状态**：§3.1.1 提到支持"在新窗口打开第二个工作区"，但 `AppState`（见 CODE_DESIGN.md）目前是进程级单例，多窗口场景下 SSH 连接池、AI 会话等是否要按窗口隔离还是全局共享（如共享同一条到同一主机的 SSH 连接）需要在实现前定案，否则容易出现"关掉一个工作区窗口却把另一个窗口的终端连接带断"的问题。
8. **远程编辑冲突检测的粒度**：§3.1.4 的冲突检测基于 `mtime` 比对，无法覆盖"文件内容没变但 mtime 因为 `touch` 之类操作变了"或反过来"content 变了但文件系统 mtime 精度不够未变化"的边缘情况；如果这个场景在实际使用中造成困扰，后续可以升级为内容哈希比对，当前先用 mtime 是复杂度和收益的平衡取舍。
9. **LSP/语义智能的中长期路线**：§3.8.7 明确了 MVP 阶段远程工作区不支持语言服务器级智能感知，但本地工作区是否在 MVP 就接入 LSP（而不仅是 Monaco 内置的基础语言支持）也未定案；如果本地都不支持，则"远程模式：无智能补全"这个徽标在 MVP 阶段其实所有工作区都适用，需要相应调整措辞，避免暗示"本地都有、只有远程没有"。

---

## 十一、总结

| 维度 | 方案 |
|------|------|
| **应用框架** | Tauri 2.0（Rust + WebView2） |
| **前端** | React 18 + TypeScript + Vite + TailwindCSS |
| **SSH/SFTP** | russh + russh-sftp |
| **终端** | xterm.js |
| **文本/代码编辑** | Monaco Editor |
| **日志搜索** | SQLite FTS5 + ripgrep（嵌入） |
| **AI 对话** | reqwest + OpenAI 兼容 API + SSE |
| **AI 编程助手** | 自研 Agent（Plan/Build 双模式 + Tool Calling + Diff） |
| **MCP 协议** | rmcp (Rust MCP SDK) |
| **Diff 引擎** | similar crate + Monaco DiffEditor |
| **脚本扩展** | Lua 5.4 (mlua) |
| **状态管理** | Zustand |
| **UI 组件** | shadcn/ui + lucide-react |
| **构建工具** | Vite + cargo tauri |
| **凭据存储** | keyring（系统密钥链，非明文 Store） |
| **错误处理** | anyhow + thiserror |

本方案全部使用开源组件，总依赖 **MIT / Apache-2.0** 协议，可自由用于商业项目。CC0-1.0（`notify`）与 ISC（`lucide-react`）亦均为宽松协议，兼容商业使用。

---

## 十二、实现状态

截至 2026-08-18，完整需求逐项对照见 [REQUIREMENTS.md](REQUIREMENTS.md)，此处只列速览，避免两份文档重复维护细节：

| 模块 | 状态 |
|------|------|
| 工作区管理（本地/远程、最近列表、mtime 冲突检测） | 已实现 |
| SSH/本地终端（TOFU、底部停靠面板、多终端 Tab、本地 PTY、默认进工作区目录） | 已实现 |
| SFTP 双栏浏览器（远程/本地对照，拖拽+右键上传下载，含重命名，远程非空目录删除除外） | 已实现 |
| 编辑器（Monaco、多 Tab、GBK/UTF-16 等编码自动探测+手动切换、Explorer 右键菜单、Markdown 预览、Makefile 语法高亮） | 已实现 |
| 全局搜索（左侧目录树，跨文件全文搜索+替换，参考 VS Code） | 已实现 |
| 日志搜索（本地 FTS5 索引 + 远程实时 rg/grep） | 已实现 |
| AI工具（统一右侧入口；OpenAI 兼容 API、Plan/Build、Tool Calling、Diff、安全机制） | 已实现 |
| 网页浏览（主窗口内嵌子 WebView，IPC 隔离 + 历史记录持久化） | 已实现 |
| 打包部署（单 exe、自定义产物目录、数据目录与 exe 同级） | 已实现 |
| Lua 插件 / MCP 客户端 | 未开始 |

本节之上的正文（一～十一）保留原始架构设计，个别小节的骨架代码示例与最终实现有出入时（例如 §3.8.3 "文件改动立即写盘+Undo" 的示例代码，实现改成了"先生成 Diff 待确认、Accept 后才落盘"），以 REQUIREMENTS.md 里记录的实际决策为准，不倒回去改动正文的原始设计描述——保留设计意图的历史记录本身也有价值。
