# 多仓库迁移执行清单

本文档把 `docs/MULTI_REPO_SPLIT_PLAN.md` 转换成持续执行的工程任务。每项任务完成后必须有代码、构建或发布验证，不以“文件已复制”视为完成。

## 当前阶段

- [x] 九个 GitHub 仓库已创建并映射到 `F:\code\wuyou\roc_tools`
- [x] `roc_desk-common` 已拆出 core/common/ui-core/common-web 基础包
- [x] 六个工具仓库已建立 Cargo workspace 和独立构建脚本
- [x] 六个 EXE 已发布到 `roc_desk-releases/v0.1.1`
- [x] 编辑器和资源管理器已接入公共 `LocalFileOps`
- [ ] 六个工具的真实 Tauri 窗口和 command 注册
- [ ] 六个工具业务后端完整接入各自 lib crate
- [ ] 六个工具前端 npm 包和独立入口
- [ ] 宿主切换为六个工具的固定 tag 依赖
- [ ] 删除宿主内重复业务源码
- [ ] 完整 EXE 人工测试和截图
- [ ] 发布 CI 和最终版本

## 执行顺序

### A. 公共接口

- [ ] 固定 core/common 版本，所有下游使用同一组 tag
- [ ] 完成 `FileOps`、连接档案、凭据引用、数据库错误和工作区接口
- [ ] 为公共接口添加最小单元测试和独立 crate 检查

### B. 工具后端

- [ ] HTTP：独立 command adapter、历史记录和变量服务
- [ ] Explorer：目录树、文件操作、远程文件适配
- [ ] Editor：文本/二进制预览、保存冲突、拖拽打开
- [ ] SQL：数据源、查询执行、对象树、结果分页
- [ ] SSH：SSH/SFTP/PTY/Agent/RDP 适配
- [ ] Workspace：PTY、符号、Git、Coding Agent 和编辑器复用

### C. 工具前端

- [ ] 六个工具分别有 `src-web/package.json`
- [ ] 导出工具根组件和公共类型
- [ ] 独立入口通过 Tauri IPC 调用对应 lib crate
- [ ] 宿主首页通过 npm 包组合工具

### D. 宿主和发布

- [ ] 宿主 Cargo 依赖全部固定 tag
- [ ] 宿主删除已迁移模块并通过 `cargo check --workspace`
- [ ] 独立 EXE 使用真实窗口，不再使用 MessageBox 壳
- [ ] 六个 EXE 启动、读写、连接、查询场景验证
- [ ] 截图写入六个仓库的 `docs/screenshots/`
- [ ] 发布库 CI 自动上传最终资产和 SHA-256

## 完成标准

只有满足以下条件才算整体完成：

1. 六个工具的业务代码只存在于各自仓库。
2. 独立 EXE 能显示真实工具窗口并执行对应核心操作。
3. 宿主只负责组合和启动，不再保留工具业务实现。
4. 所有仓库构建、宿主合并构建和发布流程均通过。
5. 每个工具 README、依赖、构建命令和截图均齐全。
