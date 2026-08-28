# roc_desk

集成开发者客户端工具：SSH 终端 + SFTP + 日志搜索 + 统一 AI 工具（问答与编程），基于 Tauri 2.0（Rust 后端 + React/TS 前端）。

完整设计见 [DESIGN.md](DESIGN.md)、[docs/CODE_DESIGN.md](docs/CODE_DESIGN.md)、[docs/UI_DESIGN.md](docs/UI_DESIGN.md)；需求与实现状态见 [REQUIREMENTS.md](REQUIREMENTS.md)。

## 目录结构

```
roc_desk/
├── src-tauri/     # Rust 后端（Tauri 应用外壳 + 所有 domain 逻辑）
├── src-web/       # React + TypeScript 前端
├── package.json   # 根目录构建入口（转发到 src-web / src-tauri）
└── docs/          # 设计文档与 UI 原型
```

## 前置依赖

- [Rust](https://rustup.rs)（`rustc`/`cargo`，Windows 上还需要 MSVC 生成工具，装 Visual Studio Build Tools 或完整 VS 即可）
- [Node.js](https://nodejs.org) 18+（含 npm）
- Windows 10/11 自带 WebView2 运行时，一般无需额外安装

## 开发模式

```bash
npm install --prefix src-web   # 只需在首次或依赖变更后执行一次
npm run tauri:dev              # 在仓库根目录执行；前端热重载 + 后端自动重编译
```

`npm run tauri:dev` 内部等价于 `cargo tauri dev`，会先跑 `beforeDevCommand`（`npm run dev --prefix src-web` 启动 Vite dev server），再启动 Rust 后端并打开应用窗口。

**必须在仓库根目录（`roc_desk/`）下执行**——因为 `src-tauri/` 和 `src-web/` 是同级目录而非嵌套关系，Tauri CLI 从根目录出发才能同时找到两边。在 `src-tauri/` 或 `src-web/` 目录内直接跑 `tauri dev` 会报"找不到 Tauri 项目"。

## 构建 Windows 安装包 / EXE

```bash
npm run tauri:build            # 在仓库根目录执行
```

这一步会：
1. 跑 `beforeBuildCommand`（`npm run build --prefix src-web`）把前端编译成静态资源（`src-web/dist/`）；
2. 用 `cargo build --release` 编译 Rust 后端，**把上一步产出的前端资源直接嵌入进二进制**（不是外部文件，运行时不依赖 `dist/` 目录）；
3. 打包成 Windows 安装包。

产物在仓库根目录的 `build/release/`（通过 `src-tauri/.cargo/config.toml` 把 Cargo 默认的
`src-tauri/target/` 重定向到这里，不是 Tauri 默认位置）：
- `roc_desk.exe` — 单文件可执行程序，双击直接运行，不需要额外安装 Node/Rust 环境
- `bundle/msi/*.msi`、`bundle/nsis/*.exe` — 安装包（`tauri.conf.json` 里 `bundle.targets: "all"` 会两种都出）

首次 release 构建会编译全部 Rust 依赖，耗时较久（视机器性能几分钟到十几分钟）；之后增量构建会快很多。

## 常用命令速查

| 命令（均在仓库根目录执行） | 作用 |
|---|---|
| `npm run tauri:dev` | 开发模式，热重载 |
| `npm run tauri:build` | 生产构建，产出 exe/安装包 |
| `npm run build:web` | 只构建前端静态资源，不涉及 Rust |
| `npm run test:web` | 前端测试（Vitest，规划中） |
| `npm run test:rust` | `cargo test --manifest-path src-tauri/Cargo.toml` |
| `cargo check`（在 `src-tauri/` 下执行） | 只类型检查 Rust 代码，不产出二进制，最快的迭代反馈 |

## 推荐 IDE 配置

- [VS Code](https://code.visualstudio.com/) + [Tauri 插件](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) + [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)

## 许可证

[MIT](LICENSE) + [Commons Clause](https://commonsclause.com/)：可以自由使用、修改、分发（包括商业环境内部使用），唯一的限制是不能把本软件本身直接拿去卖钱或作为收费服务对外提供（即"不能直接商用"，其余不受限）。完整条款见 [LICENSE](LICENSE)。
