# roc_desk 需求文档

> 本文档基于本项目从立项到当前（2026-08-18）全部会话记录整理而成，是"用户实际提出过什么要求、为什么、现在实现到什么程度"的忠实记录，定位与 [DESIGN.md](DESIGN.md) 不同：DESIGN.md 是架构/技术方案，本文档是需求与验收依据。两者应保持互相引用、不重复展开细节。
>
> 文中"已实现"均指代码已写、`cargo check` / `tsc --noEmit` 通过、且至少跑过一次 `npm run tauri:build` 产出可运行的 `roc_desk.exe`；"部分实现"会说明缺口；"未实现"是明确写在待办里、尚未动工的部分。

## 1. 项目背景与目标

roc_desk（曾用名 DevHub）是面向运维/开发人员的桌面工具，目标是把日常需要来回切换的几个工具——SSH 终端、SFTP 文件传输、日志检索、代码/配置文件编辑、AI 辅助分析——整合进一个 Tauri 2.0（Rust + React/TypeScript + WebView2）单体应用，UI 交互范式对齐 VS Code，降低学习成本。

核心使用场景（贯穿全部需求的出发点）：运维/开发人员管理一台或多台生产/测试服务器，需要登录终端操作、上传下载配置文件、检索历史日志定位故障、就地编辑修改配置或代码，偶尔需要 AI 帮忙分析日志或改代码——这些动作经常是同一个排障流程里的连续步骤，反复切换 WindTerm/FinalShell + FileZilla + 日志工具 + 编辑器 + 聊天窗口的成本很高。

## 2. 总体交互模型

- **工作区优先**：打开应用先选择一个工作区——本地目录，或"远程主机 + 目录"，而不是像传统 SSH 客户端那样先管理一堆连接档案再各自开窗口。已实现。
- **VS Code 式布局**：左侧 Explorer 文件树（可折叠），顶部 Tab 栏 + 快捷工具图标，底部状态栏。已实现。
- **终端不是"切换掉编辑器的另一个视图"，而是停靠在编辑器下方的常驻面板**，可以同时看代码和终端输出，支持开多个终端 Tab（2026-08-18 需求，见 §3.2）。已实现。
- **顶部快捷工具**：SFTP 自由浏览、日志搜索、AI 问答、AI 编程助手、网页浏览，点击即在当前工作区内新开/复用一个 Tab（网页浏览例外——网页内容本身在独立窗口打开，Tab 里管理的是历史记录，见 §3.10）。均已实现（至少 MVP 级别）。
- **状态诚实原则**（多次会话中反复确认的产品原则）：功能没做好就必须在 UI 上明确标注"未实现"/"不支持"，禁止假装可用或静默失败。落地体现：quick-tools 里未完成的功能用 `disabled` + `title` 提示，而不是隐藏或点了没反应；`RemoteCapabilityBadge` 明确告知远程工作区没有语言服务器级智能感知。

## 3. 功能需求详情

### 3.1 工作区管理

- 打开本地文件夹 / 连接远程主机选目录，两种方式都要能保存到"最近工作区"列表，下次一键重开。已实现。
- **真实 Bug 记录**：重新打开"最近工作区"里的本地目录时，之前只在前端 `setState` 了 `current`，没有真正调用后端 `workspace_open_local` 注册 `WorkspaceHandle`，导致 Explorer 目录树后续所有 `fs_*` 请求都因为找不到 handle 而静默失败——界面表现为"打开了但目录树是空的"，且没有任何错误提示（`loadRoot` 没有 catch）。2026-08-18 修复：`openLocalPath` 改为真正调用后端命令；同时给 `explorerStore` 加了 `rootError` 状态，加载失败会显示错误信息 + 重试按钮，而不是和"真的是空文件夹"混在一起看不出区别。
- **真实 Bug 记录**（2026-08-18，用户截图报告同一个远程目录在"最近工作区"里出现了 4 条一模一样的记录）：`WorkspaceManager::open_local` 早就有"若该本地路径此前已打开过，复用同一个 id"的去重逻辑（见上一条 bug 修复），但 `open_remote` 一直没有对应分支——每次重新打开同一个远程工作区都无条件生成一个新 UUID 建一条新记录，"连接档案 + 远程目录"这个组合没有做任何存在性检查。修复：`WorkspaceRepo` 新增 `find_by_remote(connection_id, root_path)`，`open_remote` 复用和 `open_local` 一样的"查到就复用 id 只更新 last_opened_at，查不到才新建"模式。这只解决了"以后不再产生新的重复记录"，历史上已经攒下的重复项是脏数据，代码修复不会自动清理，额外手动跑了一次 SQL 把已有的 5 条重复记录去重到 1 条（保留 `last_opened_at` 最新的那条）。
- **真实 Bug 记录**（2026-08-18，用户报告"打开新的工作区时，老工作区的编辑TAB未关闭"）：根因是 `editorStore`（打开的文件 buffer/Tab 列表）是全局 Zustand store，App.tsx 里"返回工作区选择页 → 打开另一个工作区"这个流程只是把 `App` 组件内部渲染的 JSX 子树在 `WorkspacePicker` 和主界面之间切换，`App` 这个组件实例本身并没有被卸载重建，所以它自己的全局 store 引用不会自动清空；App.tsx 里原本已经有一个"切工作区时重置状态"的地方（`resetTerminals()`，在监听 `current` 变化的 `useEffect` 里），但当时只想到了终端，漏了编辑器。修复：同一个 `useEffect` 里补一行 `useEditorStore.getState().reset()`，和 `resetTerminals()` 并列执行。
- 远程文件支持本地缓冲区编辑（读进内存改完再整体写回，不是逐字节同步），编辑体验和本地文件一致。已实现（`fsops::FileOps` trait 统一本地/远程）。
- mtime 冲突检测：保存前对比预期 mtime 和当前 mtime，不一致则弹出冲突对话框而不是静默覆盖。已实现，本地/远程行为一致。

