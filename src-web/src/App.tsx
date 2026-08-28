import { useEffect, useRef, useState } from "react";
import { Code2, FolderCog, Globe, Home, Sparkles, TerminalSquare, FileCode, ScrollText, Files, Search as SearchIcon } from "lucide-react";
import { useWorkspaceStore } from "./stores/workspaceStore";
import { useEditorStore } from "./stores/editorStore";
import { useTerminalStore } from "./stores/terminalStore";
import { registerHostKeyPromptListener } from "./stores/connectionStore";
import { registerAiChatListeners } from "./stores/aiChatStore";
import { registerCodingListeners } from "./stores/codingStore";
import { registerSearchListeners, useSearchStore } from "./stores/searchStore";
import { sshService } from "./services/sshService";
import { ExplorerTree } from "./components/Explorer/ExplorerTree";
import { CodeEditor } from "./components/Editor/CodeEditor";
import { TerminalPanel } from "./components/Terminal/TerminalPanel";
import { SftpBrowser } from "./components/SftpBrowser/SftpBrowser";
import { SftpFileViewer } from "./components/SftpBrowser/SftpFileViewer";
import { LogSearchPanel } from "./components/LogSearch/LogSearchPanel";
import { WebBrowserPanel } from "./components/WebBrowser/WebBrowserPanel";
import { SearchPanel } from "./components/Search/SearchPanel";
import { CodingAgentPanel } from "./components/CodingAgent/CodingAgentPanel";
import { HostKeyPromptHost } from "./components/ConnectionManager/HostKeyPromptHost";
import { ToastStack, useToastStore } from "./components/shared/Toast";
import { ThemeToggle } from "./components/shared/ThemeToggle";
import { ContextMenu, type ContextMenuItem } from "./components/shared/ContextMenu";
import { HomeShell } from "./components/RemoteTool/HomeShell";
import { formatError } from "./utils/error";
import type { WorkspaceProfile } from "./types/bindings";

