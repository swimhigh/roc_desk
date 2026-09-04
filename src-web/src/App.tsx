import { useCallback, useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import {
  ArrowLeft,
  Code2,
  FolderCog,
  Globe,
  Home,
  Sparkles,
  TerminalSquare,
  FileCode,
  ScrollText,
  Files,
  Search as SearchIcon,
  RotateCw,
  FilePlus2,
  FolderOpen,
} from "lucide-react";
import { useModeStore } from "./stores/modeStore";
import { useWorkspaceStore } from "./stores/workspaceStore";
import { isDiffId, useEditorStore } from "./stores/editorStore";
import { openExternalPaths } from "./utils/openExternalPaths";
import { useExplorerStore } from "./stores/explorerStore";
import { useTerminalStore } from "./stores/terminalStore";
import { registerAgentCertPromptListener, registerHostKeyPromptListener } from "./stores/connectionStore";
import { registerAiChatListeners } from "./stores/aiChatStore";
import { registerCodingListeners } from "./stores/codingStore";
import { registerSearchListeners, useSearchStore } from "./stores/searchStore";
import { sshService } from "./services/sshService";
import { connectionService } from "./services/connectionService";
import { localFsService } from "./services/localFsService";
import { ExplorerTree } from "./components/Explorer/ExplorerTree";
import { CodeEditor } from "./components/Editor/CodeEditor";
import { LocalFileTree } from "./components/Editor/LocalFileTree";
import { TerminalPanel } from "./components/Terminal/TerminalPanel";
import { SftpBrowser } from "./components/SftpBrowser/SftpBrowser";
import { SftpFileViewer } from "./components/SftpBrowser/SftpFileViewer";
import { AgentBrowser } from "./components/SftpBrowser/AgentBrowser";
import { LogSearchPanel } from "./components/LogSearch/LogSearchPanel";
import { WebBrowserPanel } from "./components/WebBrowser/WebBrowserPanel";
import { SearchPanel } from "./components/Search/SearchPanel";
import { CodingAgentPanel } from "./components/CodingAgent/CodingAgentPanel";
import { HostKeyPromptHost } from "./components/ConnectionManager/HostKeyPromptHost";
import { AgentCertPromptHost } from "./components/ConnectionManager/AgentCertPromptHost";
import { ToastStack, useToastStore } from "./components/shared/Toast";
import { ThemeToggle } from "./components/shared/ThemeToggle";
import { ContextMenu, type ContextMenuItem } from "./components/shared/ContextMenu";
import { HomeShell } from "./components/RemoteTool/HomeShell";
import { HomeDashboard } from "./components/Home/HomeDashboard";
import { LocalExplorerScreen } from "./components/LocalExplorer/LocalExplorerScreen";
import { WorkspacePicker } from "./components/Workspace/WorkspacePicker";
import { formatError } from "./utils/error";
import type { WorkspaceProfile } from "./types/bindings";

type ActiveView = "editor" | "sftp" | "logs" | "browser";

/** "remote" 工作区可能是 SSH 也可能是 Agent 连接（AGENT_DESIGN.md），
 * `WorkspaceProfile` 本身不带协议字段，需要另查一次连接档案列表才知道该开哪种
 * 终端——和 RemoteWorkspaceDialog.tsx 里同样的"查全量列表再 find"写法一致。 */
async function resolveWorkspaceProtocol(connectionId: string): Promise<"ssh" | "agent" | "rdp"> {
  const profiles = await connectionService.list();
  return profiles.find((p) => p.id === connectionId)?.protocol ?? "ssh";
}

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
  const updateLastSftpPaths = useWorkspaceStore((s) => s.updateLastSftpPaths);
  const mode = useModeStore((s) => s.mode);
  const modeOpen = useModeStore((s) => s.open);
  const goHome = useModeStore((s) => s.goHome);
  const recentWorkspaces = useWorkspaceStore((s) => s.recent);
  const loadRecentWorkspaces = useWorkspaceStore((s) => s.loadRecent);
  const openLocalPath = useWorkspaceStore((s) => s.openLocalPath);
  const openRemoteWorkspace = useWorkspaceStore((s) => s.openRemoteWorkspace);
  const pushToast = useToastStore((s) => s.push);
  const openPreview = useEditorStore((s) => s.openPreview);
  const pinFile = useEditorStore((s) => s.pin);
  // 没打开任何工作区、只有游离文件标签时的极简编辑器壳是否可见——和下面 `showPicker`
  // 是同一种"两棵子树都常驻挂载，只切 display"的模式（原因见下面完整 IDE 布局那段
  // 注释），但这个状态只在"本次会话从没打开过工作区"（`!current`）时才有意义。放在
  // editorStore 里（不是本地 state）是因为 WorkspacePicker.tsx 首页的"打开文件"
  // 按钮也要能触发它，那个组件和 App.tsx 没有共同的父组件方便传参。
  const standaloneShellVisible = useEditorStore((s) => s.standaloneShellVisible);
  const hideStandaloneShell = useEditorStore((s) => s.hideStandaloneShell);

  const [workspaceMenu, setWorkspaceMenu] = useState<{ x: number; y: number } | null>(null);
  // 编辑器模块左侧文件树当前浏览的根目录（用户 2026-09-04 需求："编辑器桌面需要
  // 左边有个资源管理器"）——只在 `mode === "editor"` 的进程里用得到，但和其它
  // 顶层状态一样声明在这里，不放进条件分支（Hooks 规则）。持久化到 localStorage
  // 是为了下次开编辑器模块窗口还停在上次浏览的目录，不用重新选一次。
  const [editorRoot, setEditorRoot] = useState<string | null>(() => localStorage.getItem("roc_desk-editor-tree-root"));
  // "remote" 工作区可能是 SSH 也可能是 Agent 连接（AGENT_DESIGN.md），底部终端面板
  // 的 `target` prop 渲染时需要知道具体是哪种协议，才能决定新开的终端走哪条后端
  // 命令——这个状态和下面的"自动打开默认终端"用的是同一次协议解析结果。
  const [workspaceProtocol, setWorkspaceProtocol] = useState<"ssh" | "agent" | "rdp">("ssh");
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
  /** 有界保活的 LRU 常驻工作区集合（terminalStore.ts）——"切换工作区"下拉菜单用它
   * 标一个绿点：在这个集合里说明终端 Channel/xterm 实例一直没被摘掉，切过去是
   * 真正的"原样恢复"，不在集合里则说明上次切走时已经被 LRU 淘汰，切回去会是一个
   * 全新终端（2026-09-01 用户需求：想要和 SSH 会话树一样的在线状态指示）。 */
  const residentWorkspaceIds = useTerminalStore((s) => s.residentOrder);
  const [aiToolsOpen, setAiToolsOpen] = useState(false);
  const [aiToolsWidth, setAiToolsWidth] = useState(() => {
    const stored = Number(localStorage.getItem("roc_desk-ai-tools-width"));
    return stored >= 300 && stored <= 800 ? stored : 420;
  });
  const aiToolsDragRef = useRef<{ startX: number; startWidth: number } | null>(null);

  // 这个进程冷启动时到底是首页还是某个模块窗口（`docs/HOME_MODES_DESIGN.md` §3.5），
  // 决定下面整个组件该渲染成什么样——必须在其它任何渲染分支之前先拿到，`mode`
  // 从 `undefined` 变成确定值之前先不渲染任何东西，避免闪一下首页/工作区再跳走。
  useEffect(() => {
    void useModeStore.getState().loadLaunchContext();
  }, []);

  // 工作区模块窗口带了 `--open=<workspace_id>`（首页点了"最近工作区"里的具体一项）——
  // 直接按 id 打开，不需要用户在 WorkspacePicker 里再选一次。只在这个模块窗口
  // 还没打开任何工作区时尝试一次，成功后 `current` 变为非空，条件不再满足。
  useEffect(() => {
    if (mode === "workspace" && modeOpen && !current) {
      useWorkspaceStore.getState().openById(modeOpen).catch((e) => {
        pushToast("error", `打开工作区失败：${formatError(e)}`);
      });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [mode, modeOpen]);

  // 编辑器模块窗口没有"工作区"这个概念，`terminalStore` 的有界保活池却是按
  // 工作区 id 分组的（见 terminalStore.ts）——用一个固定的伪 id 把编辑器模块窗口
  // 的终端也接进同一套保活机制，不用为它另写一套。伪 id 不是合法的工作区 UUID，
  // 不会和真实工作区撞车。
  useEffect(() => {
    if (mode === "editor") void useTerminalStore.getState().switchWorkspace("__standalone_editor__");
  }, [mode]);

  // 编辑器模块"记住最后一次打开的文件列表"（用户 2026-09-04 需求），和 App.tsx
  // 下面工作区模式那份"按 workspaceId 记 tab"的逻辑是同一个模式，只是编辑器模块
  // 没有 workspaceId，用固定 key，且只记 `origin === "standalone"` 的标签（工作区
  // 标签不会出现在这个模式的进程里，多这一层过滤是为了以防万一，不依赖"这个
  // 进程里不会有 workspace 标签"这个假设）。
  //
  // `--open=<path>`（资源管理器模块双击本地"可编辑"文件时 spawn 出来的编辑器窗口
  // 就是这么打开目标文件的）必须在"恢复历史标签"的循环**之后**再打开——`openStandaloneFile`
  // 每次都会把刚打开的文件设成当前激活标签，这两件事以前是分成两个各自独立的
  // `useEffect`，触发时机都是 `[mode, modeOpen]`，会并发跑：历史标签恢复循环里有
  // 多次 await，`--open` 那次只有一次，谁先跑完全看时序——真实反馈（2026-09-04）：
  // "新打开的可编辑文档跳转到 roc_desk 编辑器界面时，默认展示的文件不是本文件"，
  // 就是这个竞态，历史记录里排在后面的某个文件在 `--open` 目标文件之后才恢复完，
  // 把它顶掉了。合并成一个效果、显式排好"先恢复历史、再打开目标文件"的顺序即可。
  useEffect(() => {
    if (mode !== "editor") return;
    const key = "roc_desk-editor-standalone-tabs";
    let paths: string[] = [];
    try {
      const parsed = JSON.parse(localStorage.getItem(key) || "[]");
      paths = Array.isArray(parsed) ? parsed.filter((p): p is string => typeof p === "string") : [];
    } catch {
      paths = [];
    }
    let cancelled = false;
    let restoring = true;
    const standaloneTabIds = (state: ReturnType<typeof useEditorStore.getState>) =>
      state.order.filter((id) => !isDiffId(id) && state.buffers[id]?.origin === "standalone");
    const restore = async () => {
      for (const path of paths.slice(0, 30)) {
        if (cancelled) return;
        try {
          await useEditorStore.getState().openStandaloneFile(path);
        } catch {
          /* file unavailable */
        }
      }
      if (cancelled) return;
      if (modeOpen) {
        try {
          await useEditorStore.getState().openStandaloneFile(modeOpen);
        } catch (e) {
          pushToast("error", `打开文件失败：${formatError(e)}`);
        }
      }
      if (!cancelled) {
        restoring = false;
        localStorage.setItem(key, JSON.stringify(standaloneTabIds(useEditorStore.getState())));
      }
    };
    void restore();
    const unsubscribe = useEditorStore.subscribe((state) => {
      if (!restoring && !cancelled) localStorage.setItem(key, JSON.stringify(standaloneTabIds(state)));
    });
    return () => {
      cancelled = true;
      unsubscribe();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [mode, modeOpen]);

  useEffect(() => {
    const unlistenPromise = registerHostKeyPromptListener();
    return () => {
      unlistenPromise.then((unlisten) => unlisten());
    };
  }, []);

  useEffect(() => {
    const unlistenPromise = registerAgentCertPromptListener();
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
        // 协议解析要在"已经有保活的 Tab、不需要自动开新终端"这条早退路径之前做——
        // `workspaceProtocol` 状态驱动的是底部面板"新建终端"按钮该走哪条后端命令，
        // 不是只有"自动开默认终端"这一条路径用得到，即使这次切换不需要自动开终端
        // 也必须更新它，否则从 Agent 工作区切到 SSH 工作区（或反过来）之后，
        // 手动点"新建终端"仍然会用上一个工作区的协议。
        const protocol = current.kind === "remote" && current.connection_id ? await resolveWorkspaceProtocol(current.connection_id) : "ssh";
        if (!terminalCancelled) setWorkspaceProtocol(protocol);

        await useTerminalStore.getState().switchWorkspace(current.id);
        if (terminalCancelled) return;
        if (useTerminalStore.getState().tabs.length > 0) return;
        if (current.kind === "remote" && current.connection_id) {
          if (protocol === "agent") {
            if (!terminalCancelled) await openTerminal({ kind: "agent", profileId: current.connection_id, cwd: current.root_path });
          } else {
            await sshService.connect(current.connection_id);
            if (!terminalCancelled) await openTerminal({ kind: "ssh", profileId: current.connection_id, cwd: current.root_path });
          }
        } else if (current.kind === "local" && !terminalCancelled) {
          await openTerminal({ kind: "local", cwd: current.root_path });
        }
      } catch (error) {
        console.error("打开默认终端失败", error);
        pushToast("error", `打开终端失败：${formatError(error)}`);
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
    // 只持久化"属于这个工作区"的标签——游离标签（origin: "standalone"）不属于
    // 任何工作区，混进 `order` 里但不该被当成这个工作区的 tab 记下来，否则下次
    // 重开这个工作区会把游离文件路径当工作区内路径传给 `openPreview`，触发
    // `guard_local_path` 校验失败（该路径本就不在工作区根目录下）。
    const workspaceTabIds = (state: ReturnType<typeof useEditorStore.getState>) =>
      state.order.filter((id) => isDiffId(id) || state.buffers[id]?.origin === "workspace");
    const restore = async () => {
      for (const path of paths.slice(0, 30)) {
        if (cancelled) return;
        try { await useEditorStore.getState().openPreview(current.id, path); } catch { /* file unavailable */ }
      }
      if (!cancelled) {
        restoring = false;
        localStorage.setItem(key, JSON.stringify(workspaceTabIds(useEditorStore.getState())));
      }
    };
    void restore();
    const unsubscribe = useEditorStore.subscribe((state) => {
      if (!restoring && !cancelled) localStorage.setItem(key, JSON.stringify(workspaceTabIds(state)));
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

  // 拖拽/Ctrl+O/系统文件关联("打开方式")打开外部路径——具体逻辑在 utils/
  // openExternalPaths.ts（WorkspacePicker.tsx 首页"打开文件"按钮也用同一个函数），
  // 这里只是包一层"打开后把编辑器 tab 切到前台"。
  const handleOpenExternalPaths = useCallback(async (paths: string[]) => {
    // 拖拽/Ctrl+O/文件关联打开外部路径只在"工作区"/"编辑器"这两个模块窗口里有
    // 意义（会打开工作区或游离文件标签）——`ssh`/`explorer`/首页进程不渲染
    // Explorer/编辑器，落地在这两种模块的窗口里默默调用 `openLocalPath` 之类
    // 的 store action 只会产生一堆看不见效果的状态，不如直接跳过。
    const currentMode = useModeStore.getState().mode;
    if (currentMode !== "workspace" && currentMode !== "editor") return;
    await openExternalPaths(paths);
    setActiveView("editor");
  }, []);

  const openFileDialog = useCallback(async () => {
    const selected = await open({ directory: false, multiple: true });
    if (!selected) return;
    await handleOpenExternalPaths(Array.isArray(selected) ? selected : [selected]);
  }, [handleOpenExternalPaths]);

  // 编辑器模块的终端开关（用户 2026-09-04 需求："下边可以开终端"）——已有终端就
  // 只是切面板显示/隐藏，没有就新开一个，cwd 优先用左侧文件树当前浏览的目录，
  // 没选过目录就退回用户主目录（`pty_open` 的 cwd 传空字符串行为未定义，不能
  // 图省事直接传 undefined）。
  const handleEditorTerminalToggle = async () => {
    if (terminalTabs.length > 0) {
      togglePanel();
      return;
    }
    try {
      const cwd = editorRoot ?? (await localFsService.homeDir().catch(() => undefined));
      await openTerminal({ kind: "local", cwd });
    } catch (error) {
      pushToast("error", `打开终端失败：${formatError(error)}`);
    }
  };

  // Ctrl+O 全局快捷键：在首页、独立编辑器壳、完整 IDE 里都要能用（不依赖 Monaco
  // 实例是否挂载），所以挂在 window 上而不是某个具体组件里。
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && !e.shiftKey && !e.altKey && e.key.toLowerCase() === "o") {
        e.preventDefault();
        void openFileDialog();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [openFileDialog]);

  // 全局外部文件拖入：和 `useDualPaneDnd`（SFTP/Agent 双栏内部的拖拽，hooks/
  // useDualPaneDnd.ts）各管一摊——两边各自独立调用 Tauri 的 `onDragDropEvent`，
  // 互不冲突（多个监听者都会收到同一个事件），只是要避免同一次拖放被两套逻辑
  // 重复处理：双栏容器打了 `data-external-drop-zone` 标记，这里落点命中该标记
  // 就整个让开，交给双栏自己的 hook 处理。
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    (async () => {
      const fn = await getCurrentWebviewWindow().onDragDropEvent((event) => {
        if (event.payload.type !== "drop") return;
        const ratio = window.devicePixelRatio || 1;
        const x = event.payload.position.x / ratio;
        const y = event.payload.position.y / ratio;
        const hitZone = document.elementFromPoint(x, y)?.closest("[data-external-drop-zone]");
        if (hitZone) return;
        if (event.payload.paths.length > 0) void handleOpenExternalPaths(event.payload.paths);
      });
      if (cancelled) fn();
      else unlisten = fn;
    })();
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [handleOpenExternalPaths]);

  // Windows"打开方式"/双击已关联文件（`tauri-plugin-single-instance`，2026-09-03
  // 需求）：冷启动带的路径存在后端 `AppState.pending_open_paths` 里，这里挂载时
  // 取走一次；已运行实例收到的第二次启动转发走 `open-file-paths` 事件，两条路径
  // 最终都汇到同一个处理函数。
  useEffect(() => {
    void invoke<string[]>("take_pending_open_paths").then((paths) => {
      if (paths.length > 0) void handleOpenExternalPaths(paths);
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    (async () => {
      const fn = await listen<string[]>("open-file-paths", (event) => {
        void handleOpenExternalPaths(event.payload);
      });
      if (cancelled) fn();
      else unlisten = fn;
    })();
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [handleOpenExternalPaths]);

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

  // 这个进程冷启动时到底该渲染成哪个工作模块，由命令行 `--mode` 决定
  // （`docs/HOME_MODES_DESIGN.md` §3.5）——每个模块窗口只挂载自己这一种模块的
  // 子树，不再是"一个进程里塞下所有模块、靠内部状态切换"。`mode` 还没从后端
  // 取到之前不渲染任何东西，避免先闪一下首页再跳到真正的模块内容。
  if (mode === undefined) return null;

  if (mode === "ssh") {
    return (
      <>
        <div style={{ height: "100vh" }}>
          <HomeShell />
        </div>
        <ToastStack />
        <HostKeyPromptHost />
        <AgentCertPromptHost />
      </>
    );
  }

  if (mode === "explorer") {
    return (
      <>
        <LocalExplorerScreen />
        <ToastStack />
      </>
    );
  }

  if (mode === null) {
    return (
      <>
        <HomeDashboard />
        <ToastStack />
      </>
    );
  }

  if (mode === "editor") {
    return (
      <>
        <div style={{ display: "flex", flexDirection: "column", height: "100vh" }}>
          <div className="tab-bar">
            <button className="quick-tool-btn" onClick={() => void goHome()} title="返回首页">
              <Home />
            </button>
            <Code2 className="app-icon" />
            <span className="workspace-name-btn" style={{ cursor: "default" }}>
              本地文件
            </span>
            <div className="tab-item" onClick={() => void handleEditorTerminalToggle()} title="切换底部终端面板 (Ctrl+`)">
              <span className={`tab-dot ${terminalTabs.length > 0 ? "connected" : "connecting"}`} />
              <TerminalSquare className="tab-icon" />
              <span>终端</span>
            </div>
            <div className="quick-tools" style={{ marginLeft: "auto" }}>
              <button className="quick-tool-btn" title="打开文件 (Ctrl+O)" onClick={() => void openFileDialog()}>
                <FilePlus2 />
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
              <LocalFileTree
                root={editorRoot}
                onRootChange={(path) => {
                  setEditorRoot(path);
                  localStorage.setItem("roc_desk-editor-tree-root", path);
                }}
                onOpenFile={(path) =>
                  useEditorStore
                    .getState()
                    .openStandaloneFile(path)
                    .catch((e) => pushToast("error", `打开失败：${formatError(e)}`))
                }
              />
            </div>
            <div className="sidebar-resize-handle" onMouseDown={onSidebarDragStart} />
            <div style={{ flex: 1, minWidth: 0, display: "flex", flexDirection: "column", overflow: "hidden" }}>
              <div style={{ flex: 1, minHeight: 0 }}>
                <CodeEditor workspaceId={null} workspaceName="" rootPath="" />
              </div>
              <TerminalPanel target={{ kind: "local", cwd: editorRoot ?? "" }} />
            </div>
          </div>
        </div>
        <ToastStack />
        <HostKeyPromptHost />
        <AgentCertPromptHost />
      </>
    );
  }

  // 走到这里 `mode === "workspace"`：还没打开任何工作区时展示 WorkspacePicker
  // （本模块窗口自己的"选一个工作区"页面，不再是合并了会话树的 HomeShell）；
  // 通过拖拽/Ctrl+O/文件关联打开了游离文件时（2026-09-03 需求），走第三种极简
  // 布局：只有 tab 栏 + 编辑区，没有 Explorer/终端/SFTP/AI 面板——这些都要求
  // 工作区上下文，硬造一个没意义。两棵子树都常驻挂载，只切 display，避免来回
  // 切换时丢失已打开文件的编辑状态。
  if (!current) {
    return (
      <>
        <div style={{ display: standaloneShellVisible ? "none" : "block", height: "100vh" }}>
          <WorkspacePicker />
        </div>
        <div style={{ display: standaloneShellVisible ? "flex" : "none", flexDirection: "column", height: "100vh" }}>
          <div className="tab-bar">
            <button className="quick-tool-btn" onClick={hideStandaloneShell} title="返回工作区选择">
              <ArrowLeft />
            </button>
            <Code2 className="app-icon" />
            <span className="workspace-name-btn" style={{ cursor: "default" }}>
              本地文件
            </span>
            <div className="quick-tools" style={{ marginLeft: "auto" }}>
              <button className="quick-tool-btn" title="打开文件 (Ctrl+O)" onClick={() => void openFileDialog()}>
                <FilePlus2 />
              </button>
              <button
                className="quick-tool-btn"
                title="打开文件夹作为工作区"
                onClick={() =>
                  void useWorkspaceStore
                    .getState()
                    .openLocalFolder()
                    .catch((e) => pushToast("error", `打开文件夹失败：${formatError(e)}`))
                }
              >
                <FolderOpen />
              </button>
              <ThemeToggle />
            </div>
          </div>
          <div style={{ flex: 1, minHeight: 0 }}>
            <CodeEditor workspaceId={null} workspaceName="" rootPath="" />
          </div>
        </div>
        <ToastStack />
        <HostKeyPromptHost />
        <AgentCertPromptHost />
      </>
    );
  }

  const isRemote = current.kind === "remote";
  const sftpReady = isRemote; // 本地工作区下需要先选一个已保存连接才能用文件互传，尚未接（DESIGN.md §3.1.3）
  // Agent 协议的工作区底层是 AgentBrowser（Windows 路径语义、走 Agent 协议的浏览/传输命令），
  // 不是 SftpBrowser——按钮沿用同一个位置，但文案跟着协议换，避免用户以为点的是 SSH SFTP。
  const sftpTabLabel = workspaceProtocol === "agent" ? "文件传输" : "SFTP";

  const handleTerminalToggle = async () => {
    if (terminalTabs.length > 0) {
      togglePanel();
      return;
    }
    try {
      if (isRemote && current.connection_id) {
        if (workspaceProtocol === "agent") {
          await openTerminal({ kind: "agent", profileId: current.connection_id, cwd: current.root_path });
        } else {
          await sshService.connect(current.connection_id);
          await openTerminal({ kind: "ssh", profileId: current.connection_id, cwd: current.root_path });
        }
      } else if (!isRemote) {
        await openTerminal({ kind: "local", cwd: current.root_path });
      }
    } catch (error) {
      console.error("打开终端失败", error);
      pushToast("error", `打开终端失败：${formatError(error)}`);
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
    <>
      {/* 这个窗口本身就是一个独立的"工作区模块"进程（`docs/HOME_MODES_DESIGN.md`
          §3.5），不再需要和 HomeShell 共存一屏、靠 `showPicker` 切换可见性——
          HomeShell 现在是 `ssh` 模块自己独立进程的内容。点"返回首页"是
          `goHome()`：唤起/聚焦另一个进程里的启动器，不影响这个窗口本身继续开着
          （现有的 TerminalPanel/编辑器标签保活机制不受影响，这里没有任何子树会
          被卸载）。 */}
      <div style={{ display: "flex", flexDirection: "column", height: "100vh" }}>
      <div className="tab-bar">
        <button className="quick-tool-btn" onClick={() => void goHome()} title="返回首页">
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
              .map((w): ContextMenuItem => {
                // 绿点＝终端有界保活集合里还留着这个工作区（切过去终端原样恢复，
                // 不是新连的）；灰点＝已经被 LRU 淘汰，切过去会是一个全新终端。
                const online = residentWorkspaceIds.includes(w.id);
                return {
                  label: `${online ? "🟢" : "⚪"} ${w.kind === "local" ? "💻" : "🖥"} ${w.display_name} — ${w.root_path}`,
                  onClick: () => void switchWorkspace(w),
                };
              })}
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
            <span>{sftpTabLabel}</span>
          </div>
        )}

        <div className="quick-tools">
          <button
            className={`quick-tool-btn ${activeView === "sftp" ? "active" : ""}`}
            disabled={!sftpReady}
            title={sftpReady ? `${sftpTabLabel}（跨目录自由浏览与互传）` : "本地工作区需先选择一个已保存连接才能用文件互传，尚未实现"}
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
          {/* 打开一个不属于当前工作区的游离文件（拖拽/文件关联之外的第三个入口，
              2026-09-03 需求）——和 Explorer 树里打开的文件混在同一个 tab 栏，
              用 CodeEditor.tsx 里的小图标 + 完整路径 tooltip 区分。 */}
          <button className="quick-tool-btn" title="打开文件 (Ctrl+O)" onClick={() => void openFileDialog()}>
            <FilePlus2 />
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
            {/* 目录树刷新（2026-09-01 用户反馈：目录树没有任何办法刷新，工作区外部
                改了文件看不到最新状态）——只在资源管理器视图下有意义，搜索视图
                切过去这个按钮没有对应操作。 */}
            {sidebarMode === "explorer" && (
              <button
                className="quick-tool-btn"
                style={{ marginLeft: "auto" }}
                title="刷新目录树"
                onClick={() => void useExplorerStore.getState().refreshAll(current.id, current.root_path)}
              >
                <RotateCw />
              </button>
            )}
          </div>
          {/* display:flex 是关键——`.project-tree`（ExplorerTree.tsx）的 `flex: 1`
              要靠这个才能真正撑满剩余高度，不然内容少时底部空白区域不属于
              `.project-tree`，右键在那片区域点没有反应（2026-09-03 用户反馈）。
              这层本身不再自己 overflow:auto，滚动交给子组件（.project-tree /
              SearchPanel 都是 height:100% + 自己的 overflow-y:auto）内部处理。 */}
          <div style={{ flex: 1, display: "flex", flexDirection: "column", overflow: "hidden" }}>
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
                onCompare={(leftPath, rightPath) => {
                  useEditorStore
                    .getState()
                    .openDiff(current.id, leftPath, rightPath)
                    .catch((e) => pushToast("error", `对比失败：${formatError(e)}`));
                  setActiveView("editor");
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
              {!current.connection_id ? null : workspaceProtocol === "agent" ? (
                <AgentBrowser
                  profileId={current.connection_id}
                  workspaceId={current.id}
                  initialRemotePath={current.last_sftp_remote_path ?? current.root_path}
                  initialLocalPath={current.last_sftp_local_path ?? undefined}
                  onPathsChange={updateLastSftpPaths}
                />
              ) : sftpViewingPath ? (
                <SftpFileViewer
                  profileId={current.connection_id}
                  path={sftpViewingPath}
                  onBack={() => setSftpViewingPath(null)}
                />
              ) : (
                <SftpBrowser
                  profileId={current.connection_id}
                  workspaceId={current.id}
                  initialRemotePath={current.last_sftp_remote_path ?? current.root_path}
                  initialLocalPath={current.last_sftp_local_path ?? undefined}
                  onPathsChange={updateLastSftpPaths}
                  onOpenFile={(entry) => setSftpViewingPath(entry.path)}
                />
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
                  ? { kind: workspaceProtocol === "agent" ? "agent" : "ssh", profileId: current.connection_id, cwd: current.root_path }
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
      </div>

      <ToastStack />
      <HostKeyPromptHost />
      <AgentCertPromptHost />
    </>
  );
}

export default App;