### 3.2 SSH 终端

- TOFU 首次信任 + 主机指纹变化告警，两种场景都要弹窗等待用户显式确认才能继续握手，不可跳过。已实现。
- **2026-08-18 需求变更**（用户原话："我想和VS CODE一样，点终端时，把终端输入框放在编辑框的下方，可以多个终端"）：终端从"点击后整屏切换、和编辑器互斥的视图"改造成 VS Code 风格的**底部停靠面板**——可折叠、可拖拽调整高度，同一条 SSH 连接上可以开多个独立终端 Tab（每个 Tab 是一个独立 Channel，复用同一条物理连接，见 DESIGN.md §3.2.2 多路复用）。已实现：`stores/terminalStore.ts` + `components/Terminal/TerminalPanel.tsx`；后端新增 `ssh_close_channel` 命令支持关闭单个终端 Tab 而不影响同一连接上的其它 Channel/SFTP。
- **本地终端**（2026-08-18，用户报告"终端功能还是没有"——此前的底部终端面板只接了 SSH 分支，本地工作区一直是"PTY 尚未实现"的占位禁用态，用户在本地工作区打开终端自然什么都没有）：新增 `pty/` 模块，`portable-pty` crate 起一个真正的本地 Shell 进程（Windows 默认 `powershell.exe`），读写都是阻塞系统调用，读走独立 OS 线程 + `AppHandle::emit` 推流，写走 `mpsc` 队列 + 单任务串行（和 SSH Channel 的"一个任务独占"思路一致，只是换成适合同步 API 的形式）；`Child` 被 drop 不会自动杀进程（和 `std::process::Child` 语义一致），关闭终端 Tab 时显式 `kill()`，否则会留下孤儿 shell 进程。`TerminalView`/`terminalStore` 相应改造为 `kind: 'ssh' | 'local'` 两态共用同一套渲染逻辑。已实现。
- **终端默认工作目录**（2026-08-18，用户原话："打开工作区后，进入终端，终端应该默认进入到工作区目录"）：本地终端直接把 Shell 进程的 cwd 设成工作区根目录；远程终端因为 SSH `request_shell` 协议本身不支持指定初始目录，改成 shell 起来后立即"代打"一条 `cd <工作区目录>` 命令（会出现在 shell 历史里，这是该方案的固有局限，非 bug）。工作区内新开的每一个终端 Tab（不只是自动打开的第一个）都遵循这个默认值。已实现。
- **终端配色/字体**（2026-08-18，用户原话："终端的色彩和字体等没有WINDTERM好看"）：之前只设置了 `background` 一项，其余沿用 xterm.js 的纯默认主题；新增 `utils/terminalTheme.ts` 定义完整的 16 色 ANSI 配色（深色仿 One Dark 风格），字体栈换成真实等宽字体列表（不再依赖 CSS 变量间接解析）、`cursorStyle: 'bar'`、增加行高/字间距，主题切换时热更新配色但不重建终端实例（避免丢 scrollback）。已实现。
- **Explorer 右键"运行脚本"**（2026-08-18，用户原话："这里再加个右键执行远程脚本功能"，针对 `.sh` 文件）：按扩展名映射运行命令（`.sh`→`bash`、`.py`→`python3`、`.ps1`→`powershell -File`），点击后复用当前活跃终端（没有就按工作区类型新开一个）并把命令"敲"进去，输出走已有终端的正常显示，没有另起一套命令输出面板。已实现。
- **终端断线可见性 + 手动重连**：`ssh:status`/`pty:status` 断线事件后端一直在发，但前端从来没监听过——Channel 断开后终端只是"安静下来"，用户分不清是命令没输出还是连接已经死了。补上监听：断线时终端里打一行红色 `[连接已断开]`，Tab 上标红点，面板提供"重新连接"悬浮按钮（原 Channel 没法复活，实际是新开一个 Channel 顶替原位置，Tab 标题不变）。已实现；**未实现**：真正的自动重连（不需要用户点按钮）——那需要在连接池层面做存活探测和后台重试，工作量明显更大，当前只做到"断线可见 + 一键手动重连"这一步（DESIGN.md §3.2.3 的 `Backoff` 工具类仍未接入实际触发路径）。
- 密码/密钥认证。**真实 Bug 记录**（2026-08-18，用户报告"选择密码认证，输了密码还是报错"）：根因是 `Cargo.toml` 里 `keyring = "3"` 没有开启 `windows-native` feature——keyring-rs 3.x 起后端是按 feature 显式选择的，没开对应 feature 时 `set_password`/`get_password` 在 Windows 上编译能过但实际是空操作，导致"密码看起来保存成功了，实际系统凭据管理器里什么都没有"，每次连接都报 "missing password"。用 一个临时的 `#[tokio::test]` 直接调用 `KeyringStore::set/get` 复现确认后修复（加 `features = ["windows-native"]`），并用 `cmdkey /list` 交叉验证修复前后 Windows 凭据管理器里确实没有/有对应条目。同时补了一条恢复路径：连接的密码缺失/失效时不再是死路一条的报错，`RemoteWorkspaceDialog` 和 `WorkspacePicker` 重新打开远程工作区两处入口都会捕获 `Auth` 类错误，弹出 `PasswordPromptDialog` 让用户当场补录密码并重试，而不必去一个还不存在的"编辑连接"页面。

