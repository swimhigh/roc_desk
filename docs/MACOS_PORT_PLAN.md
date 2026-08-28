# roc_desk — macOS 移植方案

> 状态：**方案阶段，未开始实施**。2026-08-29 排查现状后落地这份文档，先记录结论和路径，具体动手时机另定。
>
> **范围声明：这一轮 macOS 支持不包含 RDP 远程桌面功能。** 原因见「三、RDP：明确排除」。其余能力（本地/SSH 工作区、SFTP、日志搜索、AI 编程助手、本地终端）都在移植范围内。

---

## 一、结论先行

现状：**这份代码目前在 macOS 上编译不过**，唯一的结构性卡点是 RDP 模块（`src-tauri/src/rdp/mod.rs` 没有按平台隔离，无条件引用了 Windows 专属的 `windows` crate 和 `tauri::WebviewWindow::hwnd()`）。

好消息是，排除 RDP 之后，其余代码本来就是"本地 Windows + 远程 Linux 主机"两种路径风格混用着写的，对跨平台已经算是照顾过了——本地终端（`portable-pty`）、路径处理、部分平台分支（LibreOffice 转换、本地命令 shell 选择）都已经预留了非 Windows 分支，只是没跑过、没验证过。

真正需要动代码的地方只有三处（应用数据目录、`keyring` feature、RDP 模块隔离），工作量可控。

---

## 二、现状排查明细

### 🔴 结构性卡点：RDP 模块

`src-tauri/src/lib.rs:13` 是裸的 `pub mod rdp;`，没有 `#[cfg(windows)]`。`rdp/mod.rs` 内部：

- 拉起 `wfreerdp.exe`（FreeRDP 的 Windows 客户端），用 `EnumWindows`/`GetWindowThreadProcessId`/`SetWindowLongPtrW`/`SetWindowPos`/`ClientToScreen` 等 Win32 API 把它的窗口摘掉标题栏、嵌入到主窗口里；
- 用了 `tauri::WebviewWindow::hwnd()`——Windows-only 的 Tauri API，其它平台的 Tauri 里根本没有这个方法，属于**编译期硬错误**，不是运行时才炸。

这个模块的实现是文件顶部注释记录的"两天里依次踩了 rustls 套件受限 / IronRDP 缺会话选择能力 / ActiveX mstscax.dll 激活失败 / mstsc.exe+SetParent 黑屏 四条路"之后才落地的最终方案，本质是"套壳嵌入外部进程窗口"，而 macOS/AppKit 没有对应的跨进程窗口过继机制，**这条实现思路在 mac 上完全走不通，不是改改 API 名字能解决的**。

### 🟡 需要按平台分支处理

| 位置 | 问题 | 处理方式 |
|---|---|---|
| `src-tauri/src/lib.rs` `resolve_app_data_dir()` | 数据库/日志固定放在 exe 同级的 `.rock_desk` 目录（为 Windows 便携部署设计）；macOS 上应用装在 `/Applications` 的 `.app` 包里是只读的，没法往 exe 旁边写文件 | 按 `cfg(target_os = "macos")` 改用 `~/Library/Application Support/<bundle-id>/`；这个函数是所有数据路径（DB、日志、sessions/workspaces 子目录）的唯一入口，改一处即可全部生效 |
| `src-tauri/Cargo.toml:57` | `keyring = { features = ["windows-native"] }` 写死，未按 target 拆分 | 拆成 `[target.'cfg(windows)'.dependencies] keyring = {features=["windows-native"]}` + `[target.'cfg(target_os="macos")'.dependencies] keyring = {features=["apple-native"]}`；`credential/keyring_store.rs` 本身用的是平台无关的 `keyring::Entry` API，**代码不用改** |
| `src-tauri/src/lib.rs:13` | `mod rdp` 未隔离 | 改成 `#[cfg(windows)] pub mod rdp;`，并把 `commands` 里注册 RDP 命令、前端调用 RDP 的入口也一并按平台裁掉（见「四、分阶段实施计划」步骤 1） |
| `build-portable.ps1`/`.cmd`（仓库根目录 `scripts/`） | PowerShell 写的 Windows 专属"便携版"打包流程 | mac 不走这套逻辑，改用 `tauri build` 直接产出 `.dmg`/`.app`，属于发布流程重写，不影响应用代码 |
| `tauri.conf.json` | 没有 `minimumSystemVersion`/`entitlements`/签名身份等 macOS 字段 | 本地 `cargo tauri dev` 不受影响；要分发给别人用（尤其是签名+公证）时需要补齐，可以放到"能跑起来"之后再做 |

### 🟢 基本免费（已有跨平台实现，只是没验证过）

- **本地终端** `src-tauri/src/pty/mod.rs`：`portable-pty` 已经封装了 ConPTY vs. Unix PTY 差异；shell 选择已经是双分支：
  ```rust
  #[cfg(target_os = "windows")] CommandBuilder::new("powershell.exe")
  #[cfg(not(target_os = "windows"))] CommandBuilder::new(std::env::var("SHELL")...)
  ```