type ActiveView = "editor" | "sftp" | "logs" | "browser";

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
  const recentWorkspaces = useWorkspaceStore((s) => s.recent);
  const loadRecentWorkspaces = useWorkspaceStore((s) => s.loadRecent);
  const openLocalPath = useWorkspaceStore((s) => s.openLocalPath);
  const openRemoteWorkspace = useWorkspaceStore((s) => s.openRemoteWorkspace);
  const pushToast = useToastStore((s) => s.push);
  const openPreview = useEditorStore((s) => s.openPreview);
  const pinFile = useEditorStore((s) => s.pin);

  const [workspaceMenu, setWorkspaceMenu] = useState<{ x: number; y: number } | null>(null);
  const [activeView, setActiveView] = useState<ActiveView>("editor");
  const [sidebarMode, setSidebarMode] = useState<"explorer" | "search">("explorer");
  const [sftpViewingPath, setSftpViewingPath] = useState<string | null>(null);
  const [sidebarWidth, setSidebarWidth] = useState(() => {
    const stored = Number(localStorage.getItem("roc_desk-sidebar-width"));
    return stored >= 160 && stored <= 600 ? stored : 220;
  });
  const sidebarDragRef = useRef<{ startX: number; startWidth: number } | null>(null);
  const terminalTabs = useTerminalStore((s) => s.tabs);
  const openTerminal = useTerminalStore((s) => s.openTerminal);
  const togglePanel = useTerminalStore((s) => s.togglePanel);
  const [aiToolsOpen, setAiToolsOpen] = useState(false);
  const [aiToolsWidth, setAiToolsWidth] = useState(() => {
    const stored = Number(localStorage.getItem("roc_desk-ai-tools-width"));
    return stored >= 300 && stored <= 800 ? stored : 420;
  });
  const aiToolsDragRef = useRef<{ startX: number; startWidth: number } | null>(null);

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
    // 切换工作区（含"返回工作区选择页再打开另一个"）时，上一个工作区打开的编辑器
    // 标签必须一起清掉——editorStore 是全局 Zustand store，不会因为 App 组件内部
    // 切换 WorkspacePicker/主界面这两棵 JSX 子树而自动重置（App 组件实例本身没
    // 卸载，只是子树切换），之前只重置了终端，编辑器标签被遗漏（真实 bug，
    // 2026-08-18 用户报告"打开新的工作区时，老工作区的编辑TAB未关闭"）。编辑器
    // 标签的会话保留不在本次改造范围内（用户明确只要求终端和 AI 工具保活）。
    useEditorStore.getState().reset();
    if (!current) return;
    // 终端改为有界保活（terminalStore.switchWorkspace，2026-08-27 需求：切工作区
    // 时终端会话不应该被销毁）——最近用过的几个工作区各自的 Tab/Channel 原样
    // 保留，只有真正被 LRU 淘汰的工作区才会关闭其 PTY/SSH Channel；只有"这个
    // 工作区第一次打开、或者已经被淘汰过"时 `tabs` 才会是空的，此时才需要按
    // VS Code 的习惯自动起一个默认终端。
    let terminalCancelled = false;
    void (async () => {
      try {
        await useTerminalStore.getState().switchWorkspace(current.id);
        if (terminalCancelled) return;
        if (useTerminalStore.getState().tabs.length > 0) return;
        if (current.kind === "remote" && current.connection_id) {
          await sshService.connect(current.connection_id);
          if (!terminalCancelled) await openTerminal({ kind: "ssh", profileId: current.connection_id, cwd: current.root_path });
        } else if (current.kind === "local" && !terminalCancelled) {
          await openTerminal({ kind: "local", cwd: current.root_path });
        }
      } catch (error) {
        console.error("打开默认终端失败", error);
      }
    })();
    const key = `roc_desk-editor-tabs:${current.id}`;
    let paths: string[] = [];
    try {
      const parsed = JSON.parse(localStorage.getItem(key) || "[]");
      paths = Array.isArray(parsed) ? parsed.filter((p): p is string => typeof p === "string") : [];
    } catch {
      paths = [];
    }
    let cancelled = false;
    let restoring = true;
    const restore = async () => {
      for (const path of paths.slice(0, 30)) {
        if (cancelled) return;
        try { await useEditorStore.getState().openPreview(current.id, path); } catch { /* file unavailable */ }
      }
      if (!cancelled) {
        restoring = false;
        localStorage.setItem(key, JSON.stringify(useEditorStore.getState().order));
      }
    };
    void restore();
    const unsubscribe = useEditorStore.subscribe((state) => {
      if (!restoring && !cancelled) localStorage.setItem(key, JSON.stringify(state.order));
    });
    return () => { cancelled = true; terminalCancelled = true; unsubscribe(); };
    /*
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
    })();*/
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

  const onAiToolsDragStart = (e: React.MouseEvent) => {
    e.preventDefault();
    aiToolsDragRef.current = { startX: e.clientX, startWidth: aiToolsWidth };
    let latest = aiToolsWidth;
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";
    const onMove = (ev: MouseEvent) => {
      if (!aiToolsDragRef.current) return;
      const maxWidth = Math.max(300, Math.min(800, window.innerWidth - sidebarWidth - 320));
      latest = Math.max(300, Math.min(maxWidth, aiToolsDragRef.current.startWidth + aiToolsDragRef.current.startX - ev.clientX));
      setAiToolsWidth(latest);
    };
    const onUp = () => {
      aiToolsDragRef.current = null;
      localStorage.setItem("roc_desk-ai-tools-width", String(latest));
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  };

  // 没有打开工作区时，首页是"左侧会话树 + 右侧工作区列表"的固定布局（用户
  // 2026-08-25 明确要求："不需要去选会话模式和工作区模式后再展现"）——两边
  // 一直都在，不是需要提前选的两个模式。打开一个工作区后（`current` 非空）
  // 走下面原有的 IDE 布局，和这个首页完全独立。
  if (!current) {
    return (
      <>
        <HomeShell />
        <ToastStack />
        <HostKeyPromptHost />
      </>
    );
  }

  const isRemote = current.kind === "remote";
  const sftpReady = isRemote; // 本地工作区下需要先选一个已保存连接才能用 SFTP，尚未接（DESIGN.md §3.1.3）

  const handleTerminalToggle = async () => {
    if (terminalTabs.length > 0) {
      togglePanel();
      return;
    }
    try {
      if (isRemote && current.connection_id) {
        await sshService.connect(current.connection_id);
        await openTerminal({ kind: "ssh", profileId: current.connection_id, cwd: current.root_path });
      } else if (!isRemote) {
        await openTerminal({ kind: "local", cwd: current.root_path });
      }
    } catch (error) {
      console.error("打开终端失败", error);
    }
  };

  // 点工作区名字应该是"切到另一个工作区"（下拉最近工作区列表直接切换），不是
  // "跳回首页"——之前这两件事被绑在同一个按钮上，用户点了想切工作区结果整个
  // 界面跳走，反馈明确要求拆开：这个按钮改成下拉选择，回首页单独在最左边加一个
  // 按钮（用户 2026-08-25 原话："跳到首页可以在最左边再加个切换按钮"）。
  const switchWorkspace = async (w: WorkspaceProfile) => {
    setWorkspaceMenu(null);
    try {
      if (w.kind === "local") {
        await openLocalPath(w.root_path);
      } else if (w.connection_id) {
        await openRemoteWorkspace(w.connection_id, w.root_path);
      }
    } catch (error) {
      // 常见失败原因是这条远程连接没有已保存的密码——那种场景的补录密码入口在
      // 首页的 WorkspacePicker/RemoteWorkspaceDialog 里，这个下拉菜单只做"顺利
      // 情况下快速切换"，失败了指路回首页处理，不在这里重新实现一遍密码弹窗。
      pushToast("error", `切换工作区失败：${formatError(error)}（可返回首页重新打开以补录密码等信息）`);
    }
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100vh" }}>
      <div className="tab-bar">
        <button className="quick-tool-btn" onClick={backToPicker} title="返回首页">
          <Home />
        </button>
        <Code2 className="app-icon" />
        <button
          className="workspace-name-btn"
          onClick={async (e) => {
            const rect = e.currentTarget.getBoundingClientRect();
            await loadRecentWorkspaces();
            const others = useWorkspaceStore.getState().recent.filter((w) => w.id !== current.id);
            if (others.length === 0) {
              pushToast("info", "没有其它最近工作区，可返回首页打开新的");
              return;
            }
            setWorkspaceMenu({ x: rect.left, y: rect.bottom + 4 });
          }}
          title="切换工作区"
        >
          {current.display_name} <span className="wp-chevron">▾</span>
        </button>
        {workspaceMenu && (
          <ContextMenu
            x={workspaceMenu.x}
            y={workspaceMenu.y}
            onClose={() => setWorkspaceMenu(null)}
            items={recentWorkspaces
              .filter((w) => w.id !== current.id)
              .map(
                (w): ContextMenuItem => ({
                  label: `${w.kind === "local" ? "💻" : "🖥"} ${w.display_name} — ${w.root_path}`,
                  onClick: () => void switchWorkspace(w),
                }),
              )}
          />
        )}

        <div className="tab-item" onClick={() => void handleTerminalToggle()} title="切换底部终端面板 (Ctrl+`)">
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
            className={`quick-tool-btn ${aiToolsOpen ? "active" : ""}`}
            title="AI工具（问答与编程助手）"
            onClick={() => setAiToolsOpen((open) => !open)}
          >
            <Sparkles />
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
                  opened.catch((e) => pushToast("error", `打开失败：${formatError(e)}`));
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
                  openPreview(current.id, path)
                    .then(() => {
                      useEditorStore.getState().revealLine(path, line, highlights);
                    })
                    .catch((e) => pushToast("error", `打开失败：${formatError(e)}`));
                  setActiveView("editor");
                }}
              />
            )}
          </div>
        </div>
        {/* 侧边栏拖拽调宽（用户反馈：深层嵌套路径在固定 220px 宽度下会被截断看不全）。 */}
        <div className="sidebar-resize-handle" onMouseDown={onSidebarDragStart} />
        <div style={{ flex: 1, display: "flex", flexDirection: "row", overflow: "hidden" }}>
          <div style={{ flex: 1, minWidth: 0, display: "flex", flexDirection: "column", overflow: "hidden" }}>
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
          <div style={{ flex: 1, overflow: "hidden", display: activeView === "browser" ? "block" : "none" }}>
            <WebBrowserPanel visible={activeView === "browser"} />
          </div>
          {/* 终端停靠在编辑器/SFTP/日志搜索下方，和 VS Code 的底部面板一致——不是和编辑器
              互斥切换的"视图"，是常驻的独立面板，可同时看到代码和终端输出
              （DESIGN.md §3.1.2 的多路复用体现在 UI 上）。
              必须是同一个 `<TerminalPanel>` 元素、只是 `target` 这个 prop 随
              本地/远程变化——之前是按 `isRemote` 二选一渲染两个不同位置的
              `<TerminalPanel>`，本地/远程工作区之间切换时 React 会把整个组件
              连同它内部保活的所有工作区的终端一起卸载重建，是"终端会话保持"
              功能实际复现的另一半 bug（`terminalStore.ts` 的 `allTabs` 有界保活
              机制本身没问题，但组件树都被换掉了，state 再对也没用）。 */}
          {(isRemote && current.connection_id) || !isRemote ? (
            <TerminalPanel
              target={
                isRemote && current.connection_id
                  ? { kind: "ssh", profileId: current.connection_id, cwd: current.root_path }
                  : { kind: "local", cwd: current.root_path }
              }
            />
          ) : null}
          </div>
          {aiToolsOpen && (
            <>
              <div className="ai-tools-resize-handle" onMouseDown={onAiToolsDragStart} title="左右拖动调整 AI工具宽度" />
              <aside className="ai-tools-dock" style={{ width: aiToolsWidth }}>
                <div className="ai-tools-dock-header"><Sparkles /> <span>AI工具</span><button className="icon-btn" onClick={() => setAiToolsOpen(false)} title="关闭">×</button></div>
                <CodingAgentPanel
                  workspaceId={current.id}
                  active={aiToolsOpen}
                  onOpenFile={(path, line) => {
                    const isAbsolute = /^[A-Za-z]:[\\/]/.test(path) || path.startsWith("/");
                    const separator = current.root_path.includes("\\") ? "\\" : "/";
                    const resolvedPath = isAbsolute ? path : `${current.root_path.replace(/[\\/]$/, "")}${separator}${path.replace(/^[./\\]+/, "")}`;
                    openPreview(current.id, resolvedPath)
                      .then(() => { if (line) useEditorStore.getState().revealLine(resolvedPath, line); })
                      .catch((e) => pushToast("error", `打开失败：${formatError(e)}`));
                  }}
                />
              </aside>
            </>
          )}
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
