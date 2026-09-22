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
   第九节），但不能互相依赖、不能反过来依赖宿主仓库。宿主仓库的 `app` 包依赖全部六个
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

版本策略：`core`/`ui-core` 和 `common`/`common-web` 分开打 tag（比如 `core-v1.2.0`、
`common-v1.0.0`），下游只依赖 `core` 的工具不需要因为 `common` 改动而升级。

## 五、单个工具库内部怎么摆（以 `roc_desk-ssh` 为例）

```
roc_desk-ssh/
├── Cargo.toml            # workspace: members = ["lib", "standalone"]
├── lib/                   # 包名 roc_desk_ssh，纯逻辑 + Tauri command 导出
│   ├── Cargo.toml          # 依赖 roc_desk_core = { git = "https://github.com/<you>/roc_desk-common",
│   │                       #                          package = "roc_desk_core", tag = "core-v1.2.0" }
│   └── src/
│       ├── lib.rs           # pub fn commands() -> 这个工具全部 #[tauri::command] 的列表，
│       │                    # 给 roc_desk（宿主）和 standalone 壳共用
│       └── ...（ssh.rs / sftp.rs / pty.rs 等现有逻辑原样搬过来）
├── standalone/            # 独立可执行外壳，只在"单独打包这个工具"时才需要
│   ├── Cargo.toml          # 依赖 roc_desk_ssh + roc_desk_core，binary crate
│   ├── tauri.conf.json     # 独立的 bundle identifier / 图标 / single-instance key
│   └── src/main.rs         # 几乎是模板代码：Builder::default().invoke_handler(roc_desk_ssh::commands())...
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
│                                    #   roc_desk_core       = { git = "...roc_desk-common", tag = "core-v1.2.0" }
│                                    #   roc_desk_ssh        = { git = "...roc_desk-ssh",       tag = "v1.4.0" }
│                                    #   roc_desk_sql        = { git = "...roc_desk-sql",        tag = "v1.1.0" }
│                                    #   roc_desk_workspace  = { git = "...roc_desk-workspace",  tag = "v2.0.1" }
│                                    #   roc_desk_editor     = { git = "...roc_desk-editor",     tag = "v1.0.3" }
│                                    #   roc_desk_http       = { git = "...roc_desk-http",       tag = "v0.3.0" }
│                                    #   roc_desk_explorer   = { git = "...roc_desk-explorer",   tag = "v1.2.0" }
│                                    # 把六个工具的 commands() 拼进 generate_handler!，
│                                    # 首页/多进程 launcher 逻辑（HOME_MODES_DESIGN.md）留在这里
└── src-web/                        # 宿主前端：首页启动器 + 组合六个工具的 npm 包
    └── package.json                # 依赖 "@roc_desk/ui-core"（github:...roc_desk-common#path:packages/ui-core）
                                     # + 六个 "@roc_desk/tool-*" 包，同样按 tag/commit 钉版本
```

宿主仓库不再有自己的 `core` 包，**没有任何仓库需要反过来依赖 `roc_desk`**——它是纯粹
的叶子节点（依赖图意义上的"汇聚点"，不是"被依赖点"）。`build-portable.ps1` 流程不变：
`npm run tauri:build` → 编 agent → 拷 `bin/`，只是"确认版本"这一步现在多了
`roc_desk-common` 的 tag 需要一并确认。

## 七、版本钉住策略

- **所有仓库 → `roc_desk-common`**：Cargo 用 `git = "...", tag = "core-vX.Y.Z"` 或
  `"common-vX.Y.Z"`；npm 用 `github:<you>/roc_desk-common#path:packages/ui-core&<sha>`
  或 `#path:packages/common-web&<sha>`。
- **需要 `common` 能力的工具库 → `roc_desk-common` 的 `common` 包**：同样钉 tag，
  互不影响只依赖 `core` 的其他工具。
- **宿主的 `roc_desk` → 六个工具库 + `roc_desk-common`**：同样钉 `tag`，**不用
  `branch`/`main`**——否则某个仓库随手 push 一个没测完的 commit，宿主的合并构建会
  莫名其妙跟着炸。
- **升级流程**：改完 `roc_desk-common`（或某个工具库）→ 打新 tag → 回到 `roc_desk`
  仓库把 `Cargo.toml`/`package.json` 里的 tag 改掉 → `cargo update -p <pkg>`/
  `npm install` → 这一步产生的 lockfile diff 就是"这次合并构建具体吃进了哪个版本"的
  完整记录，也是唯一需要做"跨仓库集成验证"的地方（跑一次 `cargo check` +
  `build-portable.ps1`）。如果改的是 `roc_desk-common` 的 `core`，要记得连带把所有
  依赖它的工具库都 bump 一遍（Cargo 允许同一个 crate的不同版本同时存在于依赖图里，
  但类型对不上会在编译期直接报错，不会是运行时才发现的隐患）。

## 八、发布库 `roc_desk-releases` 怎么用

这个仓库**不放源码**，只承接 Release：

- `roc_desk`（宿主）仓库打 tag（比如 `v3.1.0`）触发 CI：跑 `build-portable.ps1` 等价的
  打包步骤，产出 `roc_desk.exe`/安装包。
- CI 用一个 repo-scope 的 PAT（`GITHUB_TOKEN` 默认权限过不了跨仓库），执行
  `gh release create v3.1.0 --repo <you>/roc_desk-releases ./bin/roc_desk.exe ...`。
- 同一个 workflow 顺手把这次合并构建具体锁定的版本写进 Release 说明里（直接从
  `Cargo.lock`/`package-lock.json` 解析，不用手记）：
  ```
  roc_desk v3.1.0
  ├─ roc_desk-common    @ core-v1.2.0 / common-v1.0.0
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

## 九、AI 能力 / 目录树处理 / 编辑器组件——归类原则

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

## 十、迁移阶段

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
`common-web` 四个包原样搬过去，各自打第一个 tag（比如 `core-v0.1.0`、
`common-v0.1.0`）。这一步验证目标是"作为库被别人依赖能不能正常工作"，不需要自己能
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
   `<EditorPane/>` 包给编程工作区依赖（见第九节）
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

## 十一、这个方案的真实代价（不建议假装没有）

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

## 十二、已确认的归属决定

| 问题 | 决定 |
|---|---|
| `core`（内核）归哪 | 和 `common`（公共组件）合并放进同一个仓库 `roc_desk-common`，
  内部用 Cargo/npm workspace 分成强制（`core`/`ui-core`）和可选（`common`/
  `common-web`）两组包，避免宿主仓库自引用 |
| `browser/`/`log/`/`mcp/`/`windows_context_menu.rs` 归哪 | `roc_desk-common` 的
  `common`/`common-web` 包 |
| `http_desk/` 归哪 | 确认是独立工具，直接并入 `roc_desk-http` |
| `rdp/` 归哪 | 并入 `roc_desk-ssh`（SSH/SFTP 工具的一部分） |
| 仓库可见性 | 九个仓库全部 public——git 依赖（Cargo/npm）解析不需要配置 deploy
  key/PAT/SSH key，任何机器 clone 下来直接能跑；`roc_desk-releases` 的跨仓库
  `gh release create` 仍然需要一个 repo-scope PAT（这是"写"操作，和仓库公开与否无关） |
| 仓库数量是否锁死在 9 个 | 不锁死，以后再出现"够独立、又被 ≥ 2 个工具需要"的能力，
  可以参照 `roc_desk-common` 的先例再加仓库，标准见第九节末尾 |