- **路径处理**：Rust 端（`fsops/local.rs`、`fsops/remote.rs`）和前端（`ExplorerTree.tsx`、`SftpBrowser.tsx`、`utils/previewFile.ts` 等）都已经在同时处理 `\` 和 `/` 分隔符，没有写死盘符假设——这些代码原本就是为了兼容"本地 Windows + 远程 Linux 主机"两种路径风格才这么写的。
- **旧版 Office 转 PDF** `src-tauri/src/fsops/office_convert.rs`：探测 LibreOffice 路径的逻辑已经是 `#[cfg(windows)]`/`not(...)` 双分支，mac 分支目标是 `soffice`（macOS 上是 `/Applications/LibreOffice.app/Contents/MacOS/soffice`），代码在但没跑过。
- **AI 编程助手本地命令执行** `src-tauri/src/coding/session.rs`：`cmd /C` vs `sh -c` 已经是双分支实现。
- **图标**：`src-tauri/icons/icon.icns` 已经存在并在 `tauri.conf.json` 里引用，不用额外生成。
- **窗口管理**：全仓库搜索 `windows::Win32::*`，除 RDP 模块外没有第二处直接碰 HWND 的代码；`browser/`（内嵌子 WebView 浏览面板）用的是 Tauri 跨平台的 `Webview`/`Window::add_child` API，不受影响。

---

## 三、RDP：明确排除

这一轮 macOS 支持**不做 RDP**。原因：

1. 现有实现（拉起 `wfreerdp.exe` + Win32 窗口嵌入）是 Windows 专属方案，macOS 没有对应的跨进程窗口过继能力，无法平移；
2. 要在 mac 上做等价功能，只能重新选型（比如换纯 Rust 的 `iron-rdp` 把画面渲染到 Tauri canvas 里，而不是嵌入外部窗口），这是独立的一块工作量，和"先让核心功能在 mac 上跑起来"不是一回事，不应该互相阻塞；
3. RDP 是"远程工具模式"里的一个可选能力，SSH/SFTP/日志/AI 这些核心场景完全不依赖它。

**后续如果要做**：技术路线建议参考「一、结论先行」之外单独立项调研，不在本方案范围内，届时再补文档。

---

## 四、分阶段实施计划

### 阶段 1：让项目在 macOS 上编译通过（不含 RDP UI）

1. `src-tauri/src/lib.rs`：`pub mod rdp;` → `#[cfg(windows)] pub mod rdp;`。
2. `commands/mod.rs` 里 RDP 相关命令注册、`lib.rs` 的 `invoke_handler![...]` 列表里 `commands::rdp::*` 那几条，按 `#[cfg(windows)]` 包起来（或者保留注册但让 `commands/rdp.rs` 的函数体在非 Windows 下直接返回"当前平台不支持"的 `AppError`——两种做法都可以，前者更干净，后者前端改动更小，具体选哪种到实施时再定）。
3. 前端："远程工具模式"里选协议的地方（`Protocol` 相关 UI，参考 `bindings.ts` 的 `Protocol = "ssh" | "rdp"`），在非 Windows 平台隐藏/禁用 "rdp" 选项——可以用 `@tauri-apps/plugin-os` 或编译期区分不同的前端 bundle，具体方式到实施时再评估。
4. `Cargo.toml`：`keyring` feature 按 target 拆分（见二、表格第 2 行）。
5. 跑 `cargo check --target x86_64-apple-darwin`（或找一台真机/CI runner）验证编译通过。

### 阶段 2：应用数据目录适配

1. `resolve_app_data_dir()` 加 `#[cfg(target_os = "macos")]` 分支，改用 `~/Library/Application Support/<bundle-id>/`（bundle id 取 `tauri.conf.json` 里的 `identifier`）。
2. 确认日志目录、SQLite 数据库、`sessions`/`workspaces` 子目录、AI 编程助手历史记录等都是从这一个根目录派生的（目前看是），改一处即可。
3. 验证：全新安装（没有旧数据）能正常初始化；如果需要从 Windows 迁移数据到 mac（不太可能是真实场景，两边路径体系都不一样），不在本次范围内。

### 阶段 3：实机验证已有的"写了但没测过"分支

逐项过一遍「二、🟢 基本免费」列出的几处双分支代码：
- 本地终端能不能正常起 `$SHELL`；
- 旧版 Office（.doc/.xls/.ppt）转 PDF 预览在装了 LibreOffice 的 mac 上能不能找到 `soffice`；
- AI 编程助手执行本地命令时 `sh -c` 路径是否正常；
- SSH/SFTP 核心链路（这部分理论上完全跨平台，`russh`/`russh-sftp` 本身不依赖操作系统，但没有在 mac 上实测过，需要走一遍连接、文件浏览、编辑保存的完整流程）。

### 阶段 4：打包与分发（视需要再做）

1. `tauri.conf.json` 补 macOS `bundle` 配置：`minimumSystemVersion`、`category`、`hardenedRuntime` 等。
2. 签名 + 公证流程（需要 Apple Developer 账号和证书，这是账号/流程问题，不是代码问题）。
3. 打包脚本：参考 `build-portable.ps1` 的逻辑写一份 mac 等价物，或者直接放弃"便携版"概念、改用标准的 `tauri build` 产出 `.dmg`。

---

## 五、风险与未知项

- **实机验证缺口最大**：这份方案里"🟢 基本免费"的判断都建立在"代码逻辑看起来对"的基础上，没有一处在真实 macOS 环境跑过，阶段 3 完成之前不能认为移植已经完成。
- **`portable-pty`/`russh`/`keyring` 等第三方 crate 的 macOS 支持成熟度**：理论上都官方支持 macOS，但具体版本、权限模型（比如 macOS 钥匙串首次访问会弹系统授权框，UX 上和 Windows 凭据管理器不一样）需要实测确认。
- **前端 UI 里任何"参考 Windows 惯例"写的交互文案/快捷键**（比如菜单里可能出现的 Ctrl 而不是 Cmd）需要单独排查一遍，这份文档没有覆盖前端交互层面的适配，只覆盖了能不能编译、能不能跑通核心功能。
