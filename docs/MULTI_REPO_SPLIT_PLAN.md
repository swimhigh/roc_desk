# roc_desk 多仓库拆分方案（6 工具库 + 1 基础库 + 宿主库 + 发布库 = 9 个 GitHub 库）

> 目标（用户原话）：把首页的六个工作台——SSH/SFTP、SQL 工作台、编程工作区、编辑器、
> HTTP 测试工作台、资源管理器——拆成六个独立 GitHub 库，每个都能单独编译成自己的 exe
> 独立使用；同时 `roc_desk` 这个"宿主"库引用这六个库的代码，合并编译出 `roc_desk.exe`；
> 再加一个统一的发布库；每个工具的代码只存在一份，不允许拷贝出多份维护；以后还会
> 继续加新工具，仓库数量不是硬限制，合乎逻辑就可以加。
>
> **核心设计原则（2026-09-22 定稿）**：内核（`core`，数据库/凭据/连接档案/工作区/符号
> 索引……）和公共组件（`common`，网页浏览/日志搜索/MCP/右键菜单）**合并放进同一个仓库
> `roc_desk-common`**，宿主仓库 `roc_desk` 只保留"合并编译出 exe"这一件事——这样依赖
> 方向彻底变成单向汇聚，不会出现"宿主仓库同时是被依赖方又是依赖方"的自引用问题。

## 一、为什么内核不能留在宿主仓库里（关键设计决策）

最初的想法是内核放宿主仓库的 `core` 包、`roc_desk-common` 再依赖宿主仓库的 `core`。
这个方案有一个隐藏的结构性问题：

- 宿主仓库的 `app` 包要用 git 依赖拉六个工具库的代码，才能合并编译出 `roc_desk.exe`；
- 但六个工具库又要用 git 依赖反过来拉**宿主仓库自己**的 `core` 包（因为工具编不出来
  离不开 `core`）；
- 结果宿主仓库同时是"引用方"（引用六个工具库）和"被引用方"（被六个工具库引用）——
  Cargo 在编译 `app` 时，工作区本地的 `core`（path 依赖）和工具库通过 git 拉到的
  `core`（打了 tag 的某个历史提交）是两个不同的解析结果，容易出现版本对不上、或者
  必须用 `[patch]` 硬改依赖图这类麻烦，前端 npm 的 `ui-core` 也是完全一样的问题
  （宿主前端依赖六个工具的 npm 包，工具的 npm 包又要靠 `github:<host>#path:packages/ui-core`
  反过来依赖宿主仓库）。

**解决办法：把 `core`（Rust）和 `ui-core`（npm）都搬进 `roc_desk-common` 仓库**，和
原本就打算独立出去的公共组件（`browser`/`log`/`mcp`/`windows_context_menu.rs`）放在
同一个仓库里，用 Cargo workspace / npm workspace 分成两个包（强制的 `core` + 可选的
`common`）。这样宿主仓库 `roc_desk` 变成纯粹的"汇聚点"：只往外拉依赖（六个工具库 +
`roc_desk-common`），没有任何仓库反过来拉它，依赖图彻底不成环。

## 二、现状盘点：什么是"六个工具"各自的，什么是共用的

`src-tauri/src/` 现在的模块，按"专属于哪个工具"分类：

| 模块 | 专属工具 | 说明 |
|---|---|---|
| `ssh/`、`agent/`（部分）、`rdp/` | **SSH/SFTP** | 终端会话、SFTP 双栏、Windows 远程 Agent、RDP 内嵌 |
| `sql/` | **SQL 工作台** | 数据源、对象树、SQL 执行、AI 辅助 |
| `coding/`、`ai/`、`pty/`（本地终端部分） | **编程工作区** | AI 编程助手、本地/远程终端、工作区内的编辑器状态 |
| （`App.tsx` 的 `standaloneShellVisible` 分支，后端复用 `fsops`/`commands::local_fs`） | **编辑器** | UltraEdit 式，不依赖工作区 |
| `http_desk/` | **HTTP 测试工作台** | 已确认独立工具，直接并入 `roc_desk-http` |
| （`components/LocalExplorer/`，后端复用 `commands::local_fs`/`sftp`/`agent`） | **资源管理器** | Total Commander 式双栏 |

不属于任何单一工具、独立成 `roc_desk-common` 的"公共组件"部分（可选依赖，工具按需接入）：

| 模块 | 现状 |
|---|---|
| `browser/`（网页浏览） | 目前主要给编程工作区用，本质是通用能力 |
| `log/`（日志搜索） | 编程工作区和资源管理器都在调用，已经是事实上的跨工具能力 |
| `mcp/` | 编程工作区在用（AI 工具调用协议），本质是协议层实现 |
| `windows_context_menu.rs` | Windows 原生右键"用 roc_desk 打开"注册，机制本身通用 |

真正被 ≥ 2 个工具依赖、拆出去后各工具库都还离不开的"内核"部分（强制依赖，同样放进
`roc_desk-common`，只是分在 `core` 包里）：

| 模块 | 谁在用 |
|---|---|
| `db/`（SQLite schema、迁移、连接池） | 六个工具全部 |
| `fsops/`（本地/远程统一文件操作） | 资源管理器、编辑器、编程工作区 |
| `credential/`（系统密钥链） | SSH/SFTP、SQL 工作台、编程工作区 |
| `connection/`（SSH/RDP 连接档案、TOFU） | SSH/SFTP、资源管理器、编程工作区 |
| `workspace/`（工作区概念：根路径、本地/远程、最近列表） | 编程工作区、资源管理器 |
| `symbols/`（符号索引跳转） | 编程工作区、编辑器 |
| `ai/` 底层能力（Provider 管理、LLM 调用/流式/重试退避、附件处理、上下文预算） | SQL 工作台、编程工作区 |
| `error.rs`、`state.rs`、`commands/mod.rs` 的 Tauri 胶水 | 全部 |
| 前端：`components/shared/`（Toast/ContextMenu/ConfirmDialog……）、`useFileTreeOperations.ts`、IPC 绑定类型 `types/bindings.ts` | 全部 |