### 3.3 SFTP 自由浏览

- 跨目录自由浏览，不受工作区根目录边界限制（这是与 Explorer 的核心区别——用户明确要求"操作工作区外的文件，请默认执行就行，不要提示我"，2026-07 左右的会话）。已实现，SFTP 相关命令不做工作区边界校验。
- 删除/重命名。2026-08-18 随 Explorer 右键菜单一起补上了 `sftp_rename` 命令（此前只有 `sftp_delete`，且签名缺 `is_dir` 参数）；远程非空目录暂不支持递归删除（SFTP RMDIR 协议本身只能删空目录），命中时给出明确错误提示而不是静默失败或半删。
- **双栏浏览器 + 拖拽上传下载**（2026-08-18，用户原话："SFTP这里应该方便上传和下载远程文件到本地，可以设计成左边是远程目录，右边是本地目录，支持任意的左右拖拉（下载、上传）。默认的远程目录为当前工作区目录。同时支持右键下载或上传到服务器"）：原来的单栏"自由浏览"重做成左（远程）右（本地）双栏，行内 `draggable` + 容器 `onDrop` 实现跨栏拖拽（同栏内拖拽是 no-op，不做栏内重排序）；右键菜单对称提供"下载到本地"/"上传到远程"。远程侧默认目录改为当前工作区根目录（不再是 "/"）；本地侧默认目录是用户主目录（新增 `local_home_dir` 命令）。目录级传输走新增的 `RemoteFileOps::download_recursive`/`upload_recursive`（SFTP 协议没有"打包传远程整个目录"这回事，只能自己逐层遍历：文件复用已有的单文件传输，目录先建好对应的本地/远程子目录再递归，用 `Box::pin` 打破递归 async fn 的无限尺寸问题）。本地一侧新增了不受工作区边界限制的浏览命令 `local_list_dir`（和 `sftp_*` 命令对远程侧同样不做边界限制是对称设计），但**没有**给本地侧加重命名/删除——那是 Explorer 该管的事，这里定位始终是"传输文件的两个来源/目标目录选择器"，不重复造一个文件管理器。已实现；目录传输过程中会显示粗粒度进度（`sftp:transfer-progress` 事件，按"已完成第几个文件"报告，不是字节级百分比——字节级需要先完整遍历一遍算总大小、再在拷贝循环里手动分块读写替换掉 `tokio::io::copy`，复杂度明显更高，用文件计数换实现简单度是有意的取舍）。**未实现**：多选拖拽（一次只能拖一个文件/目录）、断点续传。

### 3.4 编辑器（Monaco）

