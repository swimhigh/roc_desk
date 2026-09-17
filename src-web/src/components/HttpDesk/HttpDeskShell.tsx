import React, { useEffect } from "react";
import { Home, Send } from "lucide-react";
import { useModeStore } from "../../stores/modeStore";
import { useWorkspaceStore } from "../../stores/workspaceStore";
import { useHttpDeskStore } from "../../stores/httpDeskStore";
import { WorkspacePicker } from "../Workspace/WorkspacePicker";
import { ThemeToggle } from "../shared/ThemeToggle";
import { HttpWorkspace } from "./HttpWorkspace";

/** HTTP 桌面模块窗口顶层壳（docs/HTTP_DESKTOP_PLAN.md §3）——复用"工作区"模块
 * 完全相同的目录/连接打开机制（`useWorkspaceStore`/`WorkspacePicker`），没打开
 * 工作区时展示的是同一个组件，不是另造一套集合选择页。 */
export const HttpDeskShell: React.FC = () => {
  const goHome = useModeStore((s) => s.goHome);
  const modeOpen = useModeStore((s) => s.open);
  const current = useWorkspaceStore((s) => s.current);
  const openById = useWorkspaceStore((s) => s.openById);
  const init = useHttpDeskStore((s) => s.init);

  // 首页点了"最近工作区"里的具体项（带 `--open=<workspace_id>`）——直接打开它，
  // 和 App.tsx 里 `workspace` 模块窗口冷启动的同款模式。
  useEffect(() => {
    if (modeOpen && !current) {
      void openById(modeOpen);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [modeOpen]);

  useEffect(() => {
    if (current) void init(current.id);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [current?.id]);

  if (!current) {
    // 和 App.tsx 里 `mode === "workspace"` 且 `!current` 时是同一个组件，只是
    // `module="http"`——展示的是"HTTP 测试工作台"自己添加过的工作区子集，不是
    // 所有工作区（2026-09 需求，见 WorkspacePicker 文档）。`WorkspacePicker`
    // 自带 Home 按钮和主题切换，这里不需要再包一层自己的 tab-bar。
    return (
      <div style={{ height: "100vh" }}>
        <WorkspacePicker module="http" />
      </div>
    );
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100vh" }}>
      <div className="tab-bar">
        <button className="quick-tool-btn" onClick={() => void goHome()} title="返回首页">
          <Home />
        </button>
        <Send className="app-icon" />
        <span className="workspace-name-btn" style={{ cursor: "default" }}>
          {current.display_name} · HTTP 测试工作台
        </span>
        <div className="quick-tools" style={{ marginLeft: "auto" }}>
          <ThemeToggle />
        </div>
      </div>
      <div style={{ flex: 1, minHeight: 0, display: "flex" }}>
        <HttpWorkspace workspaceId={current.id} />
      </div>
    </div>
  );
};