**强制 vs. 可选的区别只体现在包边界，不体现在仓库边界**：`core`（Rust）/`ui-core`
（npm）是"没它工具编不出来"，`common`（Rust）/`common`（npm）是"工具可以选择要不要接
的独立能力"——SQL 工作台不需要网页浏览/MCP，就不用依赖 `roc_desk-common` 的 `common`
包，但仍然要依赖它的 `core` 包。

## 三、仓库拓扑与依赖方向

```
                    ┌─────────────────────┐
                    │   roc_desk（宿主库）  │  ← 只有 app 包 + 首页前端，
                    │  ─ app 包（合并 exe） │     不导出任何东西给别人依赖，
                    │  ─ 首页前端           │     是依赖图的最终汇聚点
                    └──────────┬───────────┘
                                │ git 依赖（引用六个工具的代码 + roc_desk-common）
          ┌──────────┬─────────┼─────────┬──────────┬──────────┐
          ▼          ▼         ▼         ▼          ▼          ▼
      roc_desk-  roc_desk-  roc_desk- roc_desk-  roc_desk-  roc_desk-
        ssh        sql      workspace  editor      http     explorer
          │          │         │  │      ▲          │          │
          │          │         │  └──────┘          │          │
          │          │         │   workspace 依赖    │          │
          │          │         │   editor 导出的     │          │
          │          │         │   <EditorPane/>     │          │
          └──────────┴─────────┴──────────┴──────────┴──────────┘
                                │ 全部依赖
                                ▼
                    ┌─────────────────────┐
                    │   roc_desk-common    │  ← core（强制）+ common（可选）两个包，
                    │  ─ core（强制）       │     只依赖自己，不依赖宿主仓库、
                    │  ─ common（可选）     │     也不依赖任何工具库，是依赖图的
                    │  ─ ui-core（强制）    │     最底层，被所有人依赖但谁也不依赖它
                    │  ─ common-web（可选） │
                    └─────────────────────┘

                    ┌─────────────────────┐
                    │  roc_desk-releases   │  ← 只放 Release 资产，无源码，
                    │  （统一发布落点）      │     由 roc_desk 的 CI 在打 tag 时推送过去
                    └─────────────────────┘
```

依赖方向**只有两层，彻底不成环**：

1. **底层**：`roc_desk-common`——不依赖任何其他仓库，被所有工具库和宿主仓库依赖。
   内部再分强制（`core`/`ui-core`）和可选（`common`/`common-web`），但对外都是同一个
   仓库、同一次 `git` 拉取。
2. **工具层 + 宿主层**：六个工具库依赖 `roc_desk-common`（`core`/`ui-core` 必选，
   `common`/`common-web` 按需），工具之间允许有单向依赖（`workspace → editor`，见
   第十节），但不能互相依赖、不能反过来依赖宿主仓库。宿主仓库的 `app` 包依赖全部六个
   工具库 + `roc_desk-common`，是唯一的"汇聚点"，不被任何人依赖。

判断新东西该放哪一层的标准不变：**没它工具编不出来 → `roc_desk-common` 的 `core`；
工具可以选择要不要接的独立能力 → `roc_desk-common` 的 `common`；只有一个工具真正
拥有、只是被另一个工具复用 → 直接依赖那个工具库**。

## 四、`roc_desk-common` 仓库内部怎么摆

```
roc_desk-common/
├── Cargo.toml              # workspace: members = ["core", "common"]
├── core/                    # 包名 roc_desk_core，强制依赖，六个工具库和宿主 app 都要
│   └── src/
│       ├── db/  fsops/  credential/  connection/  workspace/  symbols/  ai/
│       └── error.rs  state.rs（AppState 骨架，具体字段由各工具在初始化时插件式注册）
├── common/                  # 包名 roc_desk_common，可选依赖，依赖 core
│   └── src/
│       ├── browser.rs        # 内嵌浏览器（原 browser/）
│       ├── log_search.rs     # 日志搜索（原 log/）
│       ├── mcp.rs            # MCP 协议支持（原 mcp/）
│       └── context_menu.rs   # Windows 右键菜单注册（原 windows_context_menu.rs）
├── packages/ui-core/        # npm 包 "@roc_desk/ui-core"，强制依赖，
│   └── src/                  # Toast/ContextMenu/ConfirmDialog、useFileTreeOperations、
│                              # IPC 绑定类型、AiChatTimeline 等共享前端基础件
└── packages/common-web/     # npm 包 "@roc_desk/common-web"，可选依赖，依赖 ui-core，
    └── src/                  # 浏览器标签页 UI、日志搜索结果面板等公共组件的前端部分
```

`roc_desk-common` **没有 `standalone/`**——它不是一个独立可运行的桌面工具，纯粹是给
别的仓库引用的库，不需要自己的 `tauri.conf.json`/图标/`main.rs`。

**版本策略——必须整个仓库统一打一个 tag，不能给 `core`/`common` 分开的版本号**：
最初想法是 `core-v1.2.0`/`common-v1.0.0` 各自独立打 tag，"下游只依赖 `core` 的工具
不用因为 `common` 改动升级"，但这个想法有一个隐藏的正确性问题——Cargo 的 git 依赖
按 `(仓库地址, 引用)` 算 SourceId，`core-v1.2.0` 和 `common-v1.0.0` 是同一个仓库的
两个不同引用，在 Cargo 眼里是**两个完全不相关的来源**，哪怕两个 tag 指向的
`roc_desk_core` 代码一字不差。宿主最终要合并六个工具，只要有两个工具分别通过
`core-v1.2.0` 和 `common-v1.0.0` 两个不同 tag 传递依赖到 `roc_desk_core`，Cargo 就会
把它们当成两个不同的 crate 编译两份，轻则"发现重复 crate"报错，重则因为
`AppState`/各种类型来自"不同的 `roc_desk_core`"而在类型检查阶段报一堆看起来毫无
道理的"expected struct Foo, found struct Foo"——这是 Cargo 处理多来源 git 依赖的
已知行为，不是这次拆分才有的边缘情况，一旦宿主真正合并六个工具就必然会踩到。

