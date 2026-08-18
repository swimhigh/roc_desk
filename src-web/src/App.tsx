import { useEffect, useRef, useState } from "react";
import { Code2, FolderCog, Globe, MessageSquare, Wrench, TerminalSquare, FileCode, ScrollText, Files, Search as SearchIcon } from "lucide-react";
import { useWorkspaceStore } from "./stores/workspaceStore";
import { useEditorStore } from "./stores/editorStore";
import { useTerminalStore } from "./stores/terminalStore";
import { registerHostKeyPromptListener } from "./stores/connectionStore";
import { registerAiChatListeners } from "./stores/aiChatStore";
import { registerCodingListeners } from "./stores/codingStore";
import { registerSearchListeners, useSearchStore } from "./stores/searchStore";
import { WorkspacePicker } from "./components/Workspace/WorkspacePicker";
import { ExplorerTree } from "./components/Explorer/ExplorerTree";
import { CodeEditor } from "./components/Editor/CodeEditor";
import { TerminalPanel } from "./components/Terminal/TerminalPanel";
import { SftpBrowser } from "./components/SftpBrowser/SftpBrowser";
import { SftpFileViewer } from "./components/SftpBrowser/SftpFileViewer";
import { LogSearchPanel } from "./components/LogSearch/LogSearchPanel";
import { ChatPanel } from "./components/AiChat/ChatPanel";
import { WebBrowserPanel } from "./components/WebBrowser/WebBrowserPanel";
import { SearchPanel } from "./components/Search/SearchPanel";
import { CodingAgentPanel } from "./components/CodingAgent/CodingAgentPanel";
import { HostKeyPromptHost } from "./components/ConnectionManager/HostKeyPromptHost";
import { ToastStack } from "./components/shared/Toast";
import { ThemeToggle } from "./components/shared/ThemeToggle";
import { sshService } from "./services/sshService";

type ActiveView = "editor" | "sftp" | "logs" | "aichat" | "coding" | "browser";

/**
 * 顶层入口：没有打开的工作区时展示 WorkspacePicker，
 * 否则展示 Explorer + 终端/编辑器/SFTP 的最小 IDE 布局（DESIGN.md §3.1）。
 *
 * 顶部快捷工具（SFTP/浏览器/AI问答/AI编程）始终可见——DESIGN.md §3.1.3 里这四个
 * 图标不是"远程专属"，只是各自的可用性不同（SFTP 需要一个连接，浏览器/AI 尚未实现）；
 * 之前把整行按 `kind === 'remote'` 隐藏是图省事的简化，会让本地工作区看起来
 * 功能残缺，改成"始终显示，未就绪的用 disabled+提示说明"，符合"状态诚实"原则。
 */