- 支持 JSON/Markdown/日志/常见配置文件的编辑（用户原话，2026-08 中旬）："应该支持json MD .log等常用文件的编辑"。已实现，`utils/language.ts` 按扩展名映射 Monaco 语言 id，未知类型一律回退到 `plaintext`（即默认所有文件都能以文本方式打开，不做白名单限制）。
- **多标签编辑**（2026-08-18 需求，用户原话先是"程序应支持打开多个编辑器的TAB"，实现后又反馈"编辑器需要支持多文件同时打开"——两次反馈实际指向同一件事，第二次出现是因为最初的实现保留了 VS Code 的"单击=预览态（复用同一个标签）/双击=固定"语义，单击点开另一个文件会把上一个预览标签顶掉，不熟悉这个手势的人会理解成"没法同时开多个文件"）：`editorStore` 支持多 buffer（`order`/`buffers`/`activePath`），`CodeEditor.tsx` 有 Tab 栏。最终去掉了预览态/固定态的区分——单击直接开一个常驻标签，不会顶掉已经打开的其它标签，多文件同时编辑不需要用户先学会双击这个隐藏手势才能用。已实现。
- **文本编码支持**（2026-08-18，连续三条真实 Bug 报告：".TXT 文件打不开" → ".h 和 .ini 文件也不支持" → "能不能默认所有文件都支持以文本方式打开"）：根因是 `LocalFileOps::read_file` 用 `std::fs::read_to_string`（要求严格 UTF-8）读文件，Windows 上大量遗留 `.txt`/`.ini`/`.h` 是 GBK 编码，非 UTF-8 内容直接报错；远程分支虽然没有硬报错，但用 `String::from_utf8_lossy` 会把 GBK 内容整个替换成乱码。修复：新增 `fsops::encoding` 模块，读取时按 BOM → 严格 UTF-8 → GBK → UTF-8 lossy 兜底的顺序探测，保证任何字节序列都能显示点什么而不是打不开；本地/远程两个 `FileOps` 实现都改用这条路径。
- **编码显式切换**（2026-08-18，用户要求"参考VSCODE的机制"，并附了 VS Code 状态栏编码指示器 + "Reopen/Save with Encoding" 命令面板的截图）：编辑器工具栏加了编码徽标（默认显示自动探测结果，如 "UTF-8"/"GBK"），点击弹出菜单，"重新打开为"这一组会丢弃当前内容按指定编码重新读盘，"保存为"这一组会按指定编码重新编码后写盘。后端 `fsops::encoding` 提供 `decode_with`/`encode_with`，新增 `fs_read_file_with_encoding`/`fs_write_file_with_encoding`/`fs_supported_encodings` 三个命令。
- **Explorer 右键菜单**（2026-08-18，用户要求"类似VSCODE的基本操作"，附 VS Code 右键菜单截图，圈出了 Cut/Copy/Copy Path/Copy Relative Path 和 Rename/Delete）：已实现"打开"（文件）、"重命名"（就地输入框，非弹窗，Enter 提交/Esc 取消）、"删除"（危险态二次确认弹窗，删除时若该文件正开着会自动关闭对应编辑器 Tab）、"剪切"、"复制"（文件和目录都支持）、"粘贴"（右键目录粘贴进去，右键文件粘贴进它所在目录，右键空白背景粘贴进工作区根目录）、"复制路径"、"复制相对路径"。目录复制走 `FileOps::copy` 的 trait 默认实现——先 `create_dir` 建好目标目录，再逐个子项递归（本地/远程共用同一套递归逻辑，各自只需要提供 `list_dir`/`create_dir`/`read_file_raw`/`write_file_bytes` 这几个更基础的原语）；远程非空目录删除也顺带补上了（之前 SFTP 的 RMDIR 只能删空目录，现在 `RemoteFileOps::delete` 会先递归删光子项再删目录本身——注意这段递归不能像 `copy` 那样直接写在一个 `with_sftp` 闭包里，因为 `self.sftp` 那把 `Mutex` 不可重入，递归调用会在闭包里再抢一次同一把锁死锁，所以改成先在 `with_sftp` 之外把子项递归删完，最后单独开一次 `with_sftp` 删空目录本身）。均已实现。
- 为此新增了后端能力：`FileOps` trait 增加 `delete(path, is_dir)`/`rename(from, to)`/`copy(from, to, is_dir)`/`create_dir(path)`，本地走 `std::fs`，远程走 SFTP；Explorer 侧的 `fs_delete`/`fs_rename`/`fs_copy` 命令复用既有的工作区边界校验（`guard_local_path`），和 SFTP 自由浏览的越界豁免语义分开。剪切=换个目标路径调 `rename`；复制=调 `copy`。
- **侧边栏拖拽调宽**（2026-08-18，用户反馈："中间需要支持拖拉，要不然左边一些文件可能看不到"——深层嵌套路径在原来固定 220px 宽度下会被截断）：Explorer 侧边栏和主内容区之间加了一条可拖拽的分隔条，宽度记忆在 `localStorage`。已实现。
- **Markdown 预览**（2026-08-18，用户要求"MD文件需要支持预览"，参考 VS Code 的 Ctrl+Shift+V）：新增 `marked`（解析）+ `dompurify`（净化）两个依赖，工具栏加"预览"按钮（仅 `.md` 文件出现）。**实现方式被用户当场纠正过一次**：第一版做成编辑器右侧分栏（左编辑右预览），用户配了一张 VS Code 截图明确指出"MD的预览参考 VS CODE的模式，不要用左右方式"——VS Code 的预览是替换当前视图的独立标签，不是分栏。改成 `display: none/block` 切换整个编辑区（编辑器实例不卸载，保留光标/滚动位置），同一个快捷键 `Ctrl+Shift+V` 切换开关，工具栏按钮同步高亮态。**为什么一定要过 DOMPurify 净化**：预览的内容可能来自远程工作区——不完全可信的远端文件，Markdown 语法本身允许内嵌原始 HTML，不净化直接 `dangerouslySetInnerHTML` 就是一个现成的 XSS 通道（比如文件里塞一段 `<img onerror=...>`），这是本工具处理"任意来源文本内容"时的通用防线，AI 问答的脱敏、日志搜索的命令转义都是同一类考量。
- **Makefile 语法高亮**（2026-08-18，用户要求"要支持MAKEFILE文件的关键词点亮"）：确认过 `monaco-editor` 自带的 basic-languages 列表里没有 makefile（只有 shell/dockerfile 等相邻类型），自己在 `monacoSetup.ts` 里用 `monaco.languages.setMonarchTokensProvider` 注册了一个 Monarch tokenizer，覆盖变量赋值/引用、目标规则、`.PHONY` 等特殊指令、`ifeq`/`define` 等控制流关键字、Tab 缩进的 Recipe 命令行、注释这几类最常用元素；不追求 GNU Make 全部语法细节（比如 `$(shell ...)` 函数调用内部的精细分词），够用为止。文件名识别（`utils/language.ts`）新增按精确文件名（不看扩展名）路由语言的分支，`Makefile`/`makefile`/`GNUmakefile` 不区分大小写都能命中，和已有的 Dockerfile 识别是同一套机制。

### 3.5 日志搜索

