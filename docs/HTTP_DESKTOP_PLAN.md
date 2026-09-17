# roc_desk HTTP 测试桌面（HTTP Desk）落地方案

> 状态：方案调研稿（可作为实现任务拆分的基线）
> 目标：仿照 Apifox/Postman，在 roc_desk 增加一个 HTTP 接口调试工作台，作为与「SSH 桌面」
> 「工作区」「编辑器」「SQL 桌面」平级的第六个工作模块（`WorkMode = "http"`）。
> 参考基线：本方案的组织方式、篇幅深度对齐 [docs/SQL_DESKTOP_PLAN.md](SQL_DESKTOP_PLAN.md)——
> 这是仓库里唯一一个"从零起一个大型独立工作模块"并已经落地验收过的方案，很多结论（AI 复用
> ChangeStore、独立 WorkMode 接入方式）直接沿用其结论，不重新论证一遍。
> **关键决策（round 2 修订）**：HTTP 桌面不新造一套"集合"概念，而是**直接复用现有
> `src-tauri/src/workspace/`（`WorkspaceManager`/`WorkspaceProfile`/`WorkspaceKind`/
> `FileOps`）模块**——和现在的「工作区」模块一样，以"一个本地或远程（SSH/Agent）目录"为
> 组织维度，首页展示的是同一份"最近工作区"列表，只是打开方式从 `spawnModule("workspace", id)`
> 换成 `spawnModule("http", id)`。这是本轮修订后最核心的架构变化，详见 §3.1、§4.4。

## 1. 目标与边界

### 1.1 用户目标

- 在桌面首页看到"HTTP 桌面"入口卡片、最近打开的工作区目录——和现有"工作区"模块共用同一份
  "最近工作区"列表（同一个本地文件夹或同一个 SSH/Agent 远程目录，既可以用工作区模块写代码，
  也可以用 HTTP 桌面调试接口，两者是同一份 `WorkspaceProfile` 记录的两种"打开方式"）。
- 新建工作区时可以选本地目录，也可以像现有"连接远程主机打开工作区"一样选 SSH 或 Windows
  Agent 连接 + 远程目录；在工作区内新建/导入接口集合，按文件夹组织请求；支持从 Postman
  Collection v2.1、OpenAPI/Swagger、curl 命令、HAR 导入，减少从 Apifox/Postman 迁移的摩擦。
- 打开某个工作区的 HTTP 桌面后进入独立界面：左侧请求树，中间 URL 栏 + Params/Headers/Cookies/Auth/Body/
  前置脚本/后置脚本/设置 多 Tab 编辑区，下方响应区（状态码/耗时/大小/Headers/Body/Cookies/
  测试结果），右侧 AI 工具栏——这一整套布局直接对齐用户截图里 Apifox 的调试页信息架构。
- 环境（开发/测试/生产等）与变量：全局/集合/文件夹/环境/请求级多层作用域，`{{var}}` 插值，
  内置动态值（时间戳、UUID、随机姓名/邮箱/手机号等，类比 Apifox 的"动态值"）。
- 请求发出前后可跑脚本：前置脚本改写请求，后置脚本对响应做断言、提取变量写回环境。
- 请求历史、一键复制为 curl、导出响应、批量运行一个文件夹（类似 Postman Collection Runner）。

### 1.2 非目标（首期不做）

- 不做团队云同步、云端 Mock 托管、多人协作评论——roc_desk 现状是单机便携应用，没有账号体系，
  这类能力涉及后端服务，超出"桌面工具"定位（对齐 `docs/SQL_DESKTOP_PLAN.md` §1.2 同样的边界原则）。
- 不做完整的"设计优先"（Design First）OpenAPI 可视化建模编辑器；OpenAPI 只作为**导入源**，
  不做反向的"从 UI 生成/维护 OpenAPI 文档"这类文档工具能力。
- GraphQL、gRPC、WebSocket、Server-Sent Events 首期不做，只做 REST/HTTP(S)；架构上不堵死后续
  扩展（§4.1 的 domain 分层预留 protocol 抽象），但不在首期验收范围内。
- Mock 服务只做本地进程内的最简匹配（Phase 3+，§9），不追求 Apifox"智能 Mock"那种基于 JSON
  Schema 自动推断字段规则的完整能力。
- 不内置浏览器插件/代理抓包（类似 Postman Interceptor）来"发现"已有请求；用户手动创建或导入。

## 2. 开源工具调研

用户明确要求"调研相关开源工具的代码"，这一节是本方案的调研主体，后面第 4 节的每个技术决策
都能在这里找到对应的参考来源和取舍理由。

### 2.1 调研对象与结论概览

