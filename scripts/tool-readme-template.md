# {{TITLE}}

`{{NAME}}` 是 roc_desk 多仓库拆分后的独立工具仓库。

## 用途

{{PURPOSE}}

## 依赖

- 公共基础库：`roc_desk-common` 的 `roc_desk_core`，固定使用对应 tag。
- 工具专属依赖：{{DEPS}}。
- 独立壳：`standalone` crate，负责生成该工具自己的 EXE。

## 构建 EXE

```powershell
.\build-standalone.ps1
```

默认生成 Release 版本：`bin\{{NAME}}.exe`。开发构建使用：

```powershell
.\build-standalone.ps1 -Configuration debug
```

## 运行截图

截图放在 `docs/screenshots/`，例如：

```markdown
![{{TITLE}}主界面](docs/screenshots/main.png)
```

当前仓库已预留截图目录，后续补充实际运行截图。

## 迁移状态

业务源码已经从宿主仓库复制到本仓库。宿主暂时保留兼容实现，完成公共接口适配、独立 EXE 验证和宿主 Git 依赖切换后，再删除宿主副本。

总体方案见：[多仓库拆分计划](https://github.com/swimhigh/roc_desk/blob/main/docs/MULTI_REPO_SPLIT_PLAN.md)。