function App() {
  const current = useWorkspaceStore((s) => s.current);
  const backToPicker = useWorkspaceStore((s) => s.backToPicker);
  const openPreview = useEditorStore((s) => s.openPreview);
  const pinFile = useEditorStore((s) => s.pin);

  const [activeView, setActiveView] = useState<ActiveView>("editor");
  const [sidebarMode, setSidebarMode] = useState<"explorer" | "search">("explorer");
  const [sftpViewingPath, setSftpViewingPath] = useState<string | null>(null);
  const [sidebarWidth, setSidebarWidth] = useState(() => {
    const stored = Number(localStorage.getItem("roc_desk-sidebar-width"));
    return stored >= 160 && stored <= 600 ? stored : 220;
  });
  const sidebarDragRef = useRef<{ startX: number; startWidth: number } | null>(null);
  const connectingRef = useRef<string | null>(null);
  const terminalTabs = useTerminalStore((s) => s.tabs);
  const openTerminal = useTerminalStore((s) => s.openTerminal);
  const togglePanel = useTerminalStore((s) => s.togglePanel);
  const resetTerminals = useTerminalStore((s) => s.reset);

  useEffect(() => {
    const unlistenPromise = registerHostKeyPromptListener();
    return () => {
      unlistenPromise.then((unlisten) => unlisten());
    };
  }, []);

  useEffect(() => {
    const unlistenPromise = registerAiChatListeners();
    return () => {
      unlistenPromise.then((unlisten) => unlisten());
    };
  }, []);

  useEffect(() => {
    const unlistenPromise = registerCodingListeners();
    return () => {
      unlistenPromise.then((unlisten) => unlisten());
    };
  }, []);

  useEffect(() => {
    const unlistenPromise = registerSearchListeners();
    return () => {
      unlistenPromise.then((unlisten) => unlisten());
    };
  }, []);

  useEffect(() => {
    resetTerminals();
    // 切换工作区（含"返回工作区选择页再打开另一个"）时，上一个工作区打开的编辑器
    // 标签必须一起清掉——editorStore 是全局 Zustand store，不会因为 App 组件内部
    // 切换 WorkspacePicker/主界面这两棵 JSX 子树而自动重置（App 组件实例本身没
    // 卸载，只是子树切换），之前只重置了终端，编辑器标签被遗漏（真实 bug，
    // 2026-08-18 用户报告"打开新的工作区时，老工作区的编辑TAB未关闭"）。
    useEditorStore.getState().reset();
    if (!current) return;
    if (connectingRef.current === current.id) return;
    connectingRef.current = current.id;

    (async () => {
      try {
        // 打开工作区时默认起一个终端并展开底部面板，和 VS Code 打开工作区自动开终端的
        // 习惯一致；本地/远程工作区都要有，之前只给远程开是遗留的范围裁剪，不是有意设计。
        if (current.kind === "remote" && current.connection_id) {
          await sshService.connect(current.connection_id);
          await openTerminal({ kind: "ssh", profileId: current.connection_id, cwd: current.root_path });
        } else if (current.kind === "local") {
          await openTerminal({ kind: "local", cwd: current.root_path });
        }
      } catch (e) {
        console.error("打开终端失败", e);
      }
    })();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [current]);

  const onSidebarDragStart = (e: React.MouseEvent) => {
    sidebarDragRef.current = { startX: e.clientX, startWidth: sidebarWidth };
    let latest = sidebarWidth;
    const onMove = (ev: MouseEvent) => {
      if (!sidebarDragRef.current) return;
      const delta = ev.clientX - sidebarDragRef.current.startX;
      latest = Math.max(160, Math.min(600, sidebarDragRef.current.startWidth + delta));
      setSidebarWidth(latest);
    };
    const onUp = () => {
      sidebarDragRef.current = null;
      localStorage.setItem("roc_desk-sidebar-width", String(latest));
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  };

  if (!current) {
    return (
      <>
        <WorkspacePicker />
        <ToastStack />
        <HostKeyPromptHost />
      </>
    );
  }

  const isRemote = current.kind === "remote";
  const sftpReady = isRemote; // 本地工作区下需要先选一个已保存连接才能用 SFTP，尚未接（DESIGN.md §3.1.3）

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100vh" }}>
      <div className="tab-bar">
        <Code2 className="app-icon" />
        <button className="workspace-name-btn" onClick={backToPicker} title="切换工作区 (Ctrl+Shift+O)">
          {current.display_name} <span className="wp-chevron">▾</span>
        </button>

        <div className="tab-item" onClick={() => togglePanel()} title="切换底部终端面板 (Ctrl+`)">
          <span className={`tab-dot ${terminalTabs.length > 0 ? "connected" : "connecting"}`} />
          <TerminalSquare className="tab-icon" />
          <span>终端</span>
        </div>

        <div className={`tab-item ${activeView === "editor" ? "active" : ""}`} onClick={() => setActiveView("editor")}>
          <FileCode className="tab-icon" />
          <span>编辑器</span>
        </div>

        {sftpReady && (
          <div className={`tab-item ${activeView === "sftp" ? "active" : ""}`} onClick={() => setActiveView("sftp")}>
            <FolderCog className="tab-icon" />
            <span>SFTP</span>
          </div>
        )}

        <div className="quick-tools">
          <button
            className={`quick-tool-btn ${activeView === "sftp" ? "active" : ""}`}
            disabled={!sftpReady}
            title={sftpReady ? "SFTP 传输管理器（跨目录自由浏览）" : "本地工作区需先选择一个已保存连接才能用 SFTP，尚未实现"}
            onClick={() => sftpReady && setActiveView("sftp")}
          >
            <FolderCog />
          </button>
          <button
            className={`quick-tool-btn ${activeView === "logs" ? "active" : ""}`}
            title="日志搜索（索引搜索 / 远程实时搜索）"
            onClick={() => setActiveView("logs")}
          >
            <ScrollText />
          </button>
          <button
            className={`quick-tool-btn ${activeView === "browser" ? "active" : ""}`}
            title="网页浏览（独立窗口打开，这里管理历史记录）"
            onClick={() => setActiveView("browser")}
          >
            <Globe />
          </button>
          <button
            className={`quick-tool-btn ${activeView === "aichat" ? "active" : ""}`}
            title="AI 问答（与工作区本地/远程无关，始终可用）"
            onClick={() => setActiveView("aichat")}
          >
            <MessageSquare />
          </button>
          <button
            className={`quick-tool-btn ${activeView === "coding" ? "active" : ""}`}
            title="AI 编程助手（自动绑定当前工作区）"
            onClick={() => setActiveView("coding")}
          >
            <Wrench />
          </button>
          <ThemeToggle />
        </div>
      </div>

      <div style={{ flex: 1, display: "flex", overflow: "hidden" }}>
        <div
          style={{
            width: sidebarWidth,
            flexShrink: 0,
            display: "flex",
            flexDirection: "column",
            overflow: "hidden",
            background: "var(--bg-surface)",
          }}
        >
          {/* Explorer/搜索 模式切换（参考 VS Code 左侧活动栏，2026-08-18 需求：
              "左边的目录树，需要加类似的搜索功能"）——不新建一整套活动栏系统，
              侧边栏内容本身按需要在这两种视图间切换就够用。 */}
          <div style={{ display: "flex", flexShrink: 0, borderBottom: "1px solid var(--border-subtle)" }}>
            <button
              className={`quick-tool-btn ${sidebarMode === "explorer" ? "active" : ""}`}
              title="资源管理器"
              onClick={() => setSidebarMode("explorer")}
            >
              <Files />
            </button>
            <button
              className={`quick-tool-btn ${sidebarMode === "search" ? "active" : ""}`}
              title="搜索（跨文件全文搜索/替换）"
              onClick={() => setSidebarMode("search")}
            >
              <SearchIcon />
            </button>
          </div>
          <div style={{ flex: 1, overflowY: "auto", overflowX: "hidden" }}>
            {sidebarMode === "explorer" ? (
              <ExplorerTree
                workspaceId={current.id}
                rootPath={current.root_path}
                onOpenFile={(path, opts) => {
                  const opened = openPreview(current.id, path);
                  // 双击固定：等 openPreview 落地（buffer 出现在 store 里）之后再 pin，
                  // 否则 pin 会因为 buffer 还不存在而静默失效（ExplorerTree.tsx 同款注释）。
                  if (opts?.pin) opened.then(() => pinFile(path));
                  setActiveView("editor");
                }}
                onSearchInFolder={(path, relativePath) => {
                  useSearchStore.getState().setScope(path, relativePath || path);
                  setSidebarMode("search");
                }}
              />
            ) : (
              <SearchPanel
                workspaceId={current.id}
                onOpenResult={(path, line, highlights) => {
                  openPreview(current.id, path).then(() => {
                    useEditorStore.getState().revealLine(path, line, highlights);
                  });
                  setActiveView("editor");
                }}
              />
            )}
          </div>
        </div>
        {/* 侧边栏拖拽调宽（用户反馈：深层嵌套路径在固定 220px 宽度下会被截断看不全）。 */}
        <div className="sidebar-resize-handle" onMouseDown={onSidebarDragStart} />
        <div style={{ flex: 1, display: "flex", flexDirection: "column", overflow: "hidden" }}>
          <div style={{ flex: 1, overflow: "hidden", display: activeView === "editor" ? "block" : "none" }}>
            <CodeEditor workspaceId={current.id} workspaceName={current.display_name} rootPath={current.root_path} />
          </div>
          {sftpReady && (
            <div style={{ flex: 1, overflow: "hidden", display: activeView === "sftp" ? "block" : "none" }}>
              {current.connection_id && sftpViewingPath ? (
                <SftpFileViewer
                  profileId={current.connection_id}
                  path={sftpViewingPath}
                  onBack={() => setSftpViewingPath(null)}
                />
              ) : (
                current.connection_id && (
                  <SftpBrowser
                    profileId={current.connection_id}
                    workspaceId={current.id}
                    initialRemotePath={current.root_path}
                    onOpenFile={(entry) => setSftpViewingPath(entry.path)}
                  />
                )
              )}
            </div>
          )}
          <div style={{ flex: 1, overflow: "hidden", display: activeView === "logs" ? "block" : "none" }}>
            <LogSearchPanel
              workspaceKind={current.kind}
              profileId={current.connection_id}
              workspaceName={current.display_name}
              rootPath={current.root_path}
            />
          </div>
          <div style={{ flex: 1, overflow: "hidden", display: activeView === "aichat" ? "block" : "none" }}>
            <ChatPanel />
          </div>
          <div style={{ flex: 1, overflow: "hidden", display: activeView === "coding" ? "block" : "none" }}>
            <CodingAgentPanel workspaceId={current.id} />
          </div>
          <div style={{ flex: 1, overflow: "hidden", display: activeView === "browser" ? "block" : "none" }}>
            <WebBrowserPanel visible={activeView === "browser"} />
          </div>
          {/* 终端停靠在编辑器/SFTP/日志搜索下方，和 VS Code 的底部面板一致——不是和编辑器
              互斥切换的"视图"，是常驻的独立面板，可同时看到代码和终端输出
              （DESIGN.md §3.1.2 的多路复用体现在 UI 上）。 */}
          {isRemote && current.connection_id ? (
            <TerminalPanel target={{ kind: "ssh", profileId: current.connection_id, cwd: current.root_path }} />
          ) : !isRemote ? (
            <TerminalPanel target={{ kind: "local", cwd: current.root_path }} />
          ) : null}
        </div>
      </div>

      <div
        style={{
          height: 22,
          display: "flex",
          alignItems: "center",
          padding: "0 12px",
          background: "var(--bg-surface)",
          borderTop: "1px solid var(--border-default)",
          fontSize: 11,
          color: "var(--text-secondary)",
          flexShrink: 0,
        }}
      >
        {current.root_path}
      </div>

      <ToastStack />
      <HostKeyPromptHost />
    </div>
  );
}

export default App;