| 项目 | 技术栈 | 许可证 | 存储模型 | 脚本引擎 | 对 roc_desk 的价值 |
|---|---|---|---|---|---|
| [Yaak](https://github.com/mountain-loop/yaak) | **Tauri + Rust + React/TS** | 源码 MIT（预编译二进制商用收费） | SQLite（工作区/请求/环境均为 DB 行） | Node.js 侧车进程（vendor 完整 Node 运行时），gRPC/WebSocket 通信 | 技术栈与 roc_desk **完全一致**，是唯一能直接照抄 Tauri 工程结构的参考；但插件运行时方案偏重，不直接照搬 |
| [Bruno](https://github.com/usebruno/bruno) | Electron + React（`bruno-cli`/引擎是 Node.js） | MIT | **文件系统即数据**：一个请求一个文件（`.bru` 或新的 OpenCollection YAML） | Safe Mode 用 QuickJS/WASM 隔离沙箱；Developer Mode 用 Node VM | 存储哲学（磁盘文件是唯一真相）和 roc_desk 已有的 SQL 桌面 `.sql` 文件方案高度同构，直接可以照搬这个思路（§4.4） |
| [Hoppscotch](https://github.com/hoppscotch/hoppscotch) | Vue.js（Web/Desktop/CLI 全家桶），后端 Postgres | MIT | 浏览器/客户端本地状态 + 可选云端 Postgres | 自研 `hopp-js-sandbox`，提供兼容 Postman `pm.*` 命名空间（含 `pm.expect` Chai 断言） | 脚本 API 的"**对齐 Postman `pm` 命名空间**"这个产品决策值得直接抄——大量用户手里有现成的 Postman 脚本，迁移成本最低 |
| [Insomnia](https://github.com/Kong/insomnia)（Kong） | Electron + React | Apache-2.0（桌面端；付费同步后端闭源） | NeDB → 后期迁移到自有存储，非文件系统 | Node VM（`insomnia-testing`/`node-libcurl`） | 技术栈不贴合（Electron），主要参考 UI/交互设计（环境变量作用域面板、`OneLineEditor` 插值高亮），不参考代码 |

**许可证结论**：以上四个都是 **MIT 或 Apache-2.0**，没有 SQL 桌面方案里 Beekeeper Studio 那种
AGPL 的坑（`docs/SQL_DESKTOP_PLAN.md` §4.2.1），移植代码/格式规范时按各自协议在对应文件顶部
留一行来源注释即可，不需要额外审批。Yaak 需要注意的只是"**预编译二进制**"的商用授权，这不影响
我们看它的源码做架构参考（源码本身是 MIT）。

### 2.2 Yaak——技术栈最贴近的参考实现

Yaak 是目前唯一一个和 roc_desk 技术栈（Tauri + Rust 后端 + React/TS 前端）完全一致的开源 API
客户端，"Rust 侧怎么组织 domain 代码、前端怎么通过 Tauri command 调用"这部分可以直接照抄工程
结构（`src-tauri/models`、`src-tauri/commands` 的划分方式），不需要重新摸索 Tauri 2.0 的最佳实践。

但有一个关键设计**不建议照搬**：Yaak 的脚本/插件系统靠一个**vendor 进整个 Node.js 运行时**的
侧车进程（重命名成 `yaaknode-<target-triple>`），Rust 主进程通过 WebSocket 和它跑一套自定义的
`InternalEvent` 协议做 IPC，插件用 TypeScript 写、跑在这个 Node 沙箱里。这个方案能换来"插件可以
用任意 npm 包"的灵活性，但代价是：

1. 安装包体积暴涨（vendor 一份完整 Node 运行时，通常是几十 MB 起）；roc_desk 现在的
   `build-portable.ps1` 产出物是"一个 exe + 几个瘦身过的 sidecar"（`wfreerdp.exe`、
   `roc_desk_agent.exe`），额外背一个 Node 运行时和现有"便携、轻量"的产品定位不符。
2. 多一层进程间 IPC 和一个需要单独维护版本兼容性的运行时，调试链路变长。

roc_desk 的脚本需求（前置改写请求、后置断言/提取变量）复杂度远低于"通用插件系统"，没有必要
为了这个子集去背一个 Node 侧车——§4.3 会说明改用**进程内嵌入的 QuickJS 引擎**替代，这也是
Bruno 自己在 Safe Mode 下的选择（见 §2.3），验证了"进程内嵌入的轻量 JS 引擎"这条路径对这类
产品的脚本子集是够用的。

值得照抄的是 Yaak 的**导入转换器**设计（支持 Postman/Insomnia/OpenAPI/Swagger/curl 一键导入）
和**模板函数系统**（`{{ now() }}`、`{{ uuid() }}` 这类内置函数与变量用同一套 `{{ }}` 插值语法，
不用像 Apifox 那样"动态值"和"变量引用"是两套不同的 UI 交互）——这个统一插值语法的思路直接采纳，
见 §4.5。

### 2.3 Bruno——文件即数据的存储模型，与 roc_desk 现有方案同构

Bruno 最核心的产品主张是"集合是磁盘上的纯文本文件，不是云端数据库"，天然支持 Git 版本控制、
PR 审查请求改动。它有两代文件格式：

- **`.bru`**：自定义的轻量标记语言，一个请求一个 `.bru` 文件，`meta`/`get|post|...`/`headers`/
  `body`/`auth`/`script:pre-request`/`script:post-response`/`tests`/`vars`/`assert` 等按块
  组织，块内容基本是"键: 值"或大段文本，逐行 diff 噪音很小。
- **OpenCollection YAML**（[spec.opencollection.com](https://spec.opencollection.com)，Bruno
  v3.0 起默认格式，2026-01 发布）：Bruno 把这套格式独立成了一个不绑定具体应用的开放规范
  （[opencollection-dev/opencollection](https://github.com/opencollection-dev/opencollection)，
  公开授权、附 JSON Schema 和 `@opencollection/converters` 转换工具包），描述"请求、环境、
  变量、认证、脚本、断言"的完整可执行集合，YAML 格式比自定义 `.bru` 语法更容易用标准库解析。

**这一节的结论直接决定 §4.4 的存储方案**：Bruno "文件是唯一真相"这个思路和 roc_desk 现有 SQL
桌面方案（`docs/SQL_DESKTOP_PLAN.md` §4.4，标签页内容落在真实 `.sql` 文件、SQLite 只存元数据、
AI 靠现成的 `ChangeStore`/`FileChangeCard` 流程改文件）**几乎是同一个模式**，可以直接复用同一套
基础设施，不用为 HTTP 桌面另起一套"AI 怎么改请求"的机制。但和 SQL 桌面不同的是：SQL 桌面的
"目录"是给每个数据源**合成**出来的应用内部缓存目录（数据源本身没有实体文件夹）；HTTP 桌面的
"目录"可以是一个**真实存在、用户自己选的项目文件夹**——这一点和 Bruno"集合就放在你的代码仓库里，
跟代码一起进 Git"的产品主张更贴近，也和 roc_desk 已有的「工作区」模块（本地/远程目录，本来就是
一个真实文件夹）天然同构，所以最终方案（§4.4）选择直接把 HTTP 集合数据落进已有的工作区目录，
而不是另起一个 `<collection_id>` 为 key 的应用缓存目录。剩下要决定的只是"用什么格式表示一个
请求"，§4.4 会展开对比"自造格式" vs "对齐 OpenCollection YAML"。

Bruno 的脚本沙箱同样值得参考：**Safe Mode 用 QuickJS 编译到 WASM 跑、完全隔离 Node.js
API（没有 `require()`、没有文件系统、没有网络，唯一的网络出口是显式的 `bru.sendRequest()`
钩子）**，CLI 默认也是 Safe Mode；只有显式打开的 Developer Mode 才用 Node VM 换取完整
Node.js 能力。这验证了 roc_desk 打算走的路线（§4.3：进程内嵌入 QuickJS，不给脚本默认的
文件系统/网络访问权限）是这类工具的行业共识做法，不是我们自己的冒险决定。

### 2.4 Hoppscotch——脚本 API 对齐 Postman `pm` 命名空间

Hoppscotch 的 `hopp-js-sandbox` 脚本引擎最值得参考的不是实现细节（它是 JS 生态自己的沙箱，
不能直接搬进 Rust 进程），而是一个产品决策：**脚本 API 命名空间直接兼容 Postman 的 `pm.*`**
（`pm.environment.get/set`、`pm.expect()` 走 Chai.js 断言风格、`pm.test()`），并且明确把
"导入 Postman 集合时把 pre-request/test 脚本原样保留"当成一个卖点。

roc_desk 的用户群（IT 从业者/开发者）大概率手里已经有存量的 Postman/Apifox 脚本片段。§4.3
的脚本引擎会照抄这个决策：暴露的全局对象叫 `pm`（或至少提供 `pm` 作为 `rd`/内部命名空间的别名），
API 形状对齐 Postman 公开文档里的子集（`pm.request`/`pm.response`/`pm.environment`/
`pm.variables`/`pm.expect`/`pm.test`），而不是发明一套 roc_desk 专属命名。这样用户从 Apifox/
Postman 粘贴脚本过来大概率不用改，显著降低迁移成本。

### 2.5 Insomnia——仅供 UI/交互参考

Insomnia 桌面端是 Apache-2.0（Kong 收购后维持开源，2025-11 发布的 Insomnia 12 仍然开源），
但它是 Electron + 自有存储引擎，技术栈和数据层都不贴合 roc_desk，**不作为代码移植来源**，只在
UI 设计阶段参考两点：环境变量的"多层作用域可视化"（哪个变量来自全局/哪个来自当前环境，鼠标悬浮
提示"实际生效值来自哪一层"）、请求 URL 栏对 `{{var}}` 的语法高亮与未定义变量的错误提示样式。

### 2.6 关键设计决策取舍表

| 子系统 | 采纳/参考来源 | 不采纳的做法 | 理由 |
|---|---|---|---|
| 组织维度/存储位置 | roc_desk 现有「工作区」模块（`WorkspaceManager`/本地或 SSH/Agent 远程目录），§3.1、§4.4 展开 | 仿 Yaak/Apifox 造一个独立于文件系统的"集合"实体 | 用户明确要求"以工作区为维度""最大程度复用现有工作区桌面代码"，零新增连接管理/远程文件读写代码 |
| 请求文件格式 | Bruno OpenCollection YAML 理念（磁盘文件即真相），§4.4 展开 | Yaak/Insomnia 的"纯 SQLite 行数据" | 与 roc_desk 现有 SQL 桌面 AI 编辑基础设施（`ChangeStore`）同构，直接复用；文件落在工作区目录内，天然可被 Git 跟踪 |
| 脚本引擎 | 进程内嵌 QuickJS（`rquickjs`），沙箱边界参考 Bruno Safe Mode | Yaak 的 Node.js 侧车 | 避免 vendor 完整 Node 运行时，保持便携版体积和"发射后不管"的进程模型简单 |
| 脚本 API 命名空间 | Hoppscotch/Postman 的 `pm.*` 命名空间 | 自造一套 API | 兼容用户存量脚本，迁移成本最低 |
| HTTP 执行引擎 | 项目已有的 `reqwest`（`rustls-tls` feature，`coding/webfetch.rs` 已在用） | 新引入 `hyper` 裸用或 `curl`/libcurl 绑定 | 零新增核心依赖，复用现成的 TLS/超时/重定向处理经验 |
| 导入/导出 | Postman Collection v2.1、OpenAPI/Swagger、curl、HAR；可选 OpenCollection YAML | 自造导入格式 | 覆盖用户最可能带着走的三种存量格式 |
| UI 信息架构 | 用户截图里的 Apifox 调试页布局（Params/Headers/Cookies/Auth/前置/后置/设置 + 响应区） | 照抄 Insomnia 的极简单栏布局 | 用户明确对标 Apifox，且这套多 Tab 布局已经是行业事实标准（Postman/Apifox/Insomnia 三者基本同构） |

## 3. 产品形态与信息架构

### 3.1 首页新增 HTTP 桌面区域——复用「工作区」的目录/连接机制

首页新增"HTTP 桌面"分区卡片（对齐 `docs/HOME_MODES_DESIGN.md` §3.4 的宫格首页，新增第六个
`WorkMode`）。这个分区**不建立独立的"集合列表"**，而是直接调用现有的
`workspace_list_recent`/`workspace_open_local`/`workspace_open_remote` command（`src-tauri/src/commands/workspace.rs`，
已经被"工作区"模块和首页"工作区"卡片在用）：

- **最近工作区**：和"工作区"卡片读取的是**同一份** `WorkspaceProfile` 列表（同一张 `workspaces`
  表），区别只是这里点一条记录时 `spawnModule("http", workspaceId)` 而不是
  `spawnModule("workspace", workspaceId)`——同一个目录，既可以点进去写代码，也可以点进去调
  HTTP 接口，两种入口互不冲突（参考 §2.6 决策表；一个目录可以同时是"代码工作区"和"接口
  调试工作区"，就像它现在已经可以同时承载编辑器/终端/SFTP/AI 编程助手一样）。
- **打开本地文件夹**：复用 `workspace_open_local`，前端复用现有的原生目录选择器（`tauri-plugin-dialog`）。
- **连接远程主机（SSH/Windows Agent）打开远程目录**：复用 `workspace_open_remote`，前端复用
  现有的连接选择 + 远程目录浏览 UI（`ConnectionDialog`/远程目录选择器组件）；`Protocol::Ssh`
  和 `Protocol::Agent` 两种连接档案都可以选，和"工作区"模块目前支持的完全一致——不需要为
  HTTP 桌面单独实现一套"新建远程连接"的向导。
- 首页卡片本身只做"跳转捷径"，新建/编辑连接、修改工作区路径等管理类操作全部复用工作区模块
  已有的弹窗（对齐 `docs/HOME_MODES_DESIGN.md` §3.4 的"不重新实现管理功能"原则）。

打开一个工作区之后，HTTP 桌面窗口看到的请求树/环境列表来自这个工作区目录下的
`.rock_desk/http/` 子目录（§4.4）；一个从没做过接口调试的工作区打开后是空的请求树，和"工作区"
模块打开一个还没写过代码的空文件夹是同一种"从零开始"的体验。

### 3.2 工作区内新建集合/环境

不再有独立的"新建集合向导"——集合的载体就是已经打开的工作区目录，用户在 HTTP 桌面左栏请求树
里直接操作：

1. 右键请求树根节点"新建集合"（对应工作区目录下 `.rock_desk/http/collections/<slug>/` 新建
   一个子目录，见 §4.4），一个工作区目录下可以有多个集合，用于"一个仓库里有多个后端服务、
   每个服务一套接口"这种场景；也可以什么都不建，直接在默认集合下新建请求，覆盖"整个工作区
   只有一套接口"的简单场景。
2. 可选：对着某个集合"导入"（Postman Collection JSON / OpenAPI YAML|JSON / curl 命令粘贴 /
   HAR 文件），导入结果落进该集合目录下的 `requests/`。
3. 集合下至少有一个环境（默认叫"默认环境"），可以继续新增更多环境（开发/测试/生产等）。

### 3.3 HTTP 工作区布局

```text
┌─ 工具栏：工作区名 | 集合 ▾ | 环境选择器 ▾ | 新建请求 | 导入 | 运行文件夹 ──────────┐
├──────────────┬──────────────────────────────────────────┬──────────────────┤
│ 请求树        │ URL 栏：METHOD ▾  https://host/path  [发送 ▾] [保存]           │
│ ▾ 文件夹 A    ├────────────────────────────────────────────┤ AI 工具            │
│   GET  /users │ Params│Headers│Cookies│Auth│Body│前置脚本│后置脚本│设置│文档│      │ 生成请求           │
│   POST /users │──────────────────────────────────────────────────────────│ 解释响应           │
│ ▾ 文件夹 B    │ （对应 Tab 的编辑区，Body 支持 none/form-data/               │ 生成断言/测试        │
│   ...         │  x-www-form-urlencoded/JSON/XML/Text/Binary/GraphQL）      │ 根据报错修复请求     │
│ 搜索请求      ├──────────────────────────────────────────────────────────┤ 从 curl/OpenAPI 生成 │
├──────────────┴──────────────────────────────────────────────────────────┤                    │
│ 响应区：状态码 耗时 大小 | Body(格式化/原始/预览) | Headers | Cookies | 测试结果 | 历史 │
└──────────────────────────────────────────────────────────────────────────┴────────────────────┘
```

- 左栏请求树支持拖拽排序、右键"复制为 curl"、右键"复制到其它文件夹"；文件夹可以设置
  自己的前置脚本/认证方式，子请求默认继承（对齐 Apifox/Postman 的文件夹级配置继承）。
- 中栏 URL 输入框对 `{{var}}` 做语法高亮，已定义变量正常色、未定义变量标红并给出"去哪个作用域
  新增"的快捷入口（参考 §2.5 Insomnia 的交互设计）。
- Body 编辑区里的 JSON/XML/Text 复用现有 Monaco 编辑器能力（语法高亮、格式化），不用重新造
  一个文本编辑控件；`form-data`/`x-www-form-urlencoded` 用键值对表格编辑器（可复用/改造 SQL
  桌面参数表格的样式）。
- 响应区默认展示格式化后的 Body（JSON 自动折叠/展开、图片/PDF 类型直接预览），"原始"模式展示
  未格式化的原文本，超大响应（见 §7 大小上限）截断并提示"仅预览前 N KB，导出获取全量"——这个
  "格式化/原始双模式、互不触发重新请求"的设计直接复用 SQL 桌面结果区"表格/纯文本"双模式的既有
  结论（`docs/SQL_DESKTOP_PLAN.md` §3.3）。
- 右栏 AI 工具与现有 AI provider 集成，可折叠；具体能力见 §8。

## 4. 技术架构

### 4.1 Rust domain 分层

比照 `src-tauri/src/sql/` 的组织方式新增 `http_desk` domain，但和 `sql` 不同的一点是它**直接
依赖 `crate::workspace`**（`WorkspaceManager`/`WorkspaceHandle`/`FileOps`）拿目录访问能力，
不自己管理连接和目录——这是本轮修订的核心：

```text
src-tauri/src/http_desk/
├── mod.rs
├── model.rs            # Collection/Environment/RequestDef/ExecuteResult 等 DTO
├── service.rs           # 集合发现（扫目录）、环境解析、变量解析编排——依赖注入的是
│                         # `Arc<dyn FileOps>`（来自已打开的 `WorkspaceHandle`），不是
│                         # 自己的连接池；集合/环境没有独立 SQLite CRUD，见 §4.4、§5
├── client.rs             # 基于 reqwest 的执行引擎：超时/重定向/代理/证书策略
├── script/
│   ├── mod.rs             # 脚本引擎入口：pre-request / post-response 两个钩子
│   ├── sandbox.rs         # rquickjs Context 创建、全局对象注入、执行超时
│   └── pm_api.rs          # 对齐 Postman 的 pm.* API 实现（见 §4.3、§2.4）
├── vars.rs                # 多层变量作用域解析（全局/集合/文件夹/环境/请求，见 §4.5）
├── dynamic_values.rs       # 内置动态值函数（基于 `fake` crate，见 §4.6）
├── import/
│   ├── postman.rs          # Postman Collection v2.1 导入
│   ├── openapi.rs           # OpenAPI/Swagger 导入
│   ├── curl.rs               # curl 命令解析导入
│   └── har.rs                  # HAR 导入
├── export.rs                # Postman/OpenCollection YAML 导出
├── history.rs                 # 请求历史
└── mock/                        # Phase 3+，见 §9
    └── mod.rs
```

`src-tauri/src/commands/http_desk.rs` 只暴露 command，**不重复实现打开工作区/管理连接的
command**——那些继续走 `commands/workspace.rs` 现成的 `workspace_open_local`/`workspace_open_remote`/
`workspace_list_recent`/`workspace_update_path`/`workspace_remove_recent`。前端新增：

```text
src-web/src/
├── components/HttpDesk/
│   ├── HttpDeskHome.tsx               # 复用 `workspaceService`/`useModeStore`，UI 结构
│   │                                    # 参照现成的「工作区」卡片，不新建 workspace 相关状态
│   ├── CollectionExplorer.tsx        # 左栏请求树（含"新建集合"，§3.2）
│   ├── HttpWorkspace.tsx
│   ├── RequestEditor/
│   │   ├── UrlBar.tsx
│   │   ├── ParamsTab.tsx / HeadersTab.tsx / CookiesTab.tsx / AuthTab.tsx
│   │   ├── BodyTab.tsx               # 复用 Monaco，按 Content-Type 切子编辑器
│   │   └── ScriptTab.tsx             # 前置/后置脚本，复用 Monaco JS 语言模式
│   ├── ResponsePanel/
│   │   ├── ResponsePanel.tsx
│   │   ├── ResponseBodyView.tsx      # 格式化/原始双模式
│   │   └── TestResultsView.tsx
│   ├── EnvironmentManager.tsx
│   └── HttpAiPanel.tsx
├── services/httpDesk/{collection,environment,request,history,importExport}.ts
│                       # collection.ts 只管"工作区内的集合发现/新建"，打开工作区本身继续用
│                       # 现成的 `services/workspaceService.ts`，不重复封装
├── stores/httpDeskStore.ts
└── types/httpDesk.ts
```

### 4.2 请求执行引擎

直接复用项目里已经在用的 `reqwest`（`Cargo.toml` 已启用 `rustls-tls`/`json`/`stream`
feature，`coding/webfetch.rs` 已经用它做 AI 的网页抓取工具），不新增 HTTP 客户端依赖：

- 每个集合持有一个 `reqwest::Client`（连接池/Cookie Jar 按集合隔离，不同集合之间不共享
  Cookie，符合"每个集合代表一个独立被测系统"的直觉），按环境切换时不需要重建 client，
  只是解析变量时用不同环境的取值。
- 显式可配置：跟随重定向开关及跳数上限、连接/读取超时、系统代理/自定义代理、TLS 证书校验
  开关（关闭时在 UI 上给出醒目的"不安全"提示，绝不能默认关闭，对齐 SQL 桌面方案 §7 的
  "禁止默认关闭证书校验"原则）、自定义 CA 证书导入。
- 大响应走流式读取到临时文件 + 前端分块展示，不整体 `bytes()` 进内存，超过阈值截断预览
  （具体上限见 §7）。
- 请求发出前先跑前置脚本（可能改写 URL/Headers/Body/变量），发出后再跑后置脚本（读响应、
  写断言结果、提取变量写回目标作用域）——脚本执行本身见 §4.3。

### 4.3 脚本引擎：进程内嵌入 QuickJS 沙箱

对应 §2.2/§2.3/§2.4 的调研结论，脚本引擎方案：

- **运行时**：`rquickjs`（MIT，QuickJS 的 Rust 高层绑定），在 Rust 进程内直接创建
  `rquickjs::Context` 执行脚本，不起子进程、不 vendor Node.js。QuickJS 体积小
  （几百 KB 级别），编译时静态链接进 `roc_desk.exe`，符合现有"单文件便携"的打包哲学。
- **沙箱边界**（对齐 Bruno Safe Mode 的做法，§2.3）：默认**不注入** `require`/文件系统/
  子进程/原生网络 API；脚本能访问网络的唯一方式是显式提供的 `pm.sendRequest(...)` 风格
  钩子，这个钩子在 Rust 侧统一走 §4.2 的执行引擎（复用同一份超时/证书策略），不是脚本自己
  发起裸 TCP 连接。
- **暴露的全局 API**（对齐 §2.4 的 Postman `pm.*` 命名空间决策）：
  - `pm.request` / `pm.response`：只读/可写视图，前置脚本能改 `pm.request`，后置脚本能读
    `pm.response`。
  - `pm.environment.get/set/unset`、`pm.collectionVariables.get/set`、`pm.globals.get/set`：
    对应 §4.5 的多层作用域，写操作只影响脚本执行完之后落盘的那一层，不会"偷偷"跨层写。
  - `pm.variables.get`：只读，按 §4.5 的优先级解析后的最终值。
  - `pm.expect(value)`：实现 Chai.js `expect` 断言语法的一个**子集**（`.to.equal()`/
    `.to.include()`/`.to.have.status()`/`.to.be.above()` 等常用断言），不追求 100% 复刻
    Chai——完整移植 Chai 到 QuickJS 沙箱成本高、大部分用户只用得到十几个常用断言。
  - `pm.test(name, fn)`：注册一条测试用例，异常被捕获记为失败，结果汇总进响应区"测试结果"
    Tab。
- **执行限制**：脚本执行有硬超时（如 5 秒）和内存/调用栈保护（QuickJS 原生支持设置
  `set_memory_limit`/`set_max_stack_size`），超限直接中断并在结果里标注"脚本超时/超出资源
  限制"，不允许一个失控脚本卡死整个请求发送流程。
- 前置/后置脚本文本存放位置见 §4.4（作为请求文件的一个字段/子块，随请求一起落盘，AI 可以
  像改请求其它字段一样改脚本）。

> 备选方案说明：`boa_engine`（纯 Rust 实现，无需 C 工具链，跨平台交叉编译更省心，这点对
> `docs/MACOS_PORT_PLAN.md` 里的 macOS 移植计划有潜在好处）是另一个候选。首期建议先用
> `rquickjs`（QuickJS 生态更成熟、Bruno 已经在生产验证过 QuickJS 作为沙箱的可行性），
> 如果后续 macOS 移植阶段发现 C 工具链交叉编译是实际障碍，再评估切到 `boa_engine`——两者
> 暴露给 `pm_api.rs` 的 Rust trait 接口保持一致，替换脚本引擎不影响上层。

### 4.4 存储：落在已打开的工作区目录里，走 `FileOps`（本地/SSH/Agent 通用）

这是本轮修订相对第一版方案最大的改动。第一版把集合数据放在应用内部的 `<cache_root>/http/`
缓存目录，只能是本机路径；本节改成**直接借用已打开的 `WorkspaceHandle`**（打开方式见 §3.1，
就是现有 `workspace_open_local`/`workspace_open_remote` 返回的那个 handle），所有读写走它带的
`Arc<dyn FileOps>`——本地工作区自动是 `LocalFileOps`，远程工作区自动是 `RemoteFileOps`（SSH）
或 `AgentFileOps`（Windows Agent），HTTP 桌面自己不需要判断"这是本地还是远程"、也不需要自己
建任何连接，`workspace` 模块已经把这层差异完全封装掉了（`src-tauri/src/fsops/mod.rs` 的
`FileOps` trait 本来就是"Explorer、SFTP、AI 编程助手三处共用"的通用接口，见该文件顶部注释）。

**目录结构**（相对已打开工作区的 `root_path`，通过 `file_ops.read_file`/`write_file`/
`list_dir`/`create_dir` 读写，和"工作区"模块在同一个根目录下已经维护的 `.rock_desk/workspace.json`
是邻居）：

```text
<workspace_root>/.rock_desk/http/
├── collections/
│   └── <collection_slug>/
│       ├── collection.yaml        # 集合级元数据：名称、描述、集合级认证、集合级前置脚本
│       ├── environments/
│       │   └── <env_id>.yaml      # 每个环境一个文件：变量键值对（敏感变量只存 credential_ref）
│       └── requests/
│           ├── <folder_path>/
│           │   └── <request_id>.yaml  # 一个请求一个文件
│           └── ...
└── global.yaml                     # 跨集合共享的全局变量（§4.5 第 5 层）
```

集合的"列表"不需要一张 SQLite 表去维护——`http_list_collections`（§6）就是对
`.rock_desk/http/collections/` 做一次 `file_ops.list_dir` + 读各自的 `collection.yaml`，
和 Bruno 自己"扫描文件夹发现集合"的做法一致（§2.3），这样"目录里手动加/删一个集合文件夹"
也能被正确识别，不会和数据库状态打架。

**执行历史/标签页这类瞬时 UI 状态不写回工作区目录**：请求/集合/环境这些"用户内容"必须走
`FileOps` 落进工作区（可能是远程主机），但请求历史快照、AI 会话快照这类高频写入、体量可能
较大、纯粹服务于本机 UI 的数据，沿用 `WorkspaceHandle.fallback_cache_dir`（`workspace/mod.rs`
已经有的字段，`<app 数据目录>/cache/<workspace_id>/`）——这样发一次请求不会触发一次远程
SFTP/Agent 写文件，历史记录浏览也不需要每次都往返远程主机：

```text
<fallback_cache_dir>/http/
├── history/
│   └── <history_id>.json   # 执行历史快照（含完整请求/响应，含大响应体的本地落盘路径）
└── sessions/
    └── <history_id>.json   # AI 对话历史快照，格式对齐现有 WorkspaceHistorySnapshot
```

> 这一点和"工作区"模块处理远程只读目录时"工作区身份退回本机缓存"的降级思路是同一个原则
> （`workspace/mod.rs` `open_remote` 的注释：远端不可写时退回本机缓存，但 `FileOps` 仍然
> 严格使用远程实现，不会偷偷切回本地文件系统）——HTTP 桌面把这个原则应用得更彻底：**用户
> 内容永远走 `FileOps`（可能远程），瞬时状态永远走本机缓存**，两者不混在一起。

**请求文件格式**：采用对齐 [OpenCollection YAML](https://spec.opencollection.com)
（§2.3 调研的开放规范，2026-01 由 Bruno 团队独立发布、公开授权、附 JSON Schema）的字段设计，
不是自造一套私有 DSL。理由：

1. **实现成本低**：YAML 是成熟格式，Rust 用 `serde` + `serde_yaml` 解析/序列化，不需要像
   `.bru` 那样自己写一个块状标记语言的解析器。
2. **Diff 友好**：YAML 天然逐行、缩进表达层级，`similar` crate（`ChangeStore` 已经在用）
   对 YAML 文本做行级 diff 的效果和对 SQL 文本类似，不会出现"改一个字段导致整个 JSON 括号
   结构抖动"的噪音 diff。
3. **生态兼容**：请求文件天然是"合法的 OpenCollection YAML 子集"，`export.rs`（§4.7）
   导出时可以做到近似"零转换"，用户也能直接拿现成的 `@opencollection/converters` 类工具
   校验/转换，不是一个孤岛格式；文件本身就落在用户的工作区目录里，用户直接拿 Git 管理这个
   `.rock_desk/http/` 子目录（或者以后需要的话把它挪到项目里任意可见路径）也完全可行。
4. 具体字段以实现阶段从 `spec.opencollection.com` 拉取的最新 Schema 为准（该规范发布不久，
   建议实现前重新核对一次字段名，不在本方案里锁死一份可能已经过期的示例）；roc_desk 不需要
   实现 OpenCollection 的全部能力（比如它的多协议扩展），只需要覆盖 REST 请求这个子集。

**标签页 = 文件，路径创建时就定好**：同 SQL 桌面方案的结论，新建请求时立刻分配 `request_id`
并通过 `file_ops.write_file` 建好对应 `.yaml` 文件（哪怕内容是空模板），不等到"保存"才落盘，
这样 `ChangeStore::stage()` 按路径做身份匹配才能从第一次编辑起就正常生成 diff。

**AI 工具复用与新增**：

- `read_file`/`write_file`/`edit_file`/`list_directory`/`glob`/`search_files` 原样复用现有
  `coding/tools.rs` 定义——这组工具本来就是基于 `FileOps` 抽象实现的，天然支持本地/SSH/Agent
  三种工作区，不需要 HTTP 桌面自己适配远程读写。作用域限定在
  `<workspace_root>/.rock_desk/http/` 内，路径越界校验做法照抄 SQL 桌面方案已经补上的教训
  （比照 `commands/fs.rs::guard_local_path` `canonicalize()` + `starts_with()` 检查，远程路径
  用字符串前缀校验 + 规范化，不能沿用编程助手现状"无边界"的实现）。
- "生成请求"/"根据报错修复请求"/"生成断言脚本"本质上是对 `requests/<id>.yaml` 做一次
  `edit_file`，走 `stage_change` → `FileChangeCard`（应用/拒绝/整轮撤销），和 SQL 桌面
  AI 改 `.sql` 文件走的是完全同一套前端组件和撤销语义，不需要为 HTTP 桌面单独写一套
  Pending/Applied/Rejected/Undone 状态机。
- 新增一组 HTTP 专属工具（`http_send_request`/`http_run_folder`/`http_list_history`），
  不走文件系统，直接调用 §6 的 IPC command；AI 发起的"发送请求验证自己的改动"和人手动点
  "发送"按钮走同一条执行路径，不给 AI 开小灶式的隐藏发送通道。
- 敏感变量（token、密码类环境变量）不允许 AI 上下文读到明文：`vars.rs` 解析变量时，写入
  AI 可见上下文之前，对标记为 `secret: true` 的变量做统一遮罩（见 §7）。

**这个工作区如果同时被"工作区"模块（编程助手）打开呢？** 完全没问题，且是设计上刻意允许的：
`coding_sessions`（`state.rs`）和 HTTP 桌面各自的 AI 会话都是按 `workspace_id` 独立 keyed 的
（参考 `state.coding_sessions: HashMap<workspace_id, ...>` 的既有模式），两个模块窗口是两个
独立进程、各自独立的 `AppState`（`docs/HOME_MODES_DESIGN.md` §2.2），互不共享内存态；`.rock_desk/`
目录下 `workspace.json` 的写入本身是幂等的（内容只由 `workspace_id`/`kind`/`root_path`/
`connection_id` 决定），两个进程各自打开同一个工作区不会互相踩踏。编程助手的通用文件工具如果
被引导去看 `.rock_desk/http/`，也只是"看到另一个子目录"，不会误改——这属于用户自己决定要不要
让编程助手碰这部分文件，不需要在权限引擎里做特殊隔离。

### 4.5 环境与变量作用域

多层作用域，解析优先级从高到低（就近覆盖，与 Postman/Apifox 一致）：

1. 请求级局部变量（脚本运行期临时写入，不落盘，仅当前一次执行有效）
2. 环境变量（当前选中的 `environments/<env_id>.yaml`）
3. 文件夹变量（请求所在文件夹链路上、从近到远逐层查找）
4. 集合变量（`collection.yaml`）
5. 全局变量（跨集合共享，`<workspace_root>/.rock_desk/http/global.yaml`，见 §4.4）

插值语法统一用 `{{ var }}`，同时支持内置函数调用形式 `{{ $uuid }}`/`{{ $timestamp }}`
（对齐 §2.2 Yaak 的"内置函数和变量用同一套插值语法"决策，不像 Apifox 那样"动态值"要单独
从右键菜单插入不同的占位符语法）。敏感变量（`secret: true`）在 UI 上默认打码显示、日志和
AI 上下文里统一替换成 `***`。

### 4.6 内置动态值

基于 Rust `fake` crate（MIT，社区维护的 Faker 实现）暴露一组内置函数，覆盖 Apifox"动态值"
里最常用的几类：`$uuid`、`$timestamp`、`$isoTimestamp`、`$randomInt`、`$randomFullName`、
`$randomEmail`、`$randomPhoneNumber`、`$randomIP` 等，命名对齐 Postman 内置动态变量习惯
（同样是为了降低迁移成本）。脚本里也能通过 `pm.variables.replaceIn(str)` 之类的辅助函数
触发同一套插值逻辑。

### 4.7 导入/导出

| 方向 | 格式 | 说明 |
|---|---|---|
| 导入 | Postman Collection v2.1 JSON | 覆盖率最高，含 folders/pre-request/test 脚本原样保留（脚本文本落进 `.yaml` 的 script 字段，后续走 §4.3 的沙箱执行，`pm.*` API 对齐使得大部分脚本无需改写） |
| 导入 | OpenAPI/Swagger（YAML/JSON） | 按 path+method 生成请求骨架，schema 生成示例 Body，不生成脚本/断言 |
| 导入 | curl 命令 | 解析成单个请求，用户粘贴一段 curl 文本即可 |
| 导入 | HAR | 从浏览器导出的 HAR 生成请求列表，用于"从已发生的真实流量反推接口" |
| 导出 | Postman Collection v2.1 JSON | 方便迁回 Postman/分享给还在用 Postman 的同事 |
| 导出 | OpenCollection YAML | 因为落盘格式本身已经对齐该规范（§4.4），导出近似"打包目录" |

## 5. 数据模型与持久化

因为集合/环境/请求的"内容"都不落 SQLite（§4.4，真相在工作区目录的 `.yaml` 文件里，集合列表
靠扫目录发现），这里不需要 `http_collections`/`http_environments` 表——比 SQL 桌面方案的
SQLite 表还要精简一步。在现有 SQLite migration 后追加 `00XX_http_desktop.sql`（当前仓库最新
迁移是 `0021_sql_agent_history.sql`；实现时以届时最新编号为准），只需要两张纯 UI/审计状态表，
都用**已有的** `workspaces.id`（`workspace_repo.rs`）做外键，不新增任何"工作区/连接"相关的表：

```sql
CREATE TABLE http_workspace_tabs (
  id TEXT PRIMARY KEY,
  workspace_id TEXT NOT NULL,     -- 复用 workspaces 表（workspace/profile.rs），不新增集合表
  collection_slug TEXT NOT NULL,  -- 对应 .rock_desk/http/collections/<slug>/
  request_id TEXT NOT NULL,       -- 对应 requests/<...>/<request_id>.yaml
  title TEXT NOT NULL,
  sort_order INTEGER NOT NULL DEFAULT 0,
  updated_at TEXT NOT NULL,
  FOREIGN KEY(workspace_id) REFERENCES workspaces(id) ON DELETE CASCADE
);

CREATE TABLE http_request_history (
  id TEXT PRIMARY KEY,
  workspace_id TEXT NOT NULL,
  collection_slug TEXT NOT NULL,
  request_id TEXT,                -- 请求被删除后历史仍保留，允许为空
  environment_id TEXT,
  method TEXT NOT NULL,
  url TEXT NOT NULL,
  status_code INTEGER,
  duration_ms INTEGER,
  response_size_bytes INTEGER,
  error_message TEXT,
  snapshot_path TEXT NOT NULL,    -- 指向 <fallback_cache_dir>/http/history/<id>.json（§4.4）
  created_at TEXT NOT NULL,
  FOREIGN KEY(workspace_id) REFERENCES workspaces(id) ON DELETE CASCADE
);
```

（`workspaces` 表的具体列结构见现有 `db/repo/workspace_repo.rs`/`workspace/profile.rs`，本方案
不改动它。）`http_workspace_tabs` 不存请求正文——正文的唯一真相是 `requests/<...>/<request_id>.yaml`
（§4.4），这张表只存"当前打开了哪些标签页、什么顺序"这类 UI 状态，与 SQL 桌面
`sql_workspace_tabs` 的设计原则一致。敏感值（认证 token 等）不落 SQLite 明文，走系统 keyring，
数据文件里只存 `credential_ref`，对齐 SQL 桌面方案的凭据约定。

## 6. IPC/API 设计

打开工作区本身**不新增 command**，复用 §3.1 提到的 `workspace_list_recent`/`workspace_open_local`/
`workspace_open_remote`（`commands/workspace.rs`）。以下是 HTTP 桌面新增的、工作区打开之后才用得到的
command：

| Command | 作用 |
|---|---|
| `http_list_collections` | 对已打开工作区的 `.rock_desk/http/collections/` 扫目录，返回集合列表（名称、请求数量统计），不是数据库查询 |
| `http_create_collection` / `http_delete_collection` / `http_rename_collection` | 集合 CRUD，本质是对 `.rock_desk/http/collections/<slug>/` 目录的创建/删除/改名，走 `file_ops` |
| `http_import_collection` | 从 Postman/OpenAPI/curl/HAR 导入，返回导入摘要（成功/跳过/失败计数） |
| `http_export_collection` | 导出为 Postman/OpenCollection YAML |
| `http_list_requests` | 懒加载请求树 |
| `http_create_request` / `http_delete_request` / `http_move_request` | 请求 CRUD 与拖拽排序 |
| `http_list_environments` / `http_save_environment` / `http_delete_environment` | 环境 CRUD |
| `http_send_request` | 发送请求（含前后置脚本执行），返回 `request_execution_id`，通过事件流分块传递响应体 |
| `http_cancel_request` | 取消正在进行的请求（中断底层 `reqwest` future 并关闭连接） |
| `http_run_folder` | 批量运行一个文件夹下的请求（Collection Runner），逐条返回进度事件 |
| `http_query_history` | 查询/清理历史 |
| `http_resolve_variables` | 调试用：返回某个请求在当前环境下变量插值后的最终结果（不实际发送），方便用户排查"变量怎么没生效" |
| `http_ai_generate` / `http_ai_explain_response` / `http_ai_generate_tests` / `http_ai_fix_request` | AI 能力入口，见 §8 |

大响应体不通过单次 invoke 返回：`http_send_request` 返回 `request_execution_id`，通过
`http://request/{id}/chunk`、`.../finished`、`.../error` 事件分块传递，对齐 SQL 桌面
`sql_execute` 的既有事件流约定（`docs/SQL_DESKTOP_PLAN.md` §6），不重新发明一套传输模式。

## 7. 安全与治理

- **敏感信息脱敏**：`Authorization` 头、标记为 `secret` 的环境变量、Cookie 里的 session
  token 类字段，在写入历史记录、写入日志、发送给 AI 之前统一替换为 `***`；仅在当前会话内存
  中和实际发出的 HTTP 请求里使用明文，不落盘明文（敏感变量的存储走系统 keyring + 数据文件里
  只存 `credential_ref`，与 SQL 桌面数据源密码的处理方式一致）。
- **脚本沙箱边界**（§4.3）：默认无文件系统/子进程/裸网络访问，唯一网络出口是走统一执行引擎
  的 `pm.sendRequest`；执行有硬超时和资源上限，防止失控脚本卡死或耗尽内存。
- **TLS 与证书**：默认校验服务器证书，关闭校验需要用户在该请求/环境级别显式打开并在 UI 持续
  展示醒目的"不安全连接"标记；支持导入自定义 CA、支持客户端证书（mTLS）用于内部/测试环境。
- **响应体大小与超时**：单响应体默认预览上限（如 5MB），超限截断预览、仅在导出时落临时文件
  取全量；请求超时可按集合/环境配置，超限自动取消并提示，参照 SQL 桌面对"测试连接"加超时兜底
  的既有教训（`commands/sql.rs` 的 `TEST_CONNECTION_TIMEOUT` 说明），HTTP 发送同样不能允许
  防火墙静默丢包时前端永远卡在"发送中"。
- **危险操作确认（可选特性）**：环境可标记为"生产"，对该环境下 `DELETE`/`PUT`/`PATCH`/
  非幂等 `POST` 类请求发送前弹二次确认，类比 SQL 桌面"生产环境写操作二次确认"的产品直觉，
  但 HTTP 请求本身就是这个工具的核心功能（不像 SQL 桌面默认只读），所以这里是**可选开启**
  的保护开关，不是默认策略。
- **AI 上下文脱敏**：AI 生成请求/解释响应时，发给模型的上下文默认不包含明文 secret 变量和
  `Authorization` 头的实际值，只发"这里有一个敏感变量，名字叫 X"这类结构信息。
- **导入来源校验**：导入 Postman/OpenAPI/HAR 文件时按大小和条目数做上限保护，避免一个
  异常构造的超大导入文件把 UI/数据库拖垮。

## 8. AI 工具设计

右侧 AI 面板首期提供（复用 §4.4 提到的 `ChangeStore`/`FileChangeCard` 流程，AI 对请求文件的
改动同样要经过用户"应用/拒绝/整轮撤销"确认，不直接覆盖编辑区内容）：

1. **生成请求**：根据自然语言描述、粘贴的 curl 命令或一段 OpenAPI 片段生成请求定义
   （URL/方法/Headers/Body），回填为一次待确认的文件改动。
2. **解释响应**：结合请求上下文解释状态码含义、响应结构、可能的错误原因。
3. **生成断言/测试脚本**：根据响应示例生成后置脚本里的 `pm.test(...)` 断言片段。
4. **根据报错修复请求**：把发送失败的错误信息（连接超时、4xx/5xx、脚本异常栈）连同请求定义
   发给模型，生成修复后的版本供确认。
5. **从 OpenAPI/curl 一键生成一组请求**：批量场景，把导入流程（§4.7）和 AI 结合，导入后
   AI 可选地补全示例 Body、补充常见断言。

**交互机制**：与 SQL 桌面 AI 面板完全同构——上面 1/3/4 项都是"对当前请求 `.yaml` 文件做一次
`edit_file`"，走 `stage_change` → `FileChangeCard`；AI 如果要验证自己的改动（比如生成后立刻
发送一次请求看结果），用 §4.4 提到的 `http_send_request` 工具，走和人手动点击"发送"完全相同
的执行路径，不存在 AI 专属的隐藏发送通道。AI 工具的文件读写作用域硬限制在
`<workspace_root>/.rock_desk/http/` 内（本地或远程，取决于当前工作区，见 §4.4）。

## 9. Mock 服务（Phase 3+）

首期不做（§1.2），后续如果排期允许，最简版本：

- 每个集合可选启动一个本地 Mock 服务（用项目里其它模块已经在用的异步生态起一个轻量
  HTTP server，按 `path + method` 匹配到某个请求定义，返回该请求里预先配置的"示例响应"）。
- 匹配规则只支持精确路径 + 路径参数占位符（`/users/:id`），不做 Apifox 那种基于 JSON Schema
  推断字段类型的"智能 Mock"，示例响应由用户手动填或从一次真实响应"另存为示例"生成。
- Mock 服务默认监听 `127.0.0.1` 的随机端口，不默认对外网暴露。

## 10. 实施分期与验收

### Phase 0：基础设施（1 周）

- 新增 `WorkMode = "http"`（`modeStore.ts`/首页分区/模块窗口挂载点，复用 §3.1 的
  `workspace_list_recent`/`workspace_open_local`/`workspace_open_remote`，不新写工作区打开逻辑）、
  完成 migration、`http_desk` domain 骨架（含基于 `FileOps` 的集合扫描）、`reqwest` 执行引擎
  最小实现（发单个 GET 请求跑通）。
- 验收：能从首页打开一个本地工作区、新建集合、新建请求、发送一个 GET 请求并看到状态码和响应体；
  能再打开一个远程（SSH 或 Agent）工作区验证同一套流程在远程目录下也能读写集合文件。

### Phase 1：MVP 调试工作区（2~3 周）

- 集合/环境 CRUD（走 `FileOps`，无独立数据库表）、请求树、URL 栏 + Params/Headers/Cookies/Auth/Body
  多 Tab 编辑、响应区（格式化/原始双模式）、历史记录、变量插值（§4.5 全部 5 层作用域）。
- 验收：常见 REST 场景（GET/POST JSON/表单上传/Bearer Token 鉴权）全部可用；大响应体不阻塞 UI；
  同一个工作区目录能同时被"工作区"模块和"HTTP 桌面"模块打开，互不干扰。

### Phase 2：脚本与导入导出（2 周）

- QuickJS 沙箱（§4.3）、`pm.*` API、前置/后置脚本、断言/测试结果展示；Postman/OpenAPI/curl/
  HAR 导入，Postman/OpenCollection YAML 导出。
- 验收：能导入一份真实的 Postman Collection（含脚本）并跑通；脚本超时/异常有清晰提示。

### Phase 3：AI 与体验增强（1~2 周）

- AI 生成/解释/断言/修复，复用 `ChangeStore` 状态机；批量运行文件夹（Collection Runner）；
  Mock 服务最简版（如排期允许）。

### 测试矩阵

- Rust：变量作用域解析、脚本沙箱边界（确认无法访问文件系统/发起裸网络连接）、导入解析器
  （Postman/OpenAPI/curl/HAR 各准备典型样例）、凭据引用、错误映射单测。
- 前端：请求树拖拽排序、URL 变量高亮、响应区双模式渲染、断线/超时组件测试。
- 安全：脚本逃逸尝试（`require`/`eval` 是否被正确拒绝）、敏感信息脱敏扫描（历史/日志/AI 请求
  体里不出现明文 secret）、TLS 证书错误场景、导入超大文件的保护上限。

## 11. 风险与决策点

| 风险 | 应对 |
|---|---|
| OpenCollection YAML 规范刚发布（2026-01），字段可能仍在变动 | 实现前重新核对 `spec.opencollection.com` 最新 Schema；roc_desk 只落地一个 REST 子集，即便规范后续扩展也不影响已落地字段的兼容性 |
| QuickJS 沙箱能力子集覆盖不了所有存量 Postman 脚本（比如用到了 Node 特有 API） | 明确产品预期为"覆盖常见脚本模式"，不承诺 100% 兼容；导入时对无法解析/执行报错的脚本原样保留文本但标记"需要人工检查" |
| 大量并发发送请求（Collection Runner）导致资源耗尽 | 限制并发数，复用 §4.2 的超时/大小上限，允许用户中途取消整批运行 |
| 脚本引擎 DoS（死循环/超大内存分配） | §4.3 的硬超时 + QuickJS 原生内存/栈限制,超限直接中断 |
| 敏感信息意外落盘或发给 AI | §7 统一脱敏点位（历史、日志、AI 上下文三处都要覆盖），补充自动化扫描测试 |
| 与 SQL 桌面共用的 `ChangeStore`/AI 基础设施如果后续重构，两个模块要同步改 | 抽象层面保持一致的调用方式（都是"目录 + 路径校验 + stage_change"），减少后续重构时的分叉成本 |
| 远程工作区场景下，"集合文件存在远端"和"请求实际从哪里发出"容易被用户误解——首期请求始终从运行 roc_desk 的本机发出，只是定义文件读写走远程 `FileOps`；用户如果是想"从远程主机的网络位置发起请求"（比如测试只有内网 SSH 跳板机能访问到的接口），首期不满足 | 文档/UI 上明确说明"请求从本机发出"；把"通过 SSH/Agent 隧道从远程主机实际发出请求"列为后续可选增强（需要复用 `ssh_pool`/`agent_pool` 的命令执行或端口转发能力），不在首期承诺 |
| HTTP 桌面新增读写落在"工作区"模块已有的 `.rock_desk/` 目录下，理论上有和未来"工作区"模块自身改动产生目录结构冲突的可能 | `.rock_desk/http/` 是独立子目录，只要"工作区"模块不改变 `.rock_desk/workspace.json` 之外的既有写入行为，两者不会互相覆盖；实现前在 `workspace/mod.rs` 留一行注释说明这个子目录被 HTTP 桌面使用 |

## 12. 首批开发任务清单

1. 确认现有 SQLite migration 最新编号并创建 HTTP 桌面 migration（只需 `http_workspace_tabs`/
   `http_request_history` 两张表，见 §5）。
2. 新增 `WorkMode = "http"`，接入 `spawn_module_window`/模块窗口挂载点；建立 `http_desk`
   domain（依赖注入 `Arc<dyn FileOps>`，不新建连接管理）、`reqwest` 执行引擎骨架、错误码和
   Tauri commands。
3. 落地 §4.4 的目录结构和 OpenCollection YAML 风格的请求文件读写（`serde_yaml` 序列化/
   反序列化 + 基于 `FileOps` 的路径越界校验，本地/远程都要覆盖）。
4. 在首页 HTTP 桌面卡片接入 §3.1 复用的"最近工作区"列表和工作区打开流程；实现工作区内
   "新建/导入集合"交互。
5. 实现 `HttpWorkspace` 布局：请求树、URL 栏、Params/Headers/Cookies/Auth/Body 多 Tab、
   响应区双模式渲染。
6. 接入 §4.5 的多层变量作用域解析和 `{{ }}` 插值/语法高亮。
7. 集成 `rquickjs`，落地脚本沙箱边界（§4.3）和 `pm.*` API 子集。
8. 依次接入 Postman/OpenAPI/curl/HAR 导入解析器，以及 Postman/OpenCollection YAML 导出。
9. 复用现有 AI provider 和 `ChangeStore`，落地 §8 的 AI 能力（生成/解释/断言/修复）。
10. 接入历史记录、安全脱敏（§7）、Collection Runner 批量运行。
11. 更新用户文档、README 功能列表（去掉"后续规划"标注）、故障排查手册。

## 13. 完成定义（Definition of Done）

- 用户可从首页打开一个本地目录或连接 SSH/Windows Agent 打开一个远程目录作为工作区（复用现有
  "最近工作区"列表和打开流程），在该工作区内新增/导入 HTTP 集合，完成常见 REST 场景的调试
  （GET/POST JSON/表单/文件上传/多种鉴权方式）；同一个目录可以既是代码工作区又是 HTTP 桌面
  工作区，两者互不干扰。
- 多层变量作用域和内置动态值可用，`{{ }}` 插值在 URL/Headers/Body/脚本里统一生效。
- 前置/后置脚本在沙箱内执行，断言结果清晰展示；脚本无法访问文件系统或发起沙箱外网络连接。
- 请求发送超时、取消、错误都有可理解的中文提示，且不会阻塞其它工作区。
- 敏感信息（token/密码类变量、`Authorization` 头）不落明文日志、不出现在 AI 请求体里。
- AI 对请求文件的改动都经过 Pending/Applied/Rejected/Undone 状态机确认，复用 SQL 桌面已
  验证的 `ChangeStore`/`FileChangeCard` 流程，没有绕过用户确认直接改写编辑区内容的路径。
- Postman Collection 导入导出可用，能覆盖用户从 Apifox/Postman 迁移过来的主流程。
- `npm run test:web`、`npm run test:rust` 通过；脚本沙箱逃逸测试、敏感信息脱敏扫描通过。