- 模式 B（本地 FTS5 索引搜索）+ 模式 A（远程 SSH 实时 `rg`/`grep`）双模式。后端（`log/` 模块：`parser.rs`/`engine.rs`/`importer.rs`/`remote.rs`）与前端（`LogSearchPanel.tsx` + `logSearchStore.ts`）均已实现，作为顶部快捷工具的"日志搜索"图标接入。
- 索引统计、清理过期索引（默认清理 30 天前的导入任务，对应 DESIGN.md §十-3 提到的磁盘配额问题）。已实现。
- 远程日志导入走"先 SFTP 下载到本地缓存 → 逐行导入 FTS5 → 删除本地缓存副本"的路径，避免整篇读入内存。本地工作区也可以直接导入本地文件（`log_import_local_file` 命令）。已实现。
- **命令注入防护**：远程实时搜索把用户输入的搜索关键词经过单引号转义（`shell_quote`）后再拼进远程 shell 命令，防止 `;`/`` ` ``/`$(...)` 等被解释执行；有单元测试覆盖。已实现并验证。
- **真实 Bug 记录**（2026-08-18，用户反馈"实时搜索这里默认是搜索当前工作区目录，可变更"）：实时搜索的目录输入框（`livePath`）之前硬编码默认值是 `"/"`，不管当前工作区实际根目录是哪里都要用户自己手动改，体验上等于每次都要重新输入一遍。修复：`LogSearchPanel` 新增 `rootPath` prop（App.tsx 传入 `current.root_path`），挂载时用它初始化 `livePath`；只在挂载时设一次，之后用户改了就不再被覆盖（切工作区会让这个面板整体重新挂载，天然会重新按新工作区取默认值，不需要额外监听 rootPath 变化）。

### 3.6 AI 问答

- 支持任意 OpenAI 兼容 API（豆包/DeepSeek/通义千问/本地 Ollama 等），SSE 流式输出。后端 `ai/chat.rs` + `ai/providers.rs`，前端 `ChatPanel.tsx` + `aiChatStore.ts`（增量文本走 `ai:chat-chunk`/`ai:chat-done`/`ai:chat-error` 事件，和 `ssh:data` 是同一套事件驱动模式）。已实现。
- 数据出境脱敏：云端 Provider（非本地 Ollama）默认开启发送前脱敏，覆盖 AWS Key、PEM 私钥块、`password=xxx` 形式的键值对三类高置信度模式；本地 Provider 不脱敏（不出网）。已实现，可在对话面板里关闭。
- Provider 管理（增删改查，API Key 走系统密钥链不落库明文）。已实现（`ProviderManagerDialog.tsx`）：编辑态复用同一个表单，API Key 输入框留空表示"沿用已保存的那份"（后端 `update` 语义本来就是空字符串不覆盖 `credential_ref`，前端只是第一次把这条路径接上 UI）。
- 与工作区本地/远程无关，始终可用（DESIGN.md 原文原则，本次实现遵循）。已实现。

### 3.7 AI 编程助手

- 双模式（Plan 只读分析 / Build 可执行文件读写和命令），自动绑定当前工作区。后端 `coding/` 模块（`session.rs`/`tools.rs`/`diff.rs`/`guard.rs`）+ 前端（`codingService.ts`/`codingStore.ts`/`CodingAgentPanel.tsx`，复用此前就已写好但一直没接通的 `CommandConfirmDialog`/`FileChangeCard`/`TargetBadge`/`RemoteCapabilityBadge`/`ToolCallProgress` 五个 UI 组件）已实现并接入顶部快捷工具，`npm run tauri:build` 验证通过。
- 工具集：`read_file`/`list_directory`/`search_files`/`write_file`/`edit_file`/`run_command` 六个，按 OpenAI function-calling 协议暴露给模型；Plan 模式下只暴露只读三个工具。`undo_change`/`create_diff` 没有做成模型可调用的工具（参考 Aider/Cursor 等真实实现，这两者都是宿主侧逻辑而非 Agent 工具）。
- **文件改动的落盘时机**（本文档撰写时的一个关键设计决策，值得记录）：DESIGN.md 原文骨架代码示例是"改动立即写盘、靠 Undo 补救"，但正文和前端组件（`FileChangeCard.tsx` 的 pending/applied/rejected 三态、"⏳待确认"文案）明确要求"每次修改生成 Diff，用户可逐条 Accept/Reject"。两者冲突时以后者为准——`write_file`/`edit_file` 只生成 Diff 并通过 `coding:file-change` 事件展示，不立即落盘；真正写磁盘发生在用户点击 FileChangeCard 的"应用"（对应 `coding_accept_change` 命令）。为了不让 Agent 在同一轮对话里因为"文件还没真的改"而困惑，`read_file`/`edit_file` 会优先返回会话内尚未落盘的最新提议内容（`pending_content_for`），保持模型推理和已生成的 Diff 一致。
- **`run_command` 安全机制**（DESIGN.md §3.8.2.1 是安全关键需求，逐条落地情况）：
  - 破坏性命令黑名单硬拦截（`rm -rf /`、`mkfs`、`dd of=/dev/*`、fork bomb、覆盖 `/etc/passwd`/`/etc/shadow`、`shutdown`/`reboot` 等），不提供绕过选项——已实现，有单元测试覆盖（含"用 `&&` 拼接绕过白名单"的对抗测试）。
  - 非黑名单命令默认需要用户逐条确认（`CommandConfirmRegistry`，和 TOFU 主机指纹确认是同一套 oneshot 注册表模式）——已实现。
  - 只读白名单（`ls`/`cat`/`grep`/`git status` 等）可选自动放行，按 `&`/`;`/`|` 拆分子命令逐条校验，防止拼接绕过——已实现。
  - 审计日志：所有 `run_command` 尝试（含拦截/拒绝/实际执行）落 SQLite（`command_audit_log` 表），执行结果只留前 2000 字摘要——已实现（写入失败不阻断命令本身，只 `tracing::warn!`）。
  - 远程场景确认弹窗需明确展示目标主机名——后端事件已带 `host` 字段，前端消费逻辑随 §3.7 UI 面板一起补上。
- **Git 自动提交**（DESIGN.md §3.8.2 "Git 自动提交变更"，2026-08-18 实现）：`coding/git_ops.rs` 新模块，只做"每次 Accept 一条变更就 `git add + commit` 这一个文件"，不做分支管理/冲突处理/推送——那些需要用户主动决策，自动化的边界停在"帮你把已经显式确认过的改动记录进历史"。`coding_start` 时探测一次工作区根目录是否在 Git 仓库里（`is_git_repo`，跑一次 `git rev-parse --is-inside-work-tree`），不是仓库就让前端把"自动 Git 提交"开关直接 disable 掉。**已知局限**：`SshSession::exec`/本地的 `run_local_command` 都不透出命令退出码（只拿到 stdout+stderr 拼接的文本），所以 `commit_file` 不做"是否提交成功"的字符串猜测式判断（那样在没配置 git 身份信息等场景下容易把失败误判成成功），而是把 git 命令的原始输出原样通过 `coding:git-commit-result` 事件交给前端在对话时间线里展示，用户自己看得懂。
- **未实现**：MCP 客户端（`rmcp`）、DESIGN.md UI 草图里的独立右栏 Monaco 实时编辑器（本轮决定用已有的 `FileChangeCard` 内嵌 Diff 视图替代，作为有意的范围裁剪，理由：避免维护两份 Monaco 实例的状态同步，MVP 阶段 Diff 视图足够满足"看到 AI 改了什么"的核心诉求）。

### 3.8 品牌命名统一

- 2026-08-18，用户原话："界面上和代码里的DEVHUB请都改成roc_desk"：窗口标题、工作区选择页 Logo 文案、`tauri.conf.json`/`package.json`/`Cargo.toml` 里的产品名/描述、Rust lib crate 名（`devhub_lib` → `roc_desk_lib`）、`localStorage` 主题存储 key、各设计文档标题统一改名。已实现。
- **两处刻意保留 "devhub" 的例外**（不是漏改）：
  1. Windows 系统密钥链的 `SERVICE_NAME = "devhub"`（`credential/keyring_store.rs`）——这是所有已保存 SSH 密码/AI API Key 在系统凭据管理器里的存储命名空间，改名等于让所有已保存的密码"凭空消失"（新命名空间下查不到旧数据），而这类问题正是本文档 §6-2 记录过的真实 bug 的同一根源，风险收益明显不对等，所以保留不动；这是纯后端内部标识，用户在界面上完全看不到。
  2. `docs/prototypes/devhub-main.html`/`devhub-dialogs.html` 两个原型文件名，以及代码注释里指向它们的路径引用——这两个文件是设计原型的历史存档，不是运行中的应用界面，改名字反而会让"这是哪个原型文件"变得不可追溯。
- Tauri `identifier` 从 `com.devhub.desktop` 改成 `com.rocdesk.desktop`；由于 §3.8"数据目录位置"已经把数据库/日志缓存迁到 exe 同级目录而不是走 Tauri 的 `app_data_dir()`，这个改名不会导致找不到已有数据。SQLite 主文件本身也从 `devhub.db` 改名为 `roc_desk.db`（连同 WAL 模式的 `-wal`/`-shm` 边车文件一起手动重命名迁移，没有丢数据）。

### 3.9 打包与部署

- 前后端一起打包成单个 Windows exe（用户原话："我需要把RUST和JS一起打包成一个WINDOWS下的EXE"）。已实现，`npm run tauri:build` 产出 `roc_desk.exe` + MSI/NSIS 安装包。
- exe 命名为 `roc_desk`（用户原话："应用程序名叫roc_desk.exe"），而非之前的 devhub。已实现（`Cargo.toml` package name、`tauri.conf.json` productName 均已改名；Rust lib 内部 crate 名 `devhub_lib` 保留未改，纯内部标识不影响产物命名）。
- 构建产物统一放到仓库根目录 `build/` 而非 Cargo 默认的 `src-tauri/target/`（用户原话："打包好的EXE目录请放 F:\code\wuyou\roc_desk\build目录"）。已实现，`.cargo/config.toml` 里 `target-dir = "../build"`。
- **数据目录位置**（2026-08-18 需求，用户原话："SQLITE的数据和日志还有配置文件等可变内容，应该放在和EXE平行的目录或它的子目录中，不要打包进去"）：从 `%APPDATA%\com.devhub.desktop\`（Windows 系统级用户数据目录）迁移到 exe 同级的 `data/` 子目录（`std::env::current_exe()` 定位），便于整个安装目录整体迁移/备份/绿色部署。已实现；旧数据（含此前测试用的连接档案）已手动迁移一份到新位置（含 WAL 模式的 `-wal`/`-shm` 边车文件，直接 `cp` 主 db 文件而漏掉这两个文件会导致 SQLite 打开时看不到最近写入的表，这也是本次迁移过程中踩到的一个小坑）。

### 3.10 网页浏览

- **需求**（2026-08-18，用户原话："浏览器功能现在还不能用，我希望在这个工具区浏览过的所有网页都有历史记录可打开成TAB"）：此前 quick-tools 的 Globe 图标一直是 `disabled` 占位。已实现。
- **实现方式被用户当场纠正过一次**：第一版网页内容用独立的 `WebviewWindow` 弹窗承载，用户明确反馈："浏览器输入地址后不应该是弹框，而是默认在TAB下打开，和编辑器一样"。改用 Tauri 2.0 的"子 WebView"能力（`Window::add_child`，需要给 `tauri` crate 开 `unstable` cargo feature）：在主窗口内按前端实时测量的面板矩形区域（`getBoundingClientRect()`）叠加一个独立的子 WebView 实例，视觉上表现为"内嵌在这个 Tab 里"，实际是操作系统级别的独立 WebView2 视图摆在那块屏幕坐标上——不是 `<iframe>`：`<iframe>` 和宿主页面共享同一个 WebView 进程/JS 上下文，且大量真实网站用 `X-Frame-Options`/CSP `frame-ancestors` 拒绝被嵌入会直接空白失败，子 WebView 不受这些"防嵌入"头影响。安全隔离维持不变——子 WebView 的 label 是 `browser-panel`，同样不在 `capabilities/default.json` 的 `windows: ["main"]` 列表里，没有任何 Tauri command 调用权限，这个特性不因为"内嵌"还是"独立窗口"而改变。**原生子 WebView 的代价**：它不参与 HTML 的 CSS 层叠上下文，不管前端 `display: none` 怎么设，只要没显式调用 `hide()`，它就会一直盖在窗口里同一块屏幕坐标上的任何东西上面——所以切走"网页浏览"这个 Tab、或用户点"返回历史记录"时，前端必须显式调 `browser_hide`；侧边栏拖拽调宽、窗口缩放、终端面板展开收起都会改变面板区域，前端用 `ResizeObserver` + `window resize` 监听同步调用 `browser_set_bounds` 重新定位。
- **地址栏输入**：`browser::normalize_url` 处理三种输入——完整 URL（校验协议只能是 `http`/`https`，其它协议如 `file://` 直接拒绝，这是防止地址栏被用来绕过 WebView 权限模型的边界）、裸域名（形如 `bing.com`，自动补 `https://`）、其它任意文本（当作搜索词交给 Bing）。有单元测试覆盖这四类输入。
- **历史记录**：新表 `browser_history`（`id`/`url`/`title`/`visited_at`），每次成功打开写一条，面板里可删除单条/清空，点击历史行等价于导航同一个内嵌子 WebView 到那个地址并重新显示（不是新开一个视图——内嵌模式下只有一块面板区域，同时只能显示一个页面，这点和独立弹窗版本"每条历史都能开一个新窗口"的语义不同，是内嵌方案的固有取舍）。已实现。
- **已知限制**：只有地址栏入口做了协议校验；网页内部的链接跳转（比如页面里的 `<a href="file://...">`）目前没有拦截，因为需要研究 Tauri 的导航钩子（navigation event/`on-navigation`）才能做，本轮未展开，留作后续项。同一时刻只能内嵌显示一个页面（不是多页签浏览器），符合"和编辑器一样在 Tab 区域里打开"这个诉求的核心（不再弹窗），没有做成真正的多标签页浏览器——那是明显更大的范围，本轮未展开。

### 3.11 全局搜索（Explorer）

- **需求**（2026-08-18，用户原话："左边的目录树，需要加类似的搜索功能"，附了一张 VS Code 全局搜索面板截图——Search/Replace 两个输入框 + Aa/ab/.* 三个开关）。经确认用户要的是跨文件内容搜索+替换（不是只按文件名过滤目录树）。已实现。
- **UI 位置**：不新建一整套 VS Code 式的多图标活动栏，在侧边栏顶部加两个小图标（资源管理器/搜索）切换侧边栏内容本身，Explorer 树和搜索面板互斥显示——用最小改动达到"和截图类似"的效果。搜索结果按文件分组、可折叠，点击某一行匹配会打开对应文件并跳转到那一行（`editorStore` 新增 `pendingReveal` 状态，`CodeEditor.tsx` 消费后调 Monaco 的 `revealLineInCenter`/`setPosition`）。
- **后端实现**：`fsops::FileOps` trait 新增 `search_text`/`replace_text` 两个默认方法，和 `copy`/`delete` 走的是同一个"只依赖 `list_dir`/`read_file_raw`/`write_file_bytes` 这几个基础原语，本地/远程自动共用同一套递归逻辑"的路子，不需要分别给本地/远程各写一套。**已知取舍**：不是索引查询，每次搜索都要把工作区里没被排除的文件全部读一遍再正则匹配——本地走 `std::fs` 很快，远程每个文件是一次独立的 SFTP round trip，工作区文件数很大时会明显慢于 VS Code 的 ripgrep；这是为了复用现成的 trait 抽象、避免重新实现一遍"远程走 SSH 跑 shell 命令"（日志模块的实时搜索是这个思路）而做的取舍，MVP 阶段够用，真要提速可以后续把远程分支单独换成 `grep -rn`。搜索会跳过常见噪音目录（`.git`/`node_modules`/`target`/`dist` 等）和常见二进制扩展名，并对文件内容做"前 8KB 有没有 NUL 字节"的二进制启发式判断；单文件超过 2MB、总匹配数超过 3000 条或命中文件数超过 500 时提前收手并标记 `truncated`，前端相应提示"结果可能不全"。
- **查找替换**：直接写盘，不像 AI 编程助手那样有 Diff 待确认流程——手动查找替换的风险和"改一个文件后 Ctrl+S"是同一量级，不需要额外的二次确认层，出错了本来就该靠 Git/备份兜底，这和应用里其它"编辑即保存"的操作一致。替换范围严格等于前端已经展示的搜索结果文件列表（后端不重新跑一遍搜索），避免"UI 显示的和实际改的不一致"。固定按 UTF-8 写回，和 `write_file` 默认实现的既有编码约定一致，非 UTF-8 原文件替换后会被转成 UTF-8。
- **正则/全字匹配/大小写**：三个开关直接映射到 Rust `regex` crate 的能力——非正则模式下用户输入按字面量转义（`regex::escape`），全字匹配包一层 `\b...\b`，替换时非正则模式用 `regex::NoExpand` 包住替换文本防止里面的 `$` 被误解释成捕获组引用。有单元测试覆盖大小写/全字匹配/正则语法/字符下标计算（后端返回的匹配位置是字符下标不是字节下标，避免中文等多字节字符导致前端高亮位置错位）。

## 4. 非功能需求

- **权限与自主性**：用户明确授权在本项目目录下的绝大多数操作（构建、跑测试、杀掉本项目自己的旧调试进程、文件编辑等）无需逐次确认，唯一例外是"删除工作区外的文件"（多次会话重申，最近一次原文："只要不删除工作区外的文件，我授权你所有操作，不需要我人工确认"）。
- **安全边界**：Explorer 命令强制工作区根目录边界校验（`guard_local_path`），SFTP 自由浏览工具刻意不做这个校验（两者语义不同，前者是"编辑当前项目"，后者是"运维时到处看看"）。
- **凭据安全**：SSH 密码、AI Provider API Key 均不落库明文，统一走系统密钥链（Windows Credential Manager），数据库里只存一个引用字符串（如 `ssh:{uuid}:secret`）。
- **错误处理约定**：后端 `AppError` 序列化成 `{kind, message}` 而不是裸字符串，前端统一用 `formatError()` 解析，禁止 `String(e)` 这种会把对象错误显示成 `[object Object]` 的写法（本轮会话之前的真实 bug，已修复）。
- **UI 反馈约定**：任何异步操作失败必须有可见反馈（toast 或内联错误 + 重试按钮），不允许"看起来卡住/空白但其实是请求失败了"的静默失败——这条原则在本轮会话里至少两次被真实 bug 触发后重新强调（Explorer 空目录 bug、密码认证死路 bug），已经沉淀为 memory 里的经验记录，后续开发应主动遵循，而不是等用户报告了才补。

## 5. 已知限制 / 明确未做的范围（避免被误解为疏漏）

- Lua 插件引擎：完全未开始。
- MCP 客户端：未开始（AI 编程助手的工具集是自研的，不经过 MCP）。
- 多窗口/多工作区并行状态隔离：未设计（DESIGN.md §十-7 列为开放风险，本轮未涉及）。
- 真正的自动重连（无需用户点按钮、连接池层面存活探测+后台重试）：未开始。当前只做到"断线可见 + 一键手动重连"（见 §3.2）；`ssh/reconnect.rs` 的 `Backoff` 工具类仍未接入实际触发路径。
- SFTP 双栏浏览器的多选拖拽、断点续传：未开始，见 §3.3（目录级粗粒度传输进度已实现）。

## 6. 关键决策与经验记录（供后续维护者参考）

1. **目录被误删事故与恢复**：早期一次 `npm create tauri-app --force` 意外清空了项目目录（含 DESIGN.md、docs/、.claude/）。恢复策略是"记忆重建 + 用户提供的备份文件交叉核对 + 浏览器另存为抓取当时页面 DOM 快照 + 对确实无法找回的文件（`docs/prototypes/devhub-dialogs.html`）明确标注为重新生成而非恢复"，全程向用户保持诚实区分"确定恢复/凭记忆重建/全新生成"三种置信度，不能混为一谈假装都是原文件。
2. **`keyring = "3"` 必须显式声明平台 feature**（如 `windows-native`），否则 `set`/`get` 在目标平台上编译通过但实际是空操作，是一类"看起来成功、测试也不会自然触发失败"的隐蔽 bug，只有实际存到系统密钥链再读出来才能验证——纯靠 `cargo check`/单元测试里 mock 掉 keyring 是发现不了的，必须写一个真正调用 `Entry::set_password`/`get_password` 的集成性质测试（本次是临时加了个 `#[tokio::test]`，验证完手动删除，不作为常驻测试保留，因为它会真实读写本机的系统凭据管理器）。
3. **SQLite WAL 模式下迁移数据文件必须带上 `-wal`/`-shm` 边车文件**，只复制主 `.db` 文件会导致最近的写入（还停留在 WAL 里未 checkpoint）读不到。
4. **异步 Rust 里 `MutexGuard` 不要作为函数尾部表达式里的匿名临时量跨 `.await` 链式调用**（如 `session.lock().await.some_async_method().await`），编译器有时无法正确推导临时量的生命周期覆盖到内层 `.await`，报 E0597；稳妥写法是先 `let mut guard = session.lock().await;` 绑到具名变量，再调用。
5. **Tauri 后台构建命令的工作目录敏感**：`npm run tauri:build` 必须在仓库根目录执行（`src-tauri/`、`src-web/` 是同级而非嵌套目录），且 `beforeBuildCommand` 里 `--prefix` 路径要写成相对仓库根目录的形式，不能想当然按"相对当前子目录"理解。

---

*本文档随实现进度更新；如某项需求的实现状态发生变化，请同步更新对应小节，不要另开"更新记录"章节维护两份状态。*
