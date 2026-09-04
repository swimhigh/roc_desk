import React, { useEffect, useRef, useState } from "react";
import {
  ArrowUp,
  Code2,
  Copy,
  File as FileIcon,
  Folder,
  FolderPlus,
  Home,
  HardDrive,
  Laptop,
  Pencil,
  Plus,
  RotateCw,
  Search,
  Server,
  Trash2,
  X,
} from "lucide-react";
import { localFsService } from "../../services/localFsService";
import { localFileService } from "../../services/fsService";
import { sftpService } from "../../services/sftpService";
import { agentService } from "../../services/agentService";
import { connectionService } from "../../services/connectionService";
import { useModeStore } from "../../stores/modeStore";
import { useToastStore } from "../shared/Toast";
import { ConfirmDialog } from "../shared/ConfirmDialog";
import { ContextMenu, type ContextMenuItem } from "../shared/ContextMenu";
import { ThemeToggle } from "../shared/ThemeToggle";
import { formatError } from "../../utils/error";
import { formatBytes } from "../../utils/format";
import { classifyPreview, hasNoExtension } from "../../utils/previewFile";
import { AGENT_ROOT, agentParentPath, isAgentRoot } from "../../utils/windowsPath";
import type { ConnectionProfile, FileEntry } from "../../types/bindings";

type Side = "left" | "right";
type Protocol = "local" | "ssh" | "agent";
const OTHER: Record<Side, Side> = { left: "right", right: "left" };

/** 双栏各自的完整标签页列表持久化（用户 2026-09-04 反馈："实际左右都打开过多个
 * 目录，但关闭后下一次打开，左右都只有一个目录了"）——只存"够重建一个标签"的
 * 最小信息（协议/连接 id/路径），标题、选中项这些运行期状态不用存，重新打开时
 * `navigate` 会自然填回来。旧版本只记了每栏"最后一个路径"（`roc_desk-explorer-
 * ${side}-path`），这里保留一份兼容读取：新 key 没有数据时退回旧 key，不让老用户
 * 升级后连仅有的这一个记忆都丢掉。 */
const EXPLORER_TABS_KEY = "roc_desk-explorer-tabs";
interface PersistedTab {
  protocol: Protocol;
  connectionId?: string;
  path: string;
}
function loadPersistedTabs(side: Side): PersistedTab[] {
  try {
    const raw = localStorage.getItem(EXPLORER_TABS_KEY);
    if (raw) {
      const parsed = JSON.parse(raw) as Partial<Record<Side, PersistedTab[]>>;
      const tabs = parsed[side];
      if (Array.isArray(tabs) && tabs.length > 0) {
        return tabs.filter((t): t is PersistedTab => !!t && typeof t.path === "string" && typeof t.protocol === "string");
      }
    }
  } catch {
    /* 存量数据格式不对就当没有，走下面的旧 key/新建默认标签兜底 */
  }
  const legacyPath = localStorage.getItem(`roc_desk-explorer-${side}-path`);
  return legacyPath ? [{ protocol: "local", path: legacyPath }] : [];
}
function savePersistedTabs(pane: Record<Side, { tabs: FileTab[] }>) {
  const data: Record<Side, PersistedTab[]> = {
    left: pane.left.tabs.map((t) => ({ protocol: t.protocol, connectionId: t.connectionId, path: t.path })),
    right: pane.right.tabs.map((t) => ({ protocol: t.protocol, connectionId: t.connectionId, path: t.path })),
  };
  localStorage.setItem(EXPLORER_TABS_KEY, JSON.stringify(data));
}

interface FileTab {
  id: string;
  protocol: Protocol;
  /** protocol !== "local" 时必填——对应一条已保存的连接档案。 */
  connectionId?: string;
  label: string;
  path: string;
  entries: FileEntry[];
  loading: boolean;
  selected: string[];
  anchor: string | null;
  filter: string;
  renaming: string | null;
  renameValue: string;
}

interface PaneState {
  tabs: FileTab[];
  activeId: string;
}

let tabSeq = 0;
function makeTabId(): string {
  tabSeq += 1;
  return `tab-${Date.now()}-${tabSeq}`;
}

