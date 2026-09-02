# 项目约定

## 每次完成任务后构建最新 exe

完成一次代码改动任务后（无论是 bugfix、新功能还是重构），默认执行一次本地便携版构建，
把最新的 `roc_desk.exe` 更新到仓库根目录的 `bin\` 目录，方便随时直接运行验证：

```powershell
.\build-portable.ps1
```

- 这个脚本会依次：关掉正在跑的 `roc_desk.exe` 进程（否则文件被占用导致拷贝失败）→
  `npm run tauri:build`（含 `beforeBuildCommand` 编译前端）→ 用 nightly 工具链编译
  `roc_desk_agent`（win7 兼容目标）→ 把 `roc_desk.exe`、`wfreerdp.exe`、
  `agent\roc_desk_agent.exe` 等一起拷进 `bin\`。详见 [scripts/build-portable.ps1](scripts/build-portable.ps1)。
- 只改了 `src-web/`（纯前端改动）也要走完整的 `build-portable.ps1`，因为它本来就包含
  `beforeBuildCommand` 触发的前端构建，不要为了图快单独只跑 `vite build` 就跳过这一步——
  `bin\roc_desk.exe` 必须是前后端都最新的产物。
- 例外：如果任务本身与 `roc_desk.exe`/前端无关（比如只改了 `roc_desk_agent`、纯文档、
  `README`、`DESIGN.md` 等不影响客户端产物的内容），或用户明确说了不需要构建，可以跳过。
- 这是本地便携版构建，**不是**正式发布流程——正式发布走 tag + GitHub Actions，见
  [RELEASING.md](RELEASING.md)，不要因为本地构建了 `bin\` 就去打 tag 或推送 release。
