# 发布流程

roc_desk 通过 GitHub Actions（[.github/workflows/release.yml](.github/workflows/release.yml)）自动构建并发布 Release，本地不需要手动打包上传。

## 版本号只有一个权威来源：`src-tauri/Cargo.toml`

`tauri.conf.json` 故意不写 `version` 字段——Tauri 会在缺省时自动读取 `Cargo.toml` 的 `package.version`，用它来命名 `roc_desk.exe` 的文件版本信息、MSI/NSIS 安装包的文件名（如 `roc_desk_0.1.0_x64-setup.exe`）。这样版本号只用改一个地方，不会出现"这个文件里改了、那个文件里忘了改"导致 tag 和实际产物版本对不上的问题。

根目录 `package.json`/`src-web/package.json` 的 `version` 字段是 npm 自己的东西，和发布出去的 exe/安装包版本无关，不需要跟着同步（虽然保持一致更整洁，但不是发布流程的强制要求）。

## 发布一个新版本

1. 改 `src-tauri/Cargo.toml` 里的 `version = "x.y.z"`。
2. 提交这次改动（连同这个版本要发布的所有功能改动一起）：
   ```bash
   git add -A
   git commit -m "release: vX.Y.Z"
   git push
   ```
3. 打 tag 并推送——tag 名必须是 `v` 前缀 + 和 Cargo.toml 完全一致的版本号（如 `v0.1.0`），这是 workflow 里自动校验的硬性要求，不一致会直接构建失败退出，不会误发布一个文件名对不上的 Release：
   ```bash
   git tag v0.1.0
   git push origin v0.1.0
   ```
4. 打开仓库的 **Actions** 页面看 `Release` workflow 跑的进度（GitHub 的 Windows 运行器上构建，大概几分钟）。跑完后 **Releases** 页面会自动出现一个新条目，标题是 `roc_desk vX.Y.Z`，附件是：
   - `roc_desk_X.Y.Z_x64-setup.exe` —— NSIS 安装包，**大多数用户应该下这个**：双击装完自动建快捷方式，之后也能通过控制面板卸载。
   - `roc_desk_X.Y.Z_x64_en-US.msi` —— MSI 安装包，给需要用企业软件分发工具（如 Intune、组策略）批量部署的场景用。
   - `roc_desk-vX.Y.Z-portable.exe` —— 免安装单文件版本，下载下来直接双击运行，不写注册表、不建开始菜单项，适合"临时用一下""U盘带着走"的场景；数据库/日志会写在它所在目录的 `data/` 子目录里（见 DESIGN.md §八-2 数据目录位置的设计）。

## 如果构建失败要重跑

不需要重新打一个 tag（版本号没变就不该占用一个新 tag）。去 Actions 页面找到那次失败的 run，点 **Re-run failed jobs**；或者用 `workflow_dispatch` 手动触发一次（Actions 页面 → Release workflow → Run workflow，可以选择对着已存在的 tag 跑）。

## 本地验证构建是否能过（不发布，只是自己确认）

```bash
npm run tauri:build
```
产物在仓库根目录 `build/release/`（`roc_desk.exe` 本体）和 `build/release/bundle/{msi,nsis}/`（安装包），和 CI 走的是完全同一条命令，本地能过 CI 大概率也能过。