function formatTime(epochSeconds: number | null): string {
  if (epochSeconds === null) return "—";
  const d = new Date(epochSeconds * 1000);
  return `${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")} ${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}

/** SSH 默认落地目录——和 `RemoteTool/HomeShell.tsx` 的同名启发式一致：没有更可靠的
 *"用户主目录"信息来源（不会为了猜一个默认值特地登录一次去问 `$HOME`）。 */
function defaultSshPath(username: string): string {
  return username.startsWith("root") ? "/root" : `/home/${username}`;
}

function sortEntries(entries: FileEntry[]): FileEntry[] {
  return [...entries].sort((a, b) => {
    if (a.is_dir !== b.is_dir) return a.is_dir ? -1 : 1;
    return a.name.toLowerCase().localeCompare(b.name.toLowerCase());
  });
}

/** 计算本地路径的上级目录；Windows 盘符根（如 "C:/"）没有上级，返回 null。*/
function parentOfLocal(path: string): string | null {
  const trimmed = path.replace(/\/+$/, "");
  const idx = trimmed.lastIndexOf("/");
  if (idx <= 0) return null;
  const parent = trimmed.slice(0, idx);
  return /^[A-Za-z]:$/.test(parent) ? `${parent}/` : parent;
}

function parentOfUnix(path: string): string | null {
  const trimmed = path.replace(/\/+$/, "");
  if (trimmed === "" || trimmed === "/") return null;
  const idx = trimmed.lastIndexOf("/");
  return idx <= 0 ? "/" : trimmed.slice(0, idx);
}

/** 上级目录——三种协议的路径语义都不一样（本地/SSH 是正常文件系统路径，Agent/
 * Windows 目标没有单一根，盘符根的上一级是虚拟的盘符列表，见 utils/windowsPath.ts）。*/
function upPath(tab: FileTab): string | null {
  if (tab.protocol === "local") return parentOfLocal(tab.path);
  if (tab.protocol === "agent") return isAgentRoot(tab.path) ? null : agentParentPath(tab.path);
  return parentOfUnix(tab.path);
}

function joinChildPath(tab: FileTab, dir: string, name: string): string {
  if (tab.protocol === "agent") return `${dir.replace(/\\+$/, "")}\\${name}`;
  return `${dir.replace(/\/+$/, "")}/${name}`;
}

/** 按协议分流列目录——Agent 在虚拟根（AGENT_ROOT）要走"盘符列表"而不是真的
 * `list_dir`，和 `stores/agentBrowseStore.ts` 里 AgentBrowser 用的是同一条规则。*/
async function dispatchListDir(tab: FileTab): Promise<FileEntry[]> {
  if (tab.protocol === "local") return localFsService.listDir(tab.path);
  if (tab.protocol === "ssh") return sftpService.listDir(tab.connectionId!, tab.path);
  if (isAgentRoot(tab.path)) {
    const roots = await agentService.listRoots(tab.connectionId!);
    return roots.map((r) => ({ name: r, path: r, is_dir: true, size: null, modified: null }));
  }
  return agentService.listDir(tab.connectionId!, tab.path);
}

async function dispatchDelete(tab: FileTab, entry: FileEntry): Promise<void> {
  if (tab.protocol === "local") return localFsService.deletePath(entry.path, entry.is_dir);
  if (tab.protocol === "ssh") return sftpService.delete(tab.connectionId!, entry.path, entry.is_dir);
  return agentService.delete(tab.connectionId!, entry.path, entry.is_dir);
}

async function dispatchRename(tab: FileTab, from: string, to: string): Promise<void> {
  if (tab.protocol === "local") return localFsService.rename(from, to);
  if (tab.protocol === "ssh") return sftpService.rename(tab.connectionId!, from, to);
  return agentService.rename(tab.connectionId!, from, to);
}

async function dispatchOpenExternally(tab: FileTab, entry: FileEntry): Promise<void> {
  if (tab.protocol === "local") return localFileService.openExternally(entry.path);
  if (tab.protocol === "ssh") return sftpService.openExternally(tab.connectionId!, entry.path);
  throw new Error("Agent（远程 Windows）协议暂不支持直接打开文件，可以先复制到本地");
}

/** 双栏之间复制/移动的核心分流——覆盖"两侧协议组合"这个矩阵，讲清楚哪些组合支持、
 * 哪些不支持，而不是含糊地失败：
 * - 本地↔本地：走本地文件系统操作；
 * - 本地↔远程（SSH/Agent）：走已有的上传/下载命令，"移动"= 传输成功后删源；
 * - 同一个远程连接内部：只能"移动"（远程 rename），SFTP/Agent 协议都没有暴露
 *   "服务器内复制"这个原语（现有 SftpBrowser/AgentBrowser 同样没有这个能力，
 *   不是这里新引入的限制）；
 * - 不同连接之间、或协议不同的两个远程：不支持直连互传，需要用户自己先落地到本地
 *   再传到另一边——真正实现"服务器到服务器"需要额外的中转/进度 UI，超出这一版
 *   "简洁易用"的范围。
 */
async function transferOne(src: FileTab, entry: FileEntry, dst: FileTab, mode: "copy" | "move"): Promise<void> {
  const targetPath = joinChildPath(dst, dst.path, entry.name);

  if (src.protocol === "local" && dst.protocol === "local") {
    if (mode === "copy") return localFsService.copy(entry.path, targetPath, entry.is_dir);
    return localFsService.move(entry.path, targetPath, entry.is_dir);
  }

  if (src.protocol === "local" && dst.protocol !== "local") {
    const svc = dst.protocol === "ssh" ? sftpService : agentService;
    await svc.uploadEntry(dst.connectionId!, entry.path, entry.is_dir, dst.path, makeTabId());
    if (mode === "move") await localFsService.deletePath(entry.path, entry.is_dir);
    return;
  }

  if (src.protocol !== "local" && dst.protocol === "local") {
    const svc = src.protocol === "ssh" ? sftpService : agentService;
    await svc.downloadEntry(src.connectionId!, entry.path, entry.is_dir, dst.path, makeTabId());
    if (mode === "move") await dispatchDelete(src, entry);
    return;
  }

  // 两侧都不是本地
  if (src.protocol === dst.protocol && src.connectionId === dst.connectionId) {
    if (mode === "move") return dispatchRename(src, entry.path, targetPath);
    throw new Error("同一个远程连接内暂不支持复制，只支持移动（相当于远程重命名）");
  }
  throw new Error("暂不支持不同远程连接/协议之间直接互传，请先复制到本地再传到目标位置");
}

/** 用系统默认程序打开而不是 roc_desk 编辑器的分类——可执行文件（EXE 由 Windows
 * 直接执行）；Office 文档（doc/docx/xls/xlsx/ppt/pptx，用户 2026-09-04 明确要求
 * "双击时请用系统默认程序打开"）——CodeEditor 里的 Word/Excel 只读预览面板对
 * "看一眼内容"够用，但用户真要编辑 Office 文档，交给本机真正装着的 Word/Excel/
 * PowerPoint 才是对的，不该被 roc_desk 的只读预览拦在中间。 */
const OPEN_EXTERNALLY_KINDS = new Set(["executable", "word", "excel", "legacy-office"]);

/** `classifyPreview` 把 `.bat`/`.cmd` 归成普通文本（在编辑器/SFTP 查看器里这是对
 * 的——那些场景就是要把脚本内容当文本编辑），但在资源管理器里双击就是想跟 Windows
 * 资源管理器一样直接运行它（用户 2026-09-04 明确要求）。这两个扩展名本来就是同一种
 * 批处理脚本，不单独为 `.bat` 开例外、放过 `.cmd`——不然行为不一致更让人困惑。 */
const WINDOWS_RUNNABLE_EXTENSIONS = new Set(["bat", "cmd"]);

function extensionOf(path: string): string {
  const name = path.split(/[\\/]/).pop() ?? path;
  return name.includes(".") ? name.split(".").pop()!.toLowerCase() : "";
}

function tabIcon(protocol: Protocol) {
  if (protocol === "local") return <Laptop size={11} />;
  if (protocol === "ssh") return <Server size={11} />;
  return <HardDrive size={11} />;
}

/**
 * 资源管理器模块（Total Commander 式本地双栏文件管理，`docs/HOME_MODES_DESIGN.md`
 * §3.2/§6 Phase 4）——`--mode=explorer` 启动的模块窗口只挂载这一个组件。
 *
 * 用户 2026-09-04 追加需求："左右都是可以增加N多标签的""也请支持连远程SSH目录"
 * "还有AGENT远程WINDOWS目录"——每一栏现在是一组标签页（本地/SSH/Agent 混着开都
 * 行），标签内部的浏览/选中/过滤/重命名各自独立，只有"复制/移动到对面"这类跨栏
 * 操作会牵扯到另一栏当前激活的标签。
 *
 * 仍然有意从简：不做拖拽（用按钮做复制/移动）、不做标签页跨次启动的持久化恢复
 * （只记左右各自最后停留的本地路径，和之前一致）、远程新建文件夹/服务器内复制
 * 这类现有 SftpBrowser/AgentBrowser 本来就没有的能力这里也不补，符合"简洁易用
 * 为主，后续使用中再加功能"。
 */
export const LocalExplorerScreen: React.FC = () => {
  const goHome = useModeStore((s) => s.goHome);
  const spawnModule = useModeStore((s) => s.spawnModule);
  const push = useToastStore((s) => s.push);
  const [pane, setPane] = useState<Record<Side, PaneState>>({
    left: { tabs: [], activeId: "" },
    right: { tabs: [], activeId: "" },
  });
  const [activeSide, setActiveSide] = useState<Side>("left");
  const [drives, setDrives] = useState<string[]>([]);
  const [connections, setConnections] = useState<ConnectionProfile[]>([]);
  const [menu, setMenu] = useState<{ side: Side; tabId: string; entry: FileEntry; x: number; y: number } | null>(null);
  const [addTabMenu, setAddTabMenu] = useState<{ side: Side; x: number; y: number } | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<{ side: Side; tabId: string; entries: FileEntry[] } | null>(null);
  const listRefs = useRef<Record<Side, HTMLDivElement | null>>({ left: null, right: null });
  const restoringRef = useRef(true);

  const getTab = (side: Side, tabId: string): FileTab | undefined => pane[side].tabs.find((t) => t.id === tabId);
  const activeTab = (side: Side): FileTab | undefined => getTab(side, pane[side].activeId);

  const updateTab = (side: Side, tabId: string, patch: Partial<FileTab> | ((t: FileTab) => Partial<FileTab>)) => {
    setPane((prev) => ({
      ...prev,
      [side]: {
        ...prev[side],
        tabs: prev[side].tabs.map((t) => (t.id === tabId ? { ...t, ...(typeof patch === "function" ? patch(t) : patch) } : t)),
      },
    }));
  };

  // 接收完整的 `tab` 对象，而不是只传 `tabId` 再回头查 `pane` state——`addTab`
  // 里 `setPane` 加完新标签之后立刻调用这个函数时，React 的 state 更新是异步的，
  // 这次调用所在的渲染闭包里 `pane` 还是加标签前的旧值，`getTab` 会查不到刚创建
  // 的这个标签而直接早退，新标签目录列表永远没有真正发起过请求——界面上看到的是
  // "标签建好了、路径也显示对了，但内容一直空，得手动点刷新才会加载"（真实反馈，
  // 2026-09-04 用户报告新建的 SSH 标签"必须要刷新才能看到所有文件"）。调用方手上
  // 已经有 `tab` 对象的（`addTab` 自己刚创建的、`renderPane` 里从 `activeTab(side)`
  // 拿到的）直接传对象，不需要再绕一趟可能滞后的 state 查找。
  const navigate = async (side: Side, tab: FileTab, path: string) => {
    updateTab(side, tab.id, { loading: true });
    try {
      const entries = await dispatchListDir({ ...tab, path });
      const label = tab.protocol === "local" ? path.split(/[\\/]/).filter(Boolean).pop() || path : tab.label;
      updateTab(side, tab.id, { path, entries, loading: false, selected: [], anchor: null, renaming: null, label });
    } catch (e) {
      updateTab(side, tab.id, { loading: false });
      push("error", `打开目录失败：${formatError(e)}`);
    }
  };

  const refresh = async (side: Side, tabId: string) => {
    const tab = getTab(side, tabId);
    if (tab) await navigate(side, tab, tab.path);
  };

  const addTab = (side: Side, protocol: Protocol, connectionId: string | undefined, label: string, path: string) => {
    const id = makeTabId();
    const tab: FileTab = {
      id,
      protocol,
      connectionId,
      label,
      path,
      entries: [],
      loading: false,
      selected: [],
      anchor: null,
      filter: "",
      renaming: null,
      renameValue: "",
    };
    setPane((prev) => ({ ...prev, [side]: { tabs: [...prev[side].tabs, tab], activeId: id } }));
    setActiveSide(side);
    void navigate(side, tab, path);
  };

  const closeTab = (side: Side, tabId: string) => {
    setPane((prev) => {
      const tabs = prev[side].tabs.filter((t) => t.id !== tabId);
      if (tabs.length === 0) return prev; // 每栏至少留一个标签，不允许关成空白栏
      const activeId = prev[side].activeId === tabId ? tabs[tabs.length - 1].id : prev[side].activeId;
      return { ...prev, [side]: { tabs, activeId } };
    });
  };

  // 双栏各自的完整标签页列表在关闭/重开之间原样恢复（见上面 `loadPersistedTabs`
  // 的注释）——连接列表要先拿到手才能把 SSH/Agent 标签的显示名对上，所以这里是
  // "先等连接列表、再逐个恢复标签"，不是并发两件事各自跑。`restoringRef` 期间
  // 下面的持久化 effect 不写盘，避免恢复过程中每加一个标签就存一次不完整的列表，
  // 把还没恢复完的中间状态覆盖回 localStorage。
  useEffect(() => {
    void localFsService.listDrives().then(setDrives).catch(() => {});
    (async () => {
      const allConnections = await connectionService.list().catch(() => [] as ConnectionProfile[]);
      const remoteConnections = allConnections.filter((c) => c.protocol === "ssh" || c.protocol === "agent");
      setConnections(remoteConnections);
      const labelFor = (t: PersistedTab) =>
        t.protocol === "local"
          ? t.path.split(/[\\/]/).filter(Boolean).pop() || t.path
          : remoteConnections.find((c) => c.id === t.connectionId)?.name ?? "远程连接";

      for (const side of ["left", "right"] as Side[]) {
        const persisted = loadPersistedTabs(side);
        if (persisted.length === 0) {
          const home = await localFsService.homeDir().catch(() => "C:/");
          addTab(side, "local", undefined, home.split(/[\\/]/).filter(Boolean).pop() || home, home);
        } else {
          for (const t of persisted) {
            addTab(side, t.protocol, t.connectionId, labelFor(t), t.path);
          }
        }
      }
      restoringRef.current = false;
    })();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (restoringRef.current) return;
    savePersistedTabs(pane);
  }, [pane]);

  const visible = (tab: FileTab): FileEntry[] => {
    const sorted = sortEntries(tab.entries);
    if (!tab.filter) return sorted;
    const q = tab.filter.toLowerCase();
    return sorted.filter((e) => e.name.toLowerCase().includes(q));
  };

  /** 本地文件双击（用户 2026-09-04 需求）——可执行文件/Office 文档用系统默认程序
   * 打开，其它一律认为是 roc_desk 编辑器能处理的"可编辑"文件（文本自不必说，
   * 图片/PDF/不支持预览的二进制 CodeEditor 也都有对应的只读展示面板，不会打不开），
   * 另开一个编辑器模块窗口直接定位到这个文件——比"加入当前某个编辑器窗口的标签页"
   * 简单：资源管理器是独立进程，不知道用户手上哪个编辑器窗口才是"当前"那个，也
   * 没有现成的跨进程"塞一个标签过去"的通道，新开窗口不依赖这些、结果对用户来说
   * 也是立刻看到文件内容。复用 `classifyPreview`——和编辑器/SFTP 查看器是同一套
   * 分类，没有扩展名的文件（Linux 风格可执行文件）额外嗅探文件头。 */
  const openLocalFile = async (entry: FileEntry) => {
    try {
      let external = OPEN_EXTERNALLY_KINDS.has(classifyPreview(entry.path)) || WINDOWS_RUNNABLE_EXTENSIONS.has(extensionOf(entry.path));
      if (!external && hasNoExtension(entry.path)) {
        external = await localFileService.peekIsBinary(entry.path).catch(() => false);
      }
      if (external) {
        await localFileService.openExternally(entry.path);
      } else {
        await spawnModule("editor", entry.path);
      }
    } catch (e) {
      push("error", `打开失败：${formatError(e)}`);
    }
  };

  const openEntry = (side: Side, tab: FileTab, entry: FileEntry) => {
    if (entry.is_dir) {
      void navigate(side, tab, entry.path);
      return;
    }
    if (tab.protocol === "local") {
      void openLocalFile(entry);
      return;
    }
    void dispatchOpenExternally(tab, entry).catch((e) => push("error", `打开失败：${formatError(e)}`));
  };

  const selectEntry = (side: Side, tab: FileTab, entry: FileEntry, e: React.MouseEvent) => {
    setActiveSide(side);
    updateTab(side, tab.id, (t) => {
      if (e.shiftKey && t.anchor) {
        const list = visible(t);
        const ai = list.findIndex((it) => it.path === t.anchor);
        const bi = list.findIndex((it) => it.path === entry.path);
        if (ai !== -1 && bi !== -1) {
          const [lo, hi] = ai < bi ? [ai, bi] : [bi, ai];
          return { selected: list.slice(lo, hi + 1).map((it) => it.path) };
        }
      }
      if (e.ctrlKey || e.metaKey) {
        const set = new Set(t.selected);
        if (set.has(entry.path)) set.delete(entry.path);
        else set.add(entry.path);
        return { selected: [...set], anchor: entry.path };
      }
      return { selected: [entry.path], anchor: entry.path };
    });
  };

  const onListKeyDown = (side: Side, tab: FileTab) => (e: React.KeyboardEvent<HTMLDivElement>) => {
    const list = visible(tab);
    if (e.key === "Delete") {
      const selected = list.filter((it) => tab.selected.includes(it.path));
      if (selected.length > 0) setDeleteTarget({ side, tabId: tab.id, entries: selected });
      return;
    }
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    if (list.length === 0) return;
    e.preventDefault();
    const idx = list.findIndex((it) => it.path === tab.anchor);
    const nextIdx = idx === -1 ? 0 : e.key === "ArrowDown" ? Math.min(idx + 1, list.length - 1) : Math.max(idx - 1, 0);
    const next = list[nextIdx];
    updateTab(side, tab.id, { selected: [next.path], anchor: next.path });
    listRefs.current[side]?.querySelector<HTMLElement>(`[data-path="${CSS.escape(next.path)}"]`)?.scrollIntoView({ block: "nearest" });
  };

  const startRename = (side: Side, tab: FileTab, entry: FileEntry) => {
    updateTab(side, tab.id, { renaming: entry.path, renameValue: entry.name });
  };

  const commitRename = async (side: Side, tab: FileTab, entry: FileEntry) => {
    const newName = tab.renameValue.trim();
    updateTab(side, tab.id, { renaming: null });
    if (!newName || newName === entry.name) return;
    const to = joinChildPath(tab, tab.path, newName);
    try {
      await dispatchRename(tab, entry.path, to);
      await refresh(side, tab.id);
    } catch (e) {
      push("error", `重命名失败：${formatError(e)}`);
    }
  };

  const newFolder = async (side: Side, tab: FileTab) => {
    if (tab.protocol !== "local") return;
    const existing = new Set(tab.entries.map((e) => e.name));
    let name = "新建文件夹";
    let i = 1;
    while (existing.has(name)) {
      i += 1;
      name = `新建文件夹 ${i}`;
    }
    const path = joinChildPath(tab, tab.path, name);
    try {
      await localFsService.createDir(path);
      await refresh(side, tab.id);
      updateTab(side, tab.id, { renaming: path, renameValue: name, selected: [path], anchor: path });
    } catch (e) {
      push("error", `新建文件夹失败：${formatError(e)}`);
    }
  };

  const transferSelected = async (side: Side, mode: "copy" | "move") => {
    const src = activeTab(side);
    const dst = activeTab(OTHER[side]);
    if (!src || !dst) return;
    const items = src.entries.filter((e) => src.selected.includes(e.path));
    if (items.length === 0) return;
    let failed = 0;
    for (const item of items) {
      try {
        await transferOne(src, item, dst, mode);
      } catch (e) {
        failed += 1;
        push("error", `${item.name}：${formatError(e)}`);
      }
    }
    await refresh(OTHER[side], dst.id);
    if (mode === "move") await refresh(side, src.id);
    if (failed === 0) push("success", `${mode === "copy" ? "复制" : "移动"}完成（${items.length} 项）`);
  };

  const confirmDelete = async () => {
    if (!deleteTarget) return;
    const { side, tabId, entries } = deleteTarget;
    const tab = getTab(side, tabId);
    setDeleteTarget(null);
    if (!tab) return;
    let failed = 0;
    for (const entry of entries) {
      try {
        await dispatchDelete(tab, entry);
      } catch (e) {
        failed += 1;
        push("error", `${entry.name}：${formatError(e)}`);
      }
    }
    await refresh(side, tabId);
    if (failed === 0) push("success", `已删除 ${entries.length} 项`);
  };

  const currentDrive = (path: string) => {
    const m = /^([A-Za-z]:)\//.exec(path);
    return m ? m[1] : "";
  };

  const homePath = async (tab: FileTab): Promise<string> => {
    if (tab.protocol === "local") return localFsService.homeDir().catch(() => "C:/");
    if (tab.protocol === "agent") return AGENT_ROOT;
    const conn = connections.find((c) => c.id === tab.connectionId);
    return defaultSshPath(conn?.username ?? "root");
  };

  const openAddTabMenuAt = (side: Side, e: React.MouseEvent) => {
    const rect = e.currentTarget.getBoundingClientRect();
    setAddTabMenu({ side, x: rect.left, y: rect.bottom + 4 });
  };

  const addTabMenuItems: ContextMenuItem[] = addTabMenu
    ? (() => {
        const side = addTabMenu.side;
        const items: ContextMenuItem[] = [
          {
            label: "💻 本地目录",
            onClick: () => void localFsService.homeDir().then((h) => addTab(side, "local", undefined, h.split(/[\\/]/).filter(Boolean).pop() || h, h)),
          },
        ];
        const sshConns = connections.filter((c) => c.protocol === "ssh");
        const agentConns = connections.filter((c) => c.protocol === "agent");
        sshConns.forEach((c, i) => {
          items.push({
            label: `🖥 ${c.name}（${c.username}@${c.host}）`,
            separatorBefore: i === 0,
            onClick: () => addTab(side, "ssh", c.id, c.name, defaultSshPath(c.username)),
          });
        });
        agentConns.forEach((c, i) => {
          items.push({
            label: `🗄 ${c.name}（${c.username}@${c.host}）`,
            separatorBefore: i === 0,
            onClick: () => addTab(side, "agent", c.id, c.name, AGENT_ROOT),
          });
        });
        return items;
      })()
    : [];

  const renderPane = (side: Side) => {
    const paneState = pane[side];
    const tab = activeTab(side);
    const isActive = activeSide === side;
    if (!tab) return <div style={{ flex: 1 }} />;
    const list = visible(tab);
    return (
      <div
        style={{ flex: 1, display: "flex", flexDirection: "column", minWidth: 0, minHeight: 0 }}
        onMouseDownCapture={() => setActiveSide(side)}
      >
        <div style={{ display: "flex", alignItems: "center", height: 26, overflowX: "auto", borderBottom: "1px solid var(--border-subtle)", flexShrink: 0 }}>
          {paneState.tabs.map((t) => (
            <div
              key={t.id}
              onClick={() => {
                setActiveSide(side);
                setPane((prev) => ({ ...prev, [side]: { ...prev[side], activeId: t.id } }));
              }}
              className={`explorer-tab ${t.id === paneState.activeId ? "active" : ""}`}
              title={t.protocol === "local" ? t.path : t.label}
            >
              {tabIcon(t.protocol)}
              <span className="explorer-tab-label">{t.label}</span>
              {paneState.tabs.length > 1 && (
                <X
                  size={11}
                  className="explorer-tab-close"
                  onClick={(e) => {
                    e.stopPropagation();
                    closeTab(side, t.id);
                  }}
                />
              )}
            </div>
          ))}
          <button className="quick-tool-btn" style={{ width: 22, height: 22, flexShrink: 0 }} title="新建标签页" onClick={(e) => openAddTabMenuAt(side, e)}>
            <Plus style={{ width: 12, height: 12 }} />
          </button>
        </div>

        <div className="sftp-toolbar">
          {tab.protocol === "local" && (
            <select
              className="form-input"
              style={{ height: 22, fontSize: 12, width: 56 }}
              value={currentDrive(tab.path)}
              onChange={(e) => e.target.value && void navigate(side, tab, `${e.target.value}/`)}
              title="切换盘符"
            >
              {drives.map((d) => (
                <option key={d} value={d.replace(/\/$/, "")}>
                  {d}
                </option>
              ))}
            </select>
          )}
          <button
            className="btn ghost sm"
            title="上级目录"
            disabled={upPath(tab) === null}
            onClick={() => {
              const up = upPath(tab);
              if (up !== null) void navigate(side, tab, up);
            }}
          >
            <ArrowUp style={{ width: 12, height: 12 }} />
          </button>
          <button className="btn ghost sm" title="默认目录" onClick={() => void homePath(tab).then((h) => navigate(side, tab, h))}>
            <Home style={{ width: 12, height: 12 }} />
          </button>
          <input
            className="form-input"
            style={{ flex: 1, height: 22, fontSize: 12, fontFamily: "var(--font-mono)" }}
            value={tab.path}
            placeholder={tab.protocol === "agent" && isAgentRoot(tab.path) ? "此电脑（盘符列表）" : undefined}
            onChange={(e) => updateTab(side, tab.id, { path: e.target.value })}
            onKeyDown={(e) => {
              if (e.key === "Enter") void navigate(side, tab, tab.path);
            }}
          />
          <button className="btn ghost sm" title="刷新" onClick={() => void refresh(side, tab.id)}>
            <RotateCw style={{ width: 12, height: 12 }} />
          </button>
        </div>

        <div className="sftp-filter-bar">
          <Search style={{ width: 12, height: 12, color: "var(--text-secondary)", flexShrink: 0 }} />
          <input
            className="form-input"
            style={{ flex: 1, height: 22, fontSize: 12 }}
            placeholder="过滤文件名…"
            value={tab.filter}
            onChange={(e) => updateTab(side, tab.id, { filter: e.target.value })}
            onKeyDown={(e) => {
              if (e.key === "Escape" && tab.filter) {
                e.stopPropagation();
                updateTab(side, tab.id, { filter: "" });
              }
            }}
          />
          {tab.filter && (
            <button className="btn ghost sm" title="清空过滤" onClick={() => updateTab(side, tab.id, { filter: "" })}>
              <X style={{ width: 12, height: 12 }} />
            </button>
          )}
        </div>

        <div
          ref={(el) => { listRefs.current[side] = el; }}
          tabIndex={0}
          onFocus={() => setActiveSide(side)}
          onKeyDown={onListKeyDown(side, tab)}
          style={{ flex: 1, overflowY: "auto", outline: isActive ? "1px solid var(--accent)" : "none", outlineOffset: -1 }}
        >
          <div className="file-header">
            <span>名称</span>
            <span>大小</span>
            <span>修改时间</span>
          </div>
          {tab.loading ? (
            <div style={{ padding: 16, fontSize: 12, color: "var(--text-secondary)" }}>加载中…</div>
          ) : list.length === 0 ? (
            <div style={{ padding: 16, fontSize: 12, color: "var(--text-secondary)" }}>{tab.filter ? "没有匹配的文件" : "此目录是空的"}</div>
          ) : (
            list.map((entry) => (
              <div
                key={entry.path}
                data-path={entry.path}
                className={`file-row ${tab.selected.includes(entry.path) ? "selected" : ""}`}
                onClick={(e) => selectEntry(side, tab, entry, e)}
                onDoubleClick={() => openEntry(side, tab, entry)}
                onContextMenu={(e) => {
                  e.preventDefault();
                  selectEntry(side, tab, entry, e);
                  setMenu({ side, tabId: tab.id, entry, x: e.clientX, y: e.clientY });
                }}
              >
                {tab.renaming === entry.path ? (
                  <input
                    className="tree-rename-input"
                    autoFocus
                    value={tab.renameValue}
                    onFocus={(e) => e.currentTarget.select()}
                    onClick={(e) => e.stopPropagation()}
                    onChange={(e) => updateTab(side, tab.id, { renameValue: e.target.value })}
                    onBlur={() => void commitRename(side, tab, entry)}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") void commitRename(side, tab, entry);
                      else if (e.key === "Escape") updateTab(side, tab.id, { renaming: null });
                    }}
                  />
                ) : (
                  <span style={{ display: "flex", alignItems: "center", gap: 6, overflow: "hidden" }}>
                    {entry.is_dir ? <Folder className="file-icon is-dir" /> : <FileIcon className="file-icon" />}
                    <span className="file-name">{entry.name}</span>
                  </span>
                )}
                <span className="file-size">{entry.is_dir ? "—" : entry.size !== null ? formatBytes(entry.size) : "—"}</span>
                <span className="file-time">{formatTime(entry.modified)}</span>
              </div>
            ))
          )}
        </div>
      </div>
    );
  };

  const activeSrcTab = activeTab(activeSide);
  const activeSelectedCount = activeSrcTab?.selected.length ?? 0;

  const menuItems: ContextMenuItem[] = (() => {
    if (!menu) return [];
    const tab = getTab(menu.side, menu.tabId);
    if (!tab) return [];
    return [
      {
        label: menu.entry.is_dir ? "打开" : tab.protocol === "local" ? "打开（可执行用默认程序，其它用编辑器）" : "用默认程序打开",
        onClick: () => openEntry(menu.side, tab, menu.entry),
      },
      { label: "重命名", onClick: () => startRename(menu.side, tab, menu.entry), separatorBefore: true },
      { label: `复制到${menu.side === "left" ? "右" : "左"}侧`, onClick: () => void transferSelected(menu.side, "copy") },
      { label: `移动到${menu.side === "left" ? "右" : "左"}侧`, onClick: () => void transferSelected(menu.side, "move") },
      {
        label: "删除",
        danger: true,
        separatorBefore: true,
        onClick: () => {
          const entries = tab.entries.filter((e) => tab.selected.includes(e.path));
          setDeleteTarget({ side: menu.side, tabId: tab.id, entries: entries.length > 0 ? entries : [menu.entry] });
        },
      },
    ];
  })();

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100vh" }}>
      <div className="tab-bar">
        <button className="quick-tool-btn" onClick={() => void goHome()} title="返回首页">
          <Home />
        </button>
        <Code2 className="app-icon" />
        <span className="workspace-name-btn" style={{ cursor: "default" }}>
          资源管理器
        </span>
        <div className="quick-tools" style={{ marginLeft: "auto" }}>
          <ThemeToggle />
        </div>
      </div>

      <div style={{ flex: 1, display: "flex", overflow: "hidden" }}>
        {renderPane("left")}
        <div className="sidebar-resize-handle" style={{ cursor: "default" }} />
        {renderPane("right")}
      </div>

      <div className="host-stats-bar" style={{ gap: 8 }}>
        <button
          className="btn ghost sm"
          title={activeSrcTab?.protocol === "local" ? "新建文件夹" : "远程目录暂不支持新建文件夹"}
          disabled={activeSrcTab?.protocol !== "local"}
          onClick={() => activeSrcTab && void newFolder(activeSide, activeSrcTab)}
        >
          <FolderPlus style={{ width: 12, height: 12 }} /> 新建文件夹
        </button>
        <button
          className="btn ghost sm"
          title="重命名（选中一项）"
          disabled={activeSelectedCount !== 1}
          onClick={() => {
            if (!activeSrcTab) return;
            const entry = activeSrcTab.entries.find((e) => e.path === activeSrcTab.selected[0]);
            if (entry) startRename(activeSide, activeSrcTab, entry);
          }}
        >
          <Pencil style={{ width: 12, height: 12 }} /> 重命名
        </button>
        <button className="btn ghost sm" title="复制到对面标签" disabled={activeSelectedCount === 0} onClick={() => void transferSelected(activeSide, "copy")}>
          <Copy style={{ width: 12, height: 12 }} /> 复制→对面
        </button>
        <button className="btn ghost sm" title="移动到对面标签" disabled={activeSelectedCount === 0} onClick={() => void transferSelected(activeSide, "move")}>
          <HardDrive style={{ width: 12, height: 12 }} /> 移动→对面
        </button>
        <button
          className="btn ghost sm"
          title="删除选中项"
          disabled={activeSelectedCount === 0}
          onClick={() => {
            if (!activeSrcTab) return;
            setDeleteTarget({ side: activeSide, tabId: activeSrcTab.id, entries: activeSrcTab.entries.filter((e) => activeSrcTab.selected.includes(e.path)) });
          }}
        >
          <Trash2 style={{ width: 12, height: 12 }} /> 删除
        </button>
        <span style={{ marginLeft: "auto", fontSize: 11, color: "var(--text-secondary)" }}>
          {activeSelectedCount > 0
            ? `已选中 ${activeSelectedCount} 项（${activeSide === "left" ? "左" : "右"}侧 · ${activeSrcTab?.label ?? ""}）`
            : "点选文件后可用工具栏或右键操作；「+」新建本地/SSH/Agent 标签"}
        </span>
      </div>

      {menu && <ContextMenu x={menu.x} y={menu.y} items={menuItems} onClose={() => setMenu(null)} />}
      {addTabMenu && <ContextMenu x={addTabMenu.x} y={addTabMenu.y} items={addTabMenuItems} onClose={() => setAddTabMenu(null)} />}

      {deleteTarget && (
        <ConfirmDialog
          open
          severity="danger"
          icon="🗑"
          title="确认删除"
          onDismiss={() => setDeleteTarget(null)}
          actions={
            <>
              <button className="btn ghost sm" onClick={() => setDeleteTarget(null)}>
                取消
              </button>
              <button className="btn danger-strong sm" onClick={() => void confirmDelete()}>
                删除
              </button>
            </>
          }
        >
          <p>
            确定要删除{deleteTarget.entries.length === 1 ? (
              <>
                {deleteTarget.entries[0].is_dir ? "目录" : "文件"} <strong>{deleteTarget.entries[0].name}</strong>
              </>
            ) : (
              <strong>{deleteTarget.entries.length} 项</strong>
            )}
            吗？此操作不可撤销。
          </p>
        </ConfirmDialog>
      )}
    </div>
  );
};
