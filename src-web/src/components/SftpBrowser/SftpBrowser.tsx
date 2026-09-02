import React, { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { Folder, File as FileIcon, ArrowUp, ArrowUpNarrowWide, ArrowDownNarrowWide, Laptop, Pencil, Server, RotateCw, History, ArrowLeftRight } from "lucide-react";
import { TransferLogDialog } from "./TransferLogDialog";
import { useSftpStore } from "../../stores/sftpStore";
import { useLocalFsStore } from "../../stores/localFsStore";
import { sftpService } from "../../services/sftpService";
import { localFsService } from "../../services/localFsService";
import { logSearchService } from "../../services/logSearchService";
import { useWorkspaceStore } from "../../stores/workspaceStore";
import { useToastStore } from "../shared/Toast";
import { ContextMenu, type ContextMenuItem } from "../shared/ContextMenu";
import { formatError, isCancelledTransferError } from "../../utils/error";
import { transferService } from "../../services/transferService";
import { useDualPaneDnd, type DndPayload, type PaneSide } from "../../hooks/useDualPaneDnd";
import type { FileEntry, SftpTransferProgressEvent } from "../../types/bindings";

function formatBytes(size: number | null): string {
  if (size === null) return "—";
  if (size < 1024) return `${size}B`;
  if (size < 1024 * 1024) return `${(size / 1024).toFixed(1)}KB`;
  if (size < 1024 * 1024 * 1024) return `${(size / 1024 / 1024).toFixed(1)}MB`;
  return `${(size / 1024 / 1024 / 1024).toFixed(1)}GB`;
}

function formatTime(epochSeconds: number | null): string {
  if (epochSeconds === null) return "—";
  const d = new Date(epochSeconds * 1000);
  return `${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")} ${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}

type Side = PaneSide;

type SortField = "name" | "size" | "modified";
interface SortState {
  field: SortField;
  asc: boolean;
}
const DEFAULT_SORT: SortState = { field: "name", asc: true };

/** 目录始终排在前面（和后端 list_dir 的默认排序保持一致），文件内部再按选中的
 * 字段排序——按大小/时间排序时目录之间仍按名称排列，因为目录没有 size/modified。 */
function sortEntries(entries: FileEntry[], sort: SortState): FileEntry[] {
  const dir = sort.asc ? 1 : -1;
  return [...entries].sort((a, b) => {
    if (a.is_dir !== b.is_dir) return a.is_dir ? -1 : 1;
    if (sort.field === "name" || a.is_dir) {
      return dir * a.name.toLowerCase().localeCompare(b.name.toLowerCase());
    }
    if (sort.field === "size") {
      return dir * ((a.size ?? 0) - (b.size ?? 0));
    }
    return dir * ((a.modified ?? 0) - (b.modified ?? 0));
  });
}

type DragPayload = DndPayload;

/** 左右两栏分隔比例记忆 key——和 AgentBrowser 共用同一个 key，两种双栏浏览器是
 * 同一种交互习惯，没必要分开记两份。*/
const SPLIT_STORAGE_KEY = "roc_desk-dual-pane-split-percent";
/** "远程在左/本地在右"还是反过来——纯 UI 布局偏好，和上面 SPLIT_STORAGE_KEY
 * 一样和 AgentBrowser 共用同一个 key（用户需求：习惯反过来的人可以调，记住
 * 最后一次的选择，不分连接/工作区，是一个全局的显示偏好）。 */
const SWAP_SIDES_KEY = "roc_desk-dual-pane-swap-sides";

interface SftpBrowserProps {
  profileId: string;
  /** 用来记住"这个工作区上次 SFTP 浏览时本地一侧停在哪个目录"（2026-08-18 需求，
   * 见下面 localStorage 那段注释），不是后端标识，纯前端记忆用的 key。*/
  workspaceId: string;
  /** 默认远程目录——工作区模式下就是当前工作区根目录（DESIGN.md §3.3，"默认的远程
   * 目录为当前工作区目录"），只在没有记忆或记忆的目录打不开时才会用到。*/
  initialRemotePath: string;
  /** 远程一侧要不要也按 `workspaceId` 记忆上次停留的目录（2026-08-25 需求，远程工具
   * 模式下"两边目录需要按远程会话记忆，下次进入时自动恢复"）——工作区模式不用这个，
   * 那边两侧目录改走 `initialLocalPath`/`onPathsChange`，由调用方存进后端工作区
   * 档案，不是这里的 localStorage 记忆。*/
  rememberRemotePath?: boolean;
  /** 本地一侧的初始目录——工作区模式传 `WorkspaceProfile.last_sftp_local_path`
   * （见 `onPathsChange`），不传就退回下面 localStorage 那套旧逻辑（远程工具模式的
   * SFTP 标签用这条路径，那边没有"工作区"概念，找不到地方存后端字段）。*/
  initialLocalPath?: string;
  /** 两侧目录任意一边变化都会调一次，工作区模式用它把最新路径写回
   * `workspace_update_last_sftp_paths`——下次重新打开这个工作区的 SFTP 直接定位
   * 到这里（2026-09-01 用户需求："保存到工作区信息里"，比 2026-08-18 那次改的
   * localStorage 方案更进一步：不只是记住，还要成为工作区身份的一部分）。 */
  onPathsChange?: (localPath: string, remotePath: string) => void;
  onOpenFile: (entry: FileEntry) => void;
}

function localPathStorageKey(workspaceId: string): string {
  return `roc_desk-sftp-local-path-${workspaceId}`;
}

function remotePathStorageKey(id: string): string {
  return `roc_desk-sftp-remote-path-${id}`;
}

/**
 * SFTP 双栏浏览器（DESIGN.md §3.3）：左远程/右本地，互相拖拽即下载/上传，
 * 也可以右键单条操作。远程侧默认停在当前工作区目录，本地侧默认停在这个工作区上次
 * SFTP 浏览时停留的目录——和大多数 SFTP 客户端（WinSCP/FileZilla）的双栏习惯一致。
 *
 * **本地目录按工作区记忆**（2026-08-18，用户原话："工作区对应的本地目录要记住，
 * 下次用户打开工作区的SFTP时保持这两个目录对应"）：之前本地侧每次都硬编码回到
 * 用户主目录，重新打开同一个工作区的 SFTP 面板时，上次手动导航到的本地目录（比如
 * 对应这个远程项目的本地检出目录）就丢了，得重新点几次。远程工具模式的 SFTP
 * 标签（没有工作区概念）仍然用这套 localStorage 记忆；工作区模式改成
 * `initialLocalPath`/`onPathsChange`，两侧目录都存进后端工作区档案（2026-09-01
 * 需求），不再只存本地这一侧、也不再只是前端本地存储。
 */
export const SftpBrowser: React.FC<SftpBrowserProps> = ({
  profileId,
  workspaceId,
  initialRemotePath,
  rememberRemotePath,
  initialLocalPath,
  onPathsChange,
  onOpenFile,
}) => {
  const remote = useSftpStore();
  const local = useLocalFsStore();
  const push = useToastStore((s) => s.push);
  const [menu, setMenu] = useState<{ x: number; y: number; side: Side; entry: FileEntry } | null>(null);
  const [transfer, setTransfer] = useState<{ requestId: string; count: number; path: string } | null>(null);
  const [showLog, setShowLog] = useState(false);
  const [editingSide, setEditingSide] = useState<Side | null>(null);
  const [editValue, setEditValue] = useState("");
  const [sort, setSort] = useState<Record<Side, SortState>>({ remote: DEFAULT_SORT, local: DEFAULT_SORT });

  const toggleSort = (side: Side, field: SortField) => {
    setSort((s) => {
      const current = s[side];
      const asc = current.field === field ? !current.asc : true;
      return { ...s, [side]: { field, asc } };
    });
  };

  // 左右两栏比例可拖拽调整（用户反馈：文件名长的时候单栏挤不下，需要能拉宽）——
  // 和 HomeShell.tsx/App.tsx 里侧边栏宽度拖拽是同一种模式，只是这里按容器宽度的
  // 百分比算，不是固定像素，因为面板本身会随窗口缩放。
  const splitContainerRef = useRef<HTMLDivElement>(null);
  const [splitPercent, setSplitPercent] = useState(() => {
    const stored = Number(localStorage.getItem(SPLIT_STORAGE_KEY));
    return stored >= 20 && stored <= 80 ? stored : 50;
  });
  const onSplitDragStart = () => {
    const rect = splitContainerRef.current?.getBoundingClientRect();
    if (!rect) return;
    let latest = splitPercent;
    const onMove = (ev: MouseEvent) => {
      latest = Math.max(20, Math.min(80, ((ev.clientX - rect.left) / rect.width) * 100));
      setSplitPercent(latest);
    };
    const onUp = () => {
      localStorage.setItem(SPLIT_STORAGE_KEY, String(latest));
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  };

  // "远程在左/本地在右" 还是反过来（用户需求：有的人习惯反过来，要能调，并且记住
  // 最后一次的选择，下次打开还是这样）——纯前端展示状态，不影响 remote/local 两个
  // store 本身，只是渲染时把两个面板塞进哪个 flex 槽位的问题。
  const [swapSides, setSwapSides] = useState(() => localStorage.getItem(SWAP_SIDES_KEY) === "1");
  const toggleSwapSides = () => {
    setSwapSides((s) => {
      const next = !s;
      localStorage.setItem(SWAP_SIDES_KEY, next ? "1" : "0");
      return next;
    });
  };

  useEffect(() => {
    (async () => {
      const rememberedRemote = rememberRemotePath ? localStorage.getItem(remotePathStorageKey(workspaceId)) : null;
      await remote.navigate(profileId, rememberedRemote ?? initialRemotePath);
      // 记住的目录可能已经被删除/改名，或者这台机器第一次用 SFTP 还没有记忆——
      // 两种情况都退回调用方给的默认目录，不留在一个报错状态里死等用户手动处理。
      if (rememberedRemote && useSftpStore.getState().error) {
        await remote.navigate(profileId, initialRemotePath);
      }
    })();

    (async () => {
      // `initialLocalPath`（工作区模式，来自后端）优先于 localStorage 记忆
      // （远程工具模式，没有这个 prop 时才会走到 localStorage）。
      const remembered = initialLocalPath ?? localStorage.getItem(localPathStorageKey(workspaceId));
      if (remembered) {
        await local.navigate(remembered);
      }
      // 记住的目录可能已经被删除/改名/挪盘符，或者这个工作区第一次用 SFTP 还没有
      // 记忆——两种情况都退回主目录，不留在一个报错状态里死等用户手动处理。
      if (!remembered || useLocalFsStore.getState().error) {
        const home = await localFsService.homeDir();
        await local.navigate(home);
      }
    })();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [profileId, initialRemotePath, initialLocalPath, workspaceId]);

  // 本地一侧每次导航成功都存一下，下次打开这个工作区的 SFTP 面板直接回到这里
  // （远程工具模式的 fallback；工作区模式还会额外走下面的 onPathsChange）。
  useEffect(() => {
    if (!local.cwd) return;
    localStorage.setItem(localPathStorageKey(workspaceId), local.cwd);
  }, [workspaceId, local.cwd]);

  // 工作区模式：两侧任意一边目录变化都往后端写一次（`workspace_update_last_sftp_paths`）
  // ——`onPathsChange` 不传就什么都不做（远程工具模式没有工作区可存）。
  useEffect(() => {
    if (!onPathsChange || !local.cwd || !remote.cwd) return;
    onPathsChange(local.cwd, remote.cwd);
  }, [onPathsChange, local.cwd, remote.cwd]);

  // 远程一侧同理（只在 rememberRemotePath 打开时才存，工作区模式不受影响）。
  useEffect(() => {
    if (!rememberRemotePath || !remote.cwd) return;
    localStorage.setItem(remotePathStorageKey(workspaceId), remote.cwd);
  }, [rememberRemotePath, workspaceId, remote.cwd]);

  const runTransfer = async (payload: DragPayload, targetSide: Side) => {
    if (payload.side === targetSide) return;
    const requestId = crypto.randomUUID();
    setTransfer({ requestId, count: 0, path: payload.path });
    // 粗粒度进度（按完成的文件数，不是字节百分比）：目录传输过程较长时至少能看出
    // "还在动"而不是卡死，见后端 emit_progress 的文档注释。
    const unlisten = await listen<SftpTransferProgressEvent>("sftp:transfer-progress", (event) => {
      if (event.payload.requestId !== requestId) return;
      setTransfer((s) => (s && s.requestId === requestId ? { ...s, count: s.count + 1, path: event.payload.path } : s));
    });
    try {
      if (payload.side === "remote") {
        await sftpService.downloadEntry(profileId, payload.path, payload.isDir, local.cwd, requestId);
        await local.navigate(local.cwd);
        push("success", `已下载 ${payload.name}`);
      } else {
        await sftpService.uploadEntry(profileId, payload.path, payload.isDir, remote.cwd, requestId);
        await remote.navigate(profileId, remote.cwd);
        push("success", `已上传 ${payload.name}`);
      }
    } catch (e) {
      if (isCancelledTransferError(e)) {
        push("info", "已停止传输");
        // 目录传输可能已经写了一部分——把两侧目录都刷新一下，让用户看到实际
        // 停在哪，而不是留着一份和磁盘状态对不上的旧列表。
        await Promise.all([remote.navigate(profileId, remote.cwd), local.navigate(local.cwd)]);
      } else {
        push("error", `传输失败：${formatError(e)}`);
      }
    } finally {
      unlisten();
      setTransfer(null);
    }
  };

  /** 从 Windows 资源管理器等外部窗口拖真实文件进远程面板——路径是操作系统给的
   * 绝对路径，不在本地面板当前列出的那批 entries 里，所以不能直接复用
   * runTransfer（它假设 payload 来自某一侧面板已经渲染出来的行），得先各自问一次
   * 是不是目录，再挨个传，避免并发写同一个 `transfer` 进度状态导致进度串台。 */
  const uploadExternalPaths = async (paths: string[]) => {
    for (const path of paths) {
      const name = path.replace(/[/\\]+$/, "").split(/[/\\]/).pop() ?? path;
      const requestId = crypto.randomUUID();
      setTransfer({ requestId, count: 0, path });
      const unlisten = await listen<SftpTransferProgressEvent>("sftp:transfer-progress", (event) => {
        if (event.payload.requestId !== requestId) return;
        setTransfer((s) => (s && s.requestId === requestId ? { ...s, count: s.count + 1, path: event.payload.path } : s));
      });
      let cancelled = false;
      try {
        const isDir = await localFsService.isDir(path);
        await sftpService.uploadEntry(profileId, path, isDir, remote.cwd, requestId);
        push("success", `已上传 ${name}`);
      } catch (e) {
        if (isCancelledTransferError(e)) {
          cancelled = true;
          push("info", "已停止传输");
        } else {
          push("error", `上传失败：${formatError(e)}`);
        }
      } finally {
        unlisten();
        setTransfer(null);
      }
      // 用户点了停止就不用再接着传后面拖进来的文件了——批量拖入时"停止"应该是
      // 停整批，不是只跳过当前这一个。
      if (cancelled) break;
    }
    await remote.navigate(profileId, remote.cwd);
  };

  const { remoteRef, localRef, dragOverSide, beginDrag } = useDualPaneDnd({
    onInternalTransfer: (payload, targetSide) => void runTransfer(payload, targetSide),
    onExternalUpload: (paths) => void uploadExternalPaths(paths),
  });

  // 右键"导入到本地搜索引擎"（2026-08-18 需求，用户原话："右键选中.LOG等文本类型的
  // 文件可以将他导入本地搜索引擎进行搜索"），SFTP 浏览器和 Explorer 各有一份是因为
  // 两边浏览的目录范围不一样（Explorer 限定在工作区内，SFTP 可以到处看），复用的是
  // 同一套 `logSearchService` 命令，不是重新实现。
  const importToLogSearch = async (path: string, side: Side) => {
    const current = useWorkspaceStore.getState().current;
    const hostName = current?.display_name ?? "unknown";
    try {
      const count = side === "remote" ? await logSearchService.importFile(profileId, path, hostName) : await logSearchService.importLocalFile(path, hostName);
      push("success", `已导入 ${count} 行到本地搜索引擎`);
    } catch (e) {
      push("error", `导入失败：${formatError(e)}`);
    }
  };

  const remoteMenuItems = (entry: FileEntry): ContextMenuItem[] => {
    const items: ContextMenuItem[] = [];
    if (!entry.is_dir) items.push({ label: "打开", onClick: () => onOpenFile(entry) });
    items.push(
      { label: "下载到本地", onClick: () => runTransfer({ side: "remote", path: entry.path, isDir: entry.is_dir, name: entry.name }, "local") },
    );
    if (!entry.is_dir) {
      items.push({ label: "导入到本地搜索引擎", onClick: () => importToLogSearch(entry.path, "remote") });
    }
    items.push(
      { label: "复制路径", onClick: () => navigator.clipboard.writeText(entry.path), separatorBefore: true },
      {
        label: "删除",
        danger: true,
        separatorBefore: true,
        onClick: async () => {
          try {
            await sftpService.delete(profileId, entry.path, entry.is_dir);
            await remote.navigate(profileId, remote.cwd);
          } catch (e) {
            push("error", `删除失败：${formatError(e)}`);
          }
        },
      },
    );
    return items;
  };

  const localMenuItems = (entry: FileEntry): ContextMenuItem[] => {
    const items: ContextMenuItem[] = [
      { label: "上传到远程", onClick: () => runTransfer({ side: "local", path: entry.path, isDir: entry.is_dir, name: entry.name }, "remote") },
    ];
    if (!entry.is_dir) {
      items.push({ label: "导入到本地搜索引擎", onClick: () => importToLogSearch(entry.path, "local") });
    }
    items.push({ label: "复制路径", onClick: () => navigator.clipboard.writeText(entry.path), separatorBefore: true });
    return items;
  };

  const renderPane = (
    side: Side,
    icon: React.ReactNode,
    label: string,
    state: { cwd: string; entries: FileEntry[]; loading: boolean; error: string | null; selectedPath: string | null },
    navigate: (path: string) => void,
    select: (path: string | null) => void,
    onRowDoubleClick: (entry: FileEntry) => void,
  ) => {
    const isUnix = side === "remote" || state.cwd.startsWith("/");
    const segments = state.cwd.split(/[/\\]/).filter(Boolean);
    const dirCount = state.entries.filter((e) => e.is_dir).length;
    const fileCount = state.entries.length - dirCount;
    const paneSort = sort[side];
    const sortedEntries = sortEntries(state.entries, paneSort);
    const sortIcon = (field: SortField) => {
      if (paneSort.field !== field) return null;
      const Icon = paneSort.asc ? ArrowUpNarrowWide : ArrowDownNarrowWide;
      return <Icon style={{ width: 12, height: 12 }} />;
    };
    const headerCell = (field: SortField, label: string) => (
      <span
        onClick={() => toggleSort(side, field)}
        style={{ display: "flex", alignItems: "center", gap: 4, cursor: "pointer", userSelect: "none" }}
        title="点击排序"
      >
        {label}
        {sortIcon(field)}
      </span>
    );

    return (
      <div
        ref={side === "remote" ? remoteRef : localRef}
        style={{ flex: 1, display: "flex", flexDirection: "column", minWidth: 0, minHeight: 0, background: dragOverSide === side ? "var(--accent-dim)" : undefined }}
      >
        <div className="sftp-toolbar">
          {icon}
          <span style={{ fontWeight: 600, marginRight: 4 }}>{label}</span>
          {editingSide === side ? (
            <input
              className="form-input"
              style={{ flex: 1, height: 22, fontSize: 12, fontFamily: "var(--font-mono)" }}
              autoFocus
              value={editValue}
              onFocus={(e) => e.currentTarget.select()}
              onChange={(e) => setEditValue(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  const value = editValue.trim();
                  if (value) navigate(value);
                  setEditingSide(null);
                } else if (e.key === "Escape") {
                  setEditingSide(null);
                }
              }}
              onBlur={() => setEditingSide(null)}
            />
          ) : (
            <div className="breadcrumb" style={{ overflow: "hidden" }}>
              {isUnix && (
                <span className="crumb" onClick={() => navigate("/")}>
                  /
                </span>
              )}
              {segments.map((seg, i) => {
                const path = isUnix ? "/" + segments.slice(0, i + 1).join("/") : segments.slice(0, i + 1).join("/") + "/";
                const isLast = i === segments.length - 1;
                return (
                  <span key={path}>
                    <span className="sep">›</span>{" "}
                    <span className={`crumb ${isLast ? "current" : ""}`} onClick={() => !isLast && navigate(path)}>
                      {seg}
                    </span>
                  </span>
                );
              })}
            </div>
          )}
          {/* 直接编辑路径（可以粘贴一个目录路径进来），不只是逐段点面包屑——
              2026-08-18 用户原话："目录要支持编辑或复制本地目录到编辑框"。 */}
          <button
            className="btn ghost sm"
            title="编辑路径"
            style={{ marginLeft: "auto" }}
            onClick={() => {
              setEditingSide(side);
              setEditValue(state.cwd);
            }}
          >
            <Pencil style={{ width: 12, height: 12 }} />
          </button>
          <button
            className="btn ghost sm"
            onClick={() => navigate(state.cwd.replace(/[/\\][^/\\]*[/\\]?$/, "") || (isUnix ? "/" : state.cwd))}
            disabled={state.cwd === "/" || segments.length === 0}
          >
            <ArrowUp style={{ width: 14, height: 14 }} />
          </button>
          <button className="btn ghost sm" title="刷新" onClick={() => navigate(state.cwd)}>
            <RotateCw style={{ width: 14, height: 14 }} />
          </button>
          {/* 传输历史只在远程一侧放一份入口——记录本身不分左右栏，放两份是多余的。 */}
          {side === "remote" && (
            <button className="btn ghost sm" title="传输日志" onClick={() => setShowLog(true)}>
              <History style={{ width: 14, height: 14 }} />
            </button>
          )}
          <button className="btn ghost sm" title={swapSides ? "恢复默认左右布局" : "远程/本地左右调换"} onClick={toggleSwapSides}>
            <ArrowLeftRight style={{ width: 14, height: 14 }} />
          </button>
        </div>

        {state.error && <div className="toast error" style={{ margin: 8 }}>{state.error}</div>}

        <div style={{ flex: 1, overflowY: "auto" }}>
          <div className="file-header">
            {headerCell("name", "名称")}
            {headerCell("size", "大小")}
            {headerCell("modified", "修改时间")}
          </div>
          {state.loading ? (
            <div style={{ padding: 16, fontSize: 12, color: "var(--text-secondary)" }}>加载中…</div>
          ) : state.entries.length === 0 ? (
            <div style={{ padding: 16, fontSize: 12, color: "var(--text-secondary)" }}>此目录是空的</div>
          ) : (
            sortedEntries.map((entry) => (
              <div
                key={entry.path}
                className={`file-row ${state.selectedPath === entry.path ? "selected" : ""}`}
                style={{ cursor: "grab" }}
                onMouseDown={beginDrag({ side, path: entry.path, isDir: entry.is_dir, name: entry.name })}
                onClick={() => select(entry.path)}
                onDoubleClick={() => onRowDoubleClick(entry)}
                onContextMenu={(e) => {
                  e.preventDefault();
                  select(entry.path);
                  setMenu({ x: e.clientX, y: e.clientY, side, entry });
                }}
              >
                <span style={{ display: "flex", alignItems: "center", gap: 6, overflow: "hidden" }}>
                  {entry.is_dir ? <Folder className="file-icon is-dir" /> : <FileIcon className="file-icon" />}
                  <span className="file-name">{entry.name}</span>
                </span>
                <span className="file-size">{entry.is_dir ? "—" : formatBytes(entry.size)}</span>
                <span className="file-time">{formatTime(entry.modified)}</span>
              </div>
            ))
          )}
        </div>

        <div className="sftp-footer">
          {/* 拖拽提示的方向要跟着 swapSides 走，不能假设远程永远在左——两边随时
              可能被用户调换过。 */}
          {dirCount} 目录, {fileCount} 文件 · 拖到{(side === "remote") === swapSides ? "左侧" : "右侧"}
          {side === "remote" ? "下载" : "上传"}
          {side === "remote" && " · 也可从资源管理器拖文件到此上传"}
        </div>
      </div>
    );
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      {transfer && (
        <div style={{ padding: "4px 12px", fontSize: 12, color: "var(--accent)", display: "flex", alignItems: "center", gap: 8 }}>
          <span style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", flex: 1 }}>
            传输中…{transfer.count > 0 ? ` 已完成 ${transfer.count} 项 · ${transfer.path}` : ""}
          </span>
          <button className="btn ghost sm" style={{ flexShrink: 0 }} onClick={() => transferService.cancel(transfer.requestId)}>
            停止
          </button>
        </div>
      )}
      {(() => {
        const remotePane = renderPane(
          "remote",
          <Server style={{ width: 14, height: 14 }} />,
          "远程",
          remote,
          (p) => remote.navigate(profileId, p),
          remote.select,
          (entry) => (entry.is_dir ? remote.navigate(profileId, entry.path) : onOpenFile(entry)),
        );
        const localPane = renderPane(
          "local",
          <Laptop style={{ width: 14, height: 14 }} />,
          "本地",
          local,
          (p) => local.navigate(p),
          local.select,
          (entry) => entry.is_dir && local.navigate(entry.path),
        );
        const [firstPane, secondPane] = swapSides ? [localPane, remotePane] : [remotePane, localPane];
        return (
          <div ref={splitContainerRef} style={{ flex: 1, display: "flex", overflow: "hidden", borderTop: "1px solid var(--border-default)" }}>
            <div style={{ width: `${splitPercent}%`, minWidth: 0, display: "flex", flexDirection: "column", overflow: "hidden" }}>
              {firstPane}
            </div>
            <div className="sftp-pane-resize-handle" onMouseDown={onSplitDragStart} />
            <div style={{ flex: 1, minWidth: 0, display: "flex", flexDirection: "column", overflow: "hidden" }}>
              {secondPane}
            </div>
          </div>
        );
      })()}

      {menu && (
        <ContextMenu x={menu.x} y={menu.y} items={menu.side === "remote" ? remoteMenuItems(menu.entry) : localMenuItems(menu.entry)} onClose={() => setMenu(null)} />
      )}
      {showLog && <TransferLogDialog onClose={() => setShowLog(false)} />}
    </div>
  );
};