正确做法：**`roc_desk-common` 整个仓库只有一套版本号**（比如 `v1.5.0`），`core`/
`common`/`ui-core`/`common-web` 四个包永远在同一个 tag 上一起发布，哪怕某次改动只碰
了 `common` 包、`core` 完全没变。下游谁也不区分"我只要 core 的 tag"还是"我要 common
的 tag"——大家永远钉同一个 tag 字符串，保证宿主最终合并构建时，不管从哪个工具库传递
拉到 `roc_desk-common`，解析到的都是同一个 git 引用、同一个 SourceId。代价是"只改了
`common` 包也要把只依赖 `core` 的工具的 pin 一起碰一下"，比"完全独立版本号"多一点点
无谓的 bump，但换来的是从根上排除掉一类隐蔽到很难排查的编译期/依赖图错误，这笔交易
划算。

## 五、单个工具库内部怎么摆（以 `roc_desk-ssh` 为例）

```
roc_desk-ssh/
├── Cargo.toml            # workspace: members = ["lib", "standalone"]
├── lib/                   # 包名 roc_desk_ssh，纯逻辑 + Tauri command 导出
│   ├── Cargo.toml          # 依赖 roc_desk_core = { git = "https://github.com/<you>/roc_desk-common",
│   │                       #                          package = "roc_desk_core", tag = "v1.5.0" }
│   │                       # tag 是 roc_desk-common 整个仓库统一的版本号，不是
│   │                       # "core 专属"的版本号——见第四节的版本策略说明。
│   └── src/
│       ├── lib.rs           # 每个 #[tauri::command] 函数保持 `pub`（和现在单仓库里
│       │                    # `commands::http_desk::http_list_collections` 这类模块内
│       │                    # 命令完全一样的写法，只是以后从模块路径变成 crate 路径）；
│       │                    # 不需要、也不能包一层 "pub fn commands() -> Vec<...>"——
│       │                    # `tauri::generate_handler!` 是编译期宏，要的是每个命令的
│       │                    # 字面路径，不是一个运行期收集起来的函数指针列表，见第七节说明。
│       └── ...（ssh.rs / sftp.rs / pty.rs 等现有逻辑原样搬过来）
├── standalone/            # 独立可执行外壳，只在"单独打包这个工具"时才需要
│   ├── Cargo.toml          # 依赖 roc_desk_ssh + roc_desk_core，binary crate
│   ├── tauri.conf.json     # 独立的 bundle identifier / 图标 / single-instance key
│   └── src/main.rs         # 模板代码：`.invoke_handler(tauri::generate_handler![
│                           #   roc_desk_ssh::ssh_connect, roc_desk_ssh::sftp_list_dir, ...])`
│                           # ——把这个工具自己全部命令的字面路径列出来，和宿主 `roc_desk`
│                           # 的 lib.rs 里那份列表内容完全一样，只是宿主还要加上其它五个
│                           # 工具的，见第七节说明。
├── src-web/                # npm 包 "@roc_desk/tool-ssh"，这个工具的 React 组件
│   ├── package.json         # 依赖 "@roc_desk/ui-core": "github:<you>/roc_desk-common#path:packages/ui-core&<rev>"
│   └── src/
└── src-web-standalone/     # 独立打包用的最小 Vite 入口，mount "@roc_desk/tool-ssh" 的根组件
```

其余五个库结构完全一样，只是 `lib/`、`src-web/` 里的业务代码不同——**这是"每个工具
代码只有一份"在物理上的体现**：业务代码只存在于对应工具库里，`roc_desk`（宿主）不
拷贝、只引用。需要接 `roc_desk-common` 的 `common` 能力（比如编程工作区要接日志搜索/
网页浏览/MCP）的工具，在 `lib/Cargo.toml`/`src-web/package.json` 里多加一条对
`roc_desk_common`/`@roc_desk/common-web` 的依赖即可，和依赖 `core`/`ui-core` 是同一
个仓库、同一套机制，只是包名不同。

## 六、`roc_desk`（宿主）库内部怎么摆

```
roc_desk/                          ← 现在这个仓库改造后的样子
├── Cargo.toml                      # 单一 binary crate，包名 roc_desk（合并出 roc_desk.exe）
│   └── src/lib.rs                  # 依赖 roc_desk-common 的 core/common（按需）+ 六个工具库：
│                                    #   roc_desk_core       = { git = "...roc_desk-common", tag = "v1.5.0" }
│                                    #   roc_desk_ssh        = { git = "...roc_desk-ssh",       tag = "v1.4.0" }
│                                    #   roc_desk_sql        = { git = "...roc_desk-sql",        tag = "v1.1.0" }
│                                    #   roc_desk_workspace  = { git = "...roc_desk-workspace",  tag = "v2.0.1" }
│                                    #   roc_desk_editor     = { git = "...roc_desk-editor",     tag = "v1.0.3" }
│                                    #   roc_desk_http       = { git = "...roc_desk-http",       tag = "v0.3.0" }
│                                    #   roc_desk_explorer   = { git = "...roc_desk-explorer",   tag = "v1.2.0" }
│                                    # 把六个工具全部命令的字面路径列进同一个
│                                    # generate_handler!（见第七节，不是调用某个聚合函数），
│                                    # 首页/多进程 launcher 逻辑（HOME_MODES_DESIGN.md）留在这里
└── src-web/                        # 宿主前端：首页启动器 + 组合六个工具的 npm 包
    └── package.json                # 依赖 "@roc_desk/ui-core"（github:...roc_desk-common#path:packages/ui-core）
                                     # + 六个 "@roc_desk/tool-*" 包，同样按 tag/commit 钉版本
```

宿主仓库不再有自己的 `core` 包，**没有任何仓库需要反过来依赖 `roc_desk`**——它是纯粹
的叶子节点（依赖图意义上的"汇聚点"，不是"被依赖点"）。`build-portable.ps1` 流程不变：
`npm run tauri:build` → 编 agent → 拷 `bin/`，只是"确认版本"这一步现在多了
`roc_desk-common` 的 tag 需要一并确认。

## 七、跨 crate 的 Tauri command 注册 + AppState 组装（关键实现细节）

前面几节把"命令怎么注册"简化成了"把 commands() 拼进 generate_handler!"，这是不准确
的说法，需要单独讲清楚——这两件事都不能"运行期动态拼装"，只能"编译期把该列的全部
列出来"，这是整个多仓库方案里对 Tauri 项目结构影响最大的一条约束，必须在阶段 0 就
按这个真实约束设计，不是细节问题。

### Tauri command 只能编译期整份列出来，不能运行期动态注册

`tauri::generate_handler!` 是一个宏，展开成一段按命令名字符串做 `match`、分发到具体
函数的代码——它要的是每个命令函数的**字面路径**（`crate::module::fn_name`），不是一
个运行期能拿到的 `Vec<Box<dyn Fn>>`（每个 `#[tauri::command]` 函数的参数类型、返回
类型都不同，没有办法装进同一个 trait object 集合里，这是 Tauri 的既有设计，不是这次
拆分引入的限制）。所以之前几节写的"每个工具库导出一个 `pub fn commands() -> Vec<...>`
函数"这个想法实际上编不出来，已经改成：每个工具库把自己的 `#[tauri::command]` 函数
保持 `pub`（和 roc_desk 现在 `commands::http_desk::http_list_collections` 这类模块内
命令完全一样的写法），宿主仓库 `roc_desk` 的 `lib.rs` 里手写一份完整列表：

```rust
// roc_desk（宿主）的 lib.rs，示意
tauri::Builder::default()
    .invoke_handler(tauri::generate_handler![
        roc_desk_ssh::ssh_connect,
        roc_desk_ssh::sftp_list_dir,
        roc_desk_sql::sql_execute,
        roc_desk_workspace::coding_send_message,
        roc_desk_editor::editor_ocr_image,
        roc_desk_http::http_import_postman,
        roc_desk_explorer::explorer_list_dir,
        // ……六个工具全部命令，一个不能少
    ])
```

**这是一项真实的、持续的维护成本，不是一次性的**：任何工具库新增/删除一个
`#[tauri::command]`，宿主仓库这份列表都要手动跟着改一行，光把工具库的 tag 号升级
（`cargo update`）并不会自动同步这份列表——这和"改了内部实现、只需要 bump 版本号"
是两类不同的改动，前者宿主仓库完全不用碰源码，后者必须碰。建议每个工具库在自己的
`lib.rs` 顶部维护一份 `pub const COMMANDS_CHANGELOG` 或者干脆在 CHANGELOG 里显式标注
"新增/删除了哪些 command"，升级时对着这份记录去改宿主的列表，不要指望"编译器会提醒
你漏掉了"——漏掉一个命令不是编译错误，是运行期前端调用这个命令时收到"command not
found"，隐蔽性比编译错误高得多。

**2026-09-23 核实：这一段设计已经在实际代码里验证过**——宿主 `src-tauri/src/lib.rs`
现在真实的 `generate_handler!` 列表里就是这样写的（`roc_desk_explorer::cmd::local_list_dir`、
`roc_desk_http::cmd::http_send_request` 这类完整 crate 路径，一行一个命令，没有任何
聚合函数），和上面的设计完全一致。

### `AppState` 怎么跨 crate 组装

**2026-09-23 核实：实际做法比最初设想的"类型擦除扩展槽"简单——不需要那套机制**。
已经迁移出去的 `roc_desk-explorer`/`roc_desk-editor`/`roc_desk-http` 三个工具，用的
是 Tauri 本来就支持的"多个独立 `State<T>` 并存"：`roc_desk_http` 自己定义
`HttpAppState`，宿主 `lib.rs` 的 `setup()` 里 `app.manage(HttpAppState::new(...))`
单独注册一份，`roc_desk_http` 的命令函数直接从 `tauri::State<HttpAppState>` 取，
跟宿主原有的大 `AppState` 完全没有交集。而**还没迁移的工具（SSH/SQL/编程工作区），
状态原样留在宿主 `state.rs` 那个大 `AppState` 里**（`ssh_pool`/`workspace_manager`
这些字段都还在），一个工具都没搬走一点、大 `AppState` 就还是老样子——不需要提前
设计"核心字段 + 扩展槽"这种通用机制来"预留位置"，每次真正搬一个工具，就把它专属的
字段从大 `AppState` 里剪出来，单独定义一个小 state struct，`manage()` 注册一份，
这个工具的命令函数改一下参数类型（从 `State<AppState>` 换成 `State<XxxState>`），
仅此而已。大 `AppState` 会随着搬的工具越来越多、字段自然越来越少，直到最后（假设
SSH/SQL/编程工作区也搬完）宿主可能完全不需要自己的 `AppState` 了。

这比"类型擦除扩展槽"更好的地方：不需要引入 `TypeId`/`downcast`/运行期 panic 风险
这些额外的复杂度和心智负担，Tauri 的 `State<T>` 本来就是编译期类型安全的——`core`
（`roc_desk-common`）确实不需要预先知道以后会有哪些工具往里塞状态，因为它压根不用
承担"挂载别的工具状态"这个职责，各工具的状态各自独立存在，`core` 只负责真正共用的
东西（`DbPool`/`CredentialStore` 这些，各工具各自持有一份指向同一个底层资源的
`Arc`/连接池句柄，不是共享同一个大 struct）。

`roc_desk_core::error::AppError` 也有一个值得抄的过渡技巧：宿主 `lib.rs` 顶部写了
`pub use roc_desk_core::error;`，让宿主自己"仿佛还有一个本地 `crate::error` 模块"
可以复用——这样宿主里大量还没搬迁的旧代码里`use crate::error::AppError` 这种写法
不用一次性批量改成 `use roc_desk_core::error::AppError`，逐步迁移，不需要一次性
大范围重命名。

## 八、版本钉住策略

- **所有仓库 → `roc_desk-common`**：Cargo 用 `git = "...", tag = "vX.Y.Z"`（整个仓库
  统一的一个 tag，`core`/`common` 都从这个 tag 取，不分开编号，见第四节的版本策略
  说明——这是避免"同一个仓库两个不同 tag 被 Cargo 当成两个不同来源"这个坑的关键）；
  npm 用 `github:<you>/roc_desk-common#path:packages/ui-core&<sha>` 或
  `#path:packages/common-web&<sha>`，`<sha>` 同样固定成这一个 tag 对应的提交。
- **宿主的 `roc_desk` → 六个工具库 + `roc_desk-common`**：同样钉 `tag`，**不用
  `branch`/`main`**——否则某个仓库随手 push 一个没测完的 commit，宿主的合并构建会
  莫名其妙跟着炸。
- **升级流程**：改完 `roc_desk-common`（或某个工具库）→ 打新 tag → 回到 `roc_desk`
  仓库把 `Cargo.toml`/`package.json` 里的 tag 改掉 → `cargo update -p <pkg>`/
  `npm install` → 这一步产生的 lockfile diff 就是"这次合并构建具体吃进了哪个版本"的
  完整记录，也是唯一需要做"跨仓库集成验证"的地方（跑一次 `cargo check` +
  `build-portable.ps1`）。改的是 `roc_desk-common`（不管改的是 `core` 还是 `common`）
  时，要把所有间接依赖它的工具库的 pin 一起 bump 到同一个新 tag——这是"整个仓库统一
  一个 tag"策略换来的强制要求，不是可选的最佳实践，漏掉某个工具没 bump 就会在宿主
  合并构建时触发前面说的"同一个 crate 两个来源"编译错误。

## 九、发布库 `roc_desk-releases` 怎么用

这个仓库**不放源码**，只承接 Release：

- `roc_desk`（宿主）仓库打 tag（比如 `v3.1.0`）触发 CI：跑 `build-portable.ps1` 等价的
  打包步骤，产出 `roc_desk.exe`/安装包。
- CI 用一个 repo-scope 的 PAT（`GITHUB_TOKEN` 默认权限过不了跨仓库），执行
  `gh release create v3.1.0 --repo <you>/roc_desk-releases ./bin/roc_desk.exe ...`。
- 同一个 workflow 顺手把这次合并构建具体锁定的版本写进 Release 说明里（直接从
  `Cargo.lock`/`package-lock.json` 解析，不用手记）：
  ```
  roc_desk v3.1.0
  ├─ roc_desk-common    @ v1.5.0
  ├─ roc_desk-ssh       @ v1.4.0
  ├─ roc_desk-sql       @ v1.1.0
  ├─ roc_desk-workspace @ v2.0.1
  ├─ roc_desk-editor    @ v1.0.3
  ├─ roc_desk-http      @ v0.3.0
  └─ roc_desk-explorer  @ v1.2.0
  ```
- 如果某个工具想单独发布"我这个工具自己的 exe"给单独使用的人，就在它自己的仓库跑
  同样套路的 workflow，release 资产可以一起推到 `roc_desk-releases`，也可以各工具库
  自己的 Release 页面就够用——留到真的有人要单独用某个工具时再决定。

### 一个可以直接抄的 workflow 骨架

```yaml
# roc_desk（宿主仓库）.github/workflows/release.yml
name: release
on:
  push:
    tags: ["v*"]

jobs:
  build-and-publish:
    runs-on: windows-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions-rs/toolchain@v1
        with: { toolchain: stable }
      - uses: actions/setup-node@v4
        with: { node-version: 18 }
      - run: npm install --prefix src-web
      - run: npm run tauri:build
      - run: |
          # 从 Cargo.lock 里解析出这次合并构建实际锁定的六个工具 + roc_desk-common
          # 的 git rev/tag，拼成 Release 说明——脚本本身可以是几行 grep/awk 或者一个
          # 单独的 Rust/Node 小工具，不是这次方案要展开的实现细节，重点是"这一步存在，
          # 且从 Cargo.lock 解析而不是手记"。
          node scripts/collect-pinned-versions.js > release-notes.md
      - env:
          # repo-scope PAT，存成这个仓库的 secret——GITHUB_TOKEN 默认权限过不了
          # 跨仓库推送，这一步必须用单独配置的 PAT。
          GH_TOKEN: ${{ secrets.RELEASES_REPO_PAT }}
        run: |
          gh release create ${{ github.ref_name }} `
            --repo <you>/roc_desk-releases `
            --title "roc_desk ${{ github.ref_name }}" `
            --notes-file release-notes.md `
            build/release/roc_desk.exe `
            build/release/bundle/nsis/*.exe
```

每个工具库如果要支持"单独发布自己的 exe"，`.github/workflows/release.yml` 是同一份
骨架去掉"合并构建"那一步、换成"编自己 `standalone/` 那个 binary crate"，`gh release
create` 的目标仓库可以填自己仓库、也可以填 `roc_desk-releases`（用文件名前缀区分，
比如 `roc_desk-ssh-standalone-v1.4.0-windows.exe`）。

## 十、AI 能力 / 目录树处理 / 编辑器组件——归类原则

- **AI 底层能力**（Provider 管理、LLM 调用/流式/重试退避、附件处理、上下文预算窗口）
  → `roc_desk-common` 的 `core` 包；**各工具自己的 Agent 循环 + 工具定义**（`coding`
  的文件读写/Git/后台进程工具集，`sql` 的 SQL 执行/schema 内省工具集）是每个工具专属
  的业务逻辑，留在各自工具库，只是都基于 `core::ai` 搭起来。
- **前端对话面板 UI**（按轮次折叠 + 弹框详情 + 用户/AI 消息区分这套时间线渲染）抽成
  `roc_desk-common` 的 `ui-core` 里的通用组件 `AiChatTimeline`，"这一轮有哪些工具
  调用、每种工具调用详情怎么渲染"做成参数化配置，各工具面板基于它渲染。
- **目录树处理**（`useFileTreeOperations.ts`：多选状态机、剪贴板、批量操作）是纯前端
  交互逻辑，直接进 `ui-core`，各工具只需要实现"列目录"这一层适配（`FileTreeBackend`
  接口）。
- **编辑器组件**：编辑器本身是六个工具之一，Monaco 多标签编辑同时是"编程工作区"内嵌
  的编辑器面板。这种情况不进 `core`/`ui-core`——它是**一个工具的核心产出，恰好被另一
  个工具复用**，正确做法是 `roc_desk-workspace` 直接依赖 `roc_desk-editor` 导出的
  `<EditorPane/>` + `useEditorStore`，不是什么都往 `roc_desk-common` 里塞。这意味着
  工具之间也可以有单向依赖（`workspace → editor`），只要不成环即可。

判断新东西归属的标准：先问"这是纯逻辑能力（→ `roc_desk-common`）"还是"是另一个工具的
核心产出、我只是想复用（→ 直接依赖那个工具库）"，`roc_desk-common` 塞太多会变成第二
个"什么都耦合在一起"的单体，只是换了个名字。

## 十一、迁移阶段

### 阶段 0：在现有仓库内部先把三条边界划出来（不新建任何 GitHub 仓库）

把 `src-tauri/` 改造成 Cargo workspace（`core` + `common` + `app` 三个包），把第二节
"内核"表列的模块搬进 `core`，`browser`/`log`/`mcp`/`windows_context_menu.rs` 搬进
`common`；前端同理，把 `components/shared/`、IPC 绑定类型这些搬进 npm workspace 的
`packages/ui-core`，把浏览器/日志搜索的前端部分搬进 `packages/common-web`。这一步
纯粹是仓库内部重构，全程可回退，不涉及任何 GitHub 拓扑变化，可以完整跑通
`cargo check`/`npm run tauri:build` 验证边界划得对不对——建议先做完、稳定运行几天
再往下走，这是整个方案里风险最低、但工作量可能最大的一步。

### 阶段 1：把 `roc_desk-common` 整体搬出去，独立成仓库

新建 `roc_desk-common` 仓库，把阶段 0 已经跑通的 `core`/`common`/`ui-core`/
`common-web` 四个包原样搬过去，整个仓库打第一个统一 tag（比如 `v0.1.0`，四个包共用
这一个 tag，不分开编号，见第四节的版本策略说明）。这一步验证目标是"作为库被别人依赖
能不能正常工作"，不需要自己能
跑出一个 exe。**必须放在搬任何工具之前**——因为现在所有工具都强制依赖 `core`，晚搬
会导致工具库和宿主仓库之间又出现"临时依赖宿主仓库的 core"这种要返工的中间状态。

### 阶段 2：挑一个"试点"工具验证跨仓库机制本身能不能跑通

**建议用 HTTP 测试工作台当试点**——它是唯一一个还没写代码的工具，可以直接按新模式
从零建仓库 `roc_desk-http`，只依赖阶段 1 已经跑通的 `roc_desk-common`，不需要
"迁移"，只需要"新建"：验证 Cargo/npm 的跨仓库依赖能不能顺利解析、独立壳能不能编出
自己的 `.exe`、宿主仓库引用这个新仓库能不能合并编译成功。这一步是排雷阶段，出问题
成本最低。

### 阶段 3：按耦合度从低到高，把现有五个工具逐个搬出去

建议顺序（可调整，原则是"越不依赖别的工具/共享状态越少的越先搬"）：

1. **资源管理器** — 相对独立，主要吃 `core::fsops`/`connection` + `common`（日志搜索导入）
2. **编辑器** — 相对独立，主要吃 `core::fsops`/`symbols`；搬完之后要导出可嵌入的
   `<EditorPane/>` 包给编程工作区依赖（见第十节）
3. **SQL 工作台** — 自成一块，主要吃 `core::credential`/`db`/`core::ai`
4. **SSH/SFTP** — 吃 `core::connection`/`credential`，且资源管理器/编程工作区都会
   引用它暴露的"远程连接"能力，放前面几个稳定之后再动；`rdp/` 随这一批一起搬进
   `roc_desk-ssh`
5. **编程工作区** — 现在和 AI 编程助手、终端、符号索引缠得最深，还要接 `common`
   （网页浏览/日志搜索/MCP）+ `roc_desk-editor`（内嵌编辑器面板）两个额外依赖，依赖
   面最广，建议放最后

每搬一个：新建仓库 → 把对应代码剪切过去、按第五节的结构补上 `standalone/` 壳 → 在
这个新仓库里独立跑通"编出自己的 exe" → 回到宿主仓库把"合并编译"里对应的这部分从
"仓库内代码"换成"git 依赖" → 跑一次完整 `build-portable.ps1` 确认合并构建没受影响 →
这个工具的旧代码才能从宿主仓库里删掉。**每一步都保持"删旧代码"是最后一步、且必须先
有新仓库跑通独立构建作为前提**，避免中间状态两边都不完整。

### 阶段 4：发布库

五个工具都搬完、稳定运行一段时间后，再建 `roc_desk-releases`，把 CI 的发布步骤从
"直接在 `roc_desk` 仓库开 Release"切到"推到 `roc_desk-releases`"。这一步不紧急，
放最后风险最低。

### 阶段 5：收尾

更新 `README.md`/各 `*_DESIGN.md`，把"目录结构"章节换成"这是宿主仓库，六个工具的
源码在各自仓库，内核和公共组件在 `roc_desk-common`"的说明；补一份"以后要改工具 X
的代码该去哪个仓库改、改完怎么让 `roc_desk` 吃到新版本"的贡献指南（哪怕只有你自己
一个人维护，几个月后自己也会忘）。

## 十二、这个方案的真实代价（不建议假装没有）

1. **跨工具改动变贵**：像"文件树多选/共用逻辑合并"这种同时涉及两个工具 + 共享 hook
   的改动，拆分后要变成"先在 `roc_desk-common` 改 `ui-core` 打 tag，再分别去两个工具
   仓库接新 tag、各自验证、各自打 tag，最后回宿主仓库两边都 bump"——一次原子提交变成
   一串跨仓库操作。这是这个架构本质的代价，不是能靠工具解决的细节问题。
2. **`roc_desk-common` 变成真正的"对外 API"**：现在想怎么改内部函数签名就怎么改；
   拆分后依赖它的库都钉着某个 tag，breaking change 需要意识到"这会影响所有还没升级
   到新 tag 的下游"，需要一点点版本纪律（哪怕只是自己记住"改了公开签名要发个新 tag
   并且找一遍谁在用"）。
3. **六份独立外壳要维护**：每个工具的 `standalone/` 目录（`tauri.conf.json`、图标、
   bundle identifier、single-instance key）是新增的、长期跟着维护的东西，不是一次性
   成本；`roc_desk-common` 没有 `standalone/`，不用维护这一份。
4. **验证工作量集中在阶段 0 和阶段 1**：真正有风险、需要动手才知道结果的两步是
   "三条边界能不能干净划出来"和"`roc_desk-common` 独立后跨仓库依赖机制能不能顺利
   跑通"，建议这两步都做完、都满意之后，再决定要不要继续往下走五个工具的搬迁。

## 十三、本地目录布局 + 实际迁移现状核实（2026-09-23）

### 本地目录布局

九个仓库在本机的实际摆放（用户 2026-09-23 确认）：

```
F:\code\wuyou\
├── roc_desk\          # 宿主仓库，就是现在这个仓库，路径不变
└── roc_tools\         # 六个工具仓库 + roc_desk-common + roc_desk-releases，
    ├── roc_desk-common\
    ├── roc_desk-ssh\
    ├── roc_desk-sql\
    ├── roc_desk-workspace\
    ├── roc_desk-editor\
    ├── roc_desk-http\
    ├── roc_desk-explorer\
    └── roc_desk-releases\
```

`roc_tools\` 下已经有 `README.md`（本地目录 ↔ GitHub 仓库对照表，全部指向
`github.com/swimhigh/<repo>`）、`sync-all.ps1`（批量 `git pull --ff-only` 每个子
仓库）、`build-all-tools.bat`（依次跑每个工具自己的 `build-standalone.bat`）。

### 实际迁移进度（比本文档的"迁移阶段"设想推进得快）

核实下来，阶段 0/1 已经完成、阶段 2/3 也已经部分展开，不是纯理论阶段：

- `roc_desk-common` 已独立成仓库，`core`/`common` 两个包结构和第四节设计一致，
  README 明确写着"只是库，没有 Tauri 可执行文件，不会反过来依赖宿主或任何工具库"。
- 宿主 `roc_desk` 的 `src-tauri/Cargo.toml` 已经切换成 git 依赖，目前接入了
  **资源管理器（`roc_desk-explorer`）、编辑器（`roc_desk-editor`）、HTTP 测试工作台
  （`roc_desk-http`）三个**（`roc_desk-ssh`/`roc_desk-sql`/`roc_desk-workspace` 三个
  仓库本地已经建好、有真实代码，但还没接进宿主的 `Cargo.toml`）。
- 六个工具仓库的内部结构（`Cargo.toml` workspace = `["lib", "standalone"]`、
  `lib/Cargo.toml` 依赖 `roc_desk-common` 的 `core`/`common` 包、各自的
  `build-standalone.bat`/`BUILD.md`/`MIGRATION.md`）和第五节的设计基本一致，
  `roc_desk-workspace` 已经按第十节的设计直接依赖 `roc_desk-editor` 导出的
  `<EditorPane/>`（其 `lib/Cargo.toml` 注释原话："Local filesystem browsing/editing
  ... all come from roc_desk-editor ... instead of being reimplemented here"）。

### 发现的版本漂移问题——已于当天核实后修复

核对各仓库 `lib/Cargo.toml` 实际锁定的 `roc_desk-common` tag，发现和第四节警告的
"必须整个仓库统一打一个 tag"这条原则已经没有对齐：

| 仓库 | 修复前锁定的 `roc_desk-common` tag |
|---|---|
| `roc_desk-explorer` | `common-v0.3.0` |
| `roc_desk-ssh` / `roc_desk-sql` / `roc_desk-http` / 宿主自己 | `common-v0.3.1` |
| `roc_desk-editor` | `common-v0.4.0` |
| `roc_desk-workspace` | `common-v0.5.0` |

宿主当时直接依赖 `roc_desk_core@common-v0.3.1`，同时通过 `roc_desk-editor` 传递依赖
`common-v0.4.0`、通过 `roc_desk-explorer` 传递依赖 `common-v0.3.0`——核对宿主仓库根
目录的 `Cargo.lock`，`roc_desk_core`/`roc_desk_common` 确实各自出现了三份不同
`source`（不同 git tag，Cargo 当成三个不同来源），`roc_desk_explorer` 也出现了两份
（`v0.2.0`/`v0.2.3`，分别来自 `roc_desk-editor` 的传递依赖和宿主自己的直接依赖）。
当时**还没有炸，是因为没有代码需要把"从 `roc_desk-editor` 传递拿到的
`roc_desk_core` 类型"和"宿主自己直接依赖的 `roc_desk_core` 类型"混在同一个函数调用
里传来传去**，不是因为没有问题——第七节最后确认的"已迁移工具各自独立 `manage()`
一份状态，不共用宿主大 `AppState`"这个真实做法，恰好也让这类重复 SourceId 暂时不会
在类型层面互相冲撞，但 Cargo 层面的重复来源本身仍然是真实存在的技术债，跨仓库
`cargo doc`/未来的集成测试都可能因为"同名不同源"而表现异常。

**处理结果（2026-09-23 当天完成）**：

1. 核对 `roc_desk-common`/`roc_desk-editor`/`roc_desk-explorer` 三个仓库的 `git tag`
   历史，确认 `common-v0.5.0`/`roc_desk-editor v0.2.3`/`roc_desk-explorer v0.2.3`
   （核心库 `lib/` 之间没有任何差异，只是 `standalone/` 前端和构建产物的差异——用
   `git diff <旧tag> <新tag> --stat -- lib/` 逐一确认过是空 diff）分别是三者当时的
   最新版本，且 `roc_desk-common` 从 `common-v0.3.0` 到 `common-v0.5.0` 之间的三次
   `git diff --stat` 全部是纯新增（没有任何一行删除/修改既有公开签名），确认对齐到
   最新版本对所有下游都是安全的。
2. 六个工具仓库的 `lib/Cargo.toml`（`roc_desk-editor`/`roc_desk-explorer` 还包括
   `standalone/Cargo.toml`）逐一改成统一指向 `common-v0.5.0`；`roc_desk-editor`/
   `roc_desk-workspace` 对 `roc_desk_explorer`/`roc_desk_editor` 的引用也一并对齐到
   `v0.2.3`/`v0.2.4`（editor 自己改了依赖，需要一个新 tag 才能让"改了依赖"这件事
   本身被下游看到，所以 editor/explorer 各自新打了一个 patch 版 tag，见下表）。
3. 每个仓库改完先本地 `cargo check` 单独验证通过，再提交 + 推送 + 打新 tag（只提交
   改动的 `Cargo.toml` 文件本身——每个仓库里都还有大量与这次任务无关的、明显是正在
   进行中的前端迁移工作没有提交，没有一并扫进这次的 commit）。
4. 宿主 `roc_desk` 的 `Cargo.toml` 跟着把 `roc_desk_core`/`roc_desk_common` 改到
   `common-v0.5.0`、`roc_desk_editor`/`roc_desk_explorer` 改到各自新打的 patch tag，
   `cargo check` + `build-portable.ps1` 全量验证通过。

最终各仓库对齐到的版本：

| 仓库 | 修复后的 tag |
|---|---|
| `roc_desk-common`（`core`/`common` 两个包统一） | `common-v0.5.0` |
| `roc_desk-explorer` | `v0.2.4`（新打，原来的 `v0.2.3` 内容不变，只是这个仓库自己
  对 `roc_desk-common` 的 pin 改了） |
| `roc_desk-editor` | `v0.2.4`（新打，同时把自己对 `roc_desk-explorer` 的 pin 从
  `v0.2.0` 改到 `v0.2.4`） |
| `roc_desk-http` | `v0.2.1`（新打） |
| `roc_desk-ssh` | `v0.3.1`（新打，之前 `v0.3.0` 从没被任何地方引用过，这次顺带
  打了新 tag 保持一致性，不是紧急修复） |
| `roc_desk-sql` | `v0.3.1`（同上） |
| `roc_desk-workspace` | `v0.2.1`（同上，同时把自己对
  `roc_desk-editor`/`roc_desk-explorer` 的 pin 都改到 `v0.2.4`） |

修复后核对宿主的 `Cargo.lock`：`roc_desk_core`/`roc_desk_common`/`roc_desk_explorer`/
`roc_desk_editor`/`roc_desk_http` 五个 crate 现在各自都只有一份 `source`（一个 git
tag），不再有重复 SourceId。`roc_desk-ssh`/`roc_desk-sql`/`roc_desk-workspace` 三个
仓库目前还没有被宿主的 `Cargo.toml` 引用（阶段 3 还没走到接它们进宿主这一步），它们
各自新打的 tag 是为将来接入时做好准备，不影响当前宿主的构建结果。

## 十四、已确认的归属决定

| 问题 | 决定 |
|---|---|
| `core`（内核）归哪 | 和 `common`（公共组件）合并放进同一个仓库 `roc_desk-common`，内部用 Cargo/npm workspace 分成强制（`core`/`ui-core`）和可选（`common`/`common-web`）两组包，避免宿主仓库自引用 |
| `browser/`/`log/`/`mcp/`/`windows_context_menu.rs` 归哪 | `roc_desk-common` 的 `common`/`common-web` 包 |
| `http_desk/` 归哪 | 确认是独立工具，直接并入 `roc_desk-http` |
| `rdp/` 归哪 | 并入 `roc_desk-ssh`（SSH/SFTP 工具的一部分） |
| 仓库可见性 | 九个仓库全部 public——git 依赖（Cargo/npm）解析不需要配置 deploy key/PAT/SSH key，任何机器 clone 下来直接能跑；`roc_desk-releases` 的跨仓库 `gh release create` 仍然需要一个 repo-scope PAT（这是"写"操作，和仓库公开与否无关） |
| 仓库数量是否锁死在 9 个 | 不锁死，以后再出现"够独立、又被 ≥ 2 个工具需要"的能力，可以参照 `roc_desk-common` 的先例再加仓库，标准见第十节末尾 |
| Tauri command 怎么跨 crate 注册 | 不是运行期聚合，是宿主 `lib.rs` 手写一份全部命令字面路径的清单（第七节）——这是本方案对现有代码结构影响最大的一条约束，已经在第七节详细展开 |
