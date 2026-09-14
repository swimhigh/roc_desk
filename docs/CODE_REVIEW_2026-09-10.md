# roc_desk 代码评审

评审日期：2026-09-10  
范围：当前工作区全部未提交变更，重点覆盖 `src-tauri`、`codex-engine`、`agent`、`protocol` 和 `src-web`。  
验证：`cargo check --workspace` 通过；`src-web` 的 `npx tsc --noEmit` 通过。构建过程中仅观察到 Rust 警告（未使用字段/参数及未来不兼容依赖），没有编译错误。

## 必须修复

### CR-01：文件写入冲突被当成成功，可能造成静默丢失修改

- 位置：`src-tauri/src/coding/changes.rs:101-110, 141-145, 162-178`
- `write_and_commit` 对 `WriteOutcome::Conflict` 只取 `current_mtime` 并继续返回成功；`stage` 在完全授权模式下把变更标成 `Applied`，`accept` 也会把待处理变更标成 `Applied`。
- `undo` 和 `redo` 采用同样的处理方式。
- 当用户或编辑器在 AI 生成 Diff 后修改了文件时，写入实际没有发生，但前端会收到成功同步信息、状态变为已应用，后续撤销还可能覆盖用户的新内容。
- 建议：冲突必须转换为 `AppError::Conflict` 并保留 `Pending/Applied` 原状态；返回当前磁盘内容和 mtime，让用户重新生成或显式确认覆盖。需要为 accept、full-auto、undo、redo 分别增加冲突测试。

### CR-02：Codex 的递归建目录路径拼接错误

- 位置：`src-tauri/src/coding/codex_exec_target.rs:116-128`
- `create_directory` 在 `recursive=true` 时从 `workspace_root` 开始，又逐个 `push(path.components())`。对绝对路径会把根组件再次拼入工作区；对相对路径也没有先解析到工作区根目录。结果可能创建错误目录、跨出工作区，或在 Windows 下生成无效路径。
- 建议：先用统一的工作区路径解析/校验函数把请求路径解析为目标路径，再从目标路径的父级开始按组件创建；绝对路径只允许位于工作区内。补充 Windows 盘符、UNC、`..` 和相对路径测试。

## 高风险问题

### CR-03：Codex 执行目标忽略了请求的 cwd 和 env

- 位置：`src-tauri/src/coding/codex_exec_target.rs:132-158`
- `run_command` 将 `_cwd`、`_env` 完全忽略，所有命令都由共享执行函数使用会话工作区和宿主环境运行。
- Codex 请求在子目录执行或依赖工具传入的环境变量时会得到错误结果；更严重的是，模型可通过预期的 cwd 语义访问错误目录。
- 建议：对 cwd 做工作区内解析和校验，并把允许的环境变量显式传递到本地/远程执行实现；若远程实现无法安全支持，应明确拒绝而不是静默忽略。

### CR-04：Codex 分支绕过了现有消息附件和部分会话策略

- 位置：`src-tauri/src/coding/session.rs:360-376`
- Codex 分支只把 `user_text` 写入引擎，未使用 `attachments`、会话模式及自研循环中的上下文处理；同时 `codex_exec_target` 固定传入 `auto_allow_readonly=false`。
- 用户在 UI 中附加的文件/图片不会进入 Codex 请求；只读自动放行设置也对 Codex 路径失效，造成行为不一致。
- 建议：统一构造用户消息和策略快照，再交给两种引擎；为附件、Plan/Build 模式和只读命令分别增加端到端测试。

## 中风险问题

### CR-05：变更状态机没有并发版本保护

- 位置：`src-tauri/src/coding/changes.rs:155-178, 194-214`
- accept/undo 读取状态后先 await 写文件，随后才修改状态。期间另一个请求可以对同一 change 发起操作；虽然外层 Mutex 限制了同一 store 的调用，但跨会话恢复、事件重入和未来拆分调用时容易产生重复写入窗口。
- 建议：为变更增加版本/操作中状态，或在写入前后再次校验状态和磁盘 mtime；所有状态迁移集中到带明确事务语义的方法中。

### CR-06：构建存在未清理警告和未使用配置

- `codex-engine/src/engine.rs:84` 的 `thread_manager` 未读取。
- `src-tauri/src/commands/local_fs.rs:69` 的 `is_dir` 未使用。
- Cargo 检查还提示 `proc-macro-error2` 将在未来 Rust 版本不兼容。
- 建议：清理无效字段/参数，锁定或升级相关依赖，并在 CI 中将新增 warning 视为需要处理的回归信号。

## 建议补充的验证

1. 为 `WriteOutcome::Conflict` 编写 accept/full-auto/undo/redo 的单元测试，断言磁盘内容、状态和事件三者一致。
2. 在 Windows 和 Linux 上测试 Codex 的绝对路径、相对路径、`..`、盘符和 UNC 建目录请求。
3. 增加 Codex cwd/env/附件透传的集成测试。
4. 对历史会话恢复后继续发送消息做回归测试，确认消息上下文、待处理变更和当前工作区完全一致。

## 结论

当前代码可以编译，前端类型检查通过，但不建议直接发布新增 Codex 文件操作能力。CR-01 会把实际写入失败伪装成成功，具有数据完整性风险；CR-02/CR-03 会使模型文件操作偏离用户选定的工作区。修复这三项并补齐冲突和路径测试后，再进行发布验收。
