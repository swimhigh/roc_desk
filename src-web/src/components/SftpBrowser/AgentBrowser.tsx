import React, { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { Folder, File as FileIcon, ArrowUp, ArrowUpNarrowWide, ArrowDownNarrowWide, Laptop, Pencil, HardDrive, RotateCw, History, ArrowLeftRight } from "lucide-react";
import { TransferLogDialog } from "./TransferLogDialog";
import { useAgentBrowseStore } from "../../stores/agentBrowseStore";
import { useLocalFsStore } from "../../stores/localFsStore";
import { agentService } from "../../services/agentService";
import { localFsService } from "../../services/localFsService";
import { logSearchService } from "../../services/logSearchService";
import { useToastStore } from "../shared/Toast";
import { ContextMenu, type ContextMenuItem } from "../shared/ContextMenu";
import { formatError, isCancelledTransferError } from "../../utils/error";
import { transferService } from "../../services/transferService";
import { AGENT_ROOT, agentParentPath, isAgentRoot } from "../../utils/windowsPath";
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

/** 左右两栏分隔比例记忆 key——和 SftpBrowser 共用同一个 key，两种双栏浏览器是
 * 同一种交互习惯，没必要分开记两份。*/
const SPLIT_STORAGE_KEY = "roc_desk-dual-pane-split-percent";
/** "远程在左/本地在右"还是反过来，和 SftpBrowser 共用同一个 key。 */
const SWAP_SIDES_KEY = "roc_desk-dual-pane-swap-sides";

interface AgentBrowserProps {
  profileId: string;
  /** 本地一侧上次浏览停留的目录记忆 key（纯前端便利性状态，和 SftpBrowser 同款）。*/
  workspaceId: string;
  /** 默认远程目录——工作区模式下就是当前工作区根目录（和 SftpBrowser 的
   * `initialRemotePath` 同一个用途），只在没有记忆或记忆的目录打不开时才会用到。
   * 不传就退回 AGENT_ROOT（"此电脑"盘符列表），是会话树"文件传输"入口的默认行为。 */
  initialRemotePath?: string;
  /** 远程一侧要不要也按 `workspaceId` 记忆上次停留的目录——工作区模式不用这个，
   * 那边两侧目录改走 `initialLocalPath`/`onPathsChange`，存进后端工作区档案
   * （和 SftpBrowser 的同名 props 同一条设计）。 */
  rememberRemotePath?: boolean;
  /** 本地一侧的初始目录——工作区模式传 `WorkspaceProfile.last_sftp_local_path`，
   * 不传就退回下面 localStorage 的旧逻辑（会话树"文件传输"入口用）。 */
  initialLocalPath?: string;
  /** 两侧目录任意一边变化都会调一次，工作区模式用它把最新路径写回
   * `workspace_update_last_sftp_paths`。*/
  onPathsChange?: (localPath: string, remotePath: string) => void;
}

function localPathStorageKey(workspaceId: string): string {
  return `roc_desk-agent-browse-local-path-${workspaceId}`;
}
function remotePathStorageKey(id: string): string {
  return `roc_desk-agent-browse-remote-path-${id}`;
}

/**
 * Agent 版的双栏文件浏览器（AGENT_DESIGN.md §四.3）：左远程（Windows Agent 目标）/
 * 右本地，互相拖拽即下载/上传——和 `SftpBrowser.tsx` 是同一种交互模式，独立成一个
 * 组件是因为远程一侧协议完全不同（Agent 而不是 SFTP）、且远程路径是 Windows 语义
 * （盘符 + 反斜杠，没有单一根目录 "/"，第一层是虚拟的"此电脑"盘符列表），
 * 导航逻辑没法直接复用 SftpBrowser 那套按 POSIX 路径假设写的面包屑/上级目录计算。
 *
 * 目前不支持双击文件预览/编辑（`SftpFileViewer` 深度依赖 SFTP 专属的图片/PDF/
 * Word/Excel/JAR/EXE 预览命令，Agent 侧尚未实现这些）——这里只做浏览 + 传输 +
 * 删除/重命名，是 AGENT_DESIGN.md 里"点一下方便互传"这个具体诉求的最小完整实现，
 * 预览/编辑留作后续按需扩展。
 *
 * 拖拽传输见 `useDualPaneDnd`：面板内互拖和从 Windows 资源管理器拖真实文件进来
 * 是两套完全独立的机制（前者手搓鼠标事件，后者用 Tauri 的 `onDragDropEvent`），
 * 因为 Windows 上 Tauri 默认的 `dragDropEnabled: true` 会让浏览器原生 HTML5
 * 拖拽在前端完全不触发。
 */
export const AgentBrowser: React.FC<AgentBrowserProps> = ({
  profileId,
  workspaceId,
  initialRemotePath,
  rememberRemotePath,
  initialLocalPath,
  onPathsChange,
}) => {
  const remote = useAgentBrowseStore();
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

  // 左右两栏比例可拖拽调整，和 SftpBrowser.tsx 同一套实现（按容器宽度百分比算，
  // 不是固定像素，面板会随窗口缩放）。
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

  // "远程在左/本地在右" 还是反过来，和 SftpBrowser.tsx 同一套实现。
  const [swapSides, setSwapSides] = useState(() => localStorage.getItem(SWAP_SIDES_KEY) === "1");
  const toggleSwapSides = () => {
    setSwapSides((s) => {
      const next = !s;
      localStorage.setItem(SWAP_SIDES_KEY, next ? "1" : "0");
      return next;
    });
  };

  const defaultRemote = initialRemotePath ?? AGENT_ROOT;

  useEffect(() => {
    (async () => {
      const remembered = rememberRemotePath ? localStorage.getItem(remotePathStorageKey(workspaceId)) : null;
      await remote.navigate(profileId, remembered ?? defaultRemote);
      // 记住的目录可能已经被删除/改名，或者这台机器第一次浏览还没有记忆——
      // 两种情况都退回调用方给的默认目录，不留在一个报错状态里死等用户手动处理。
      if (remembered && useAgentBrowseStore.getState().error) {
        await remote.navigate(profileId, defaultRemote);
      }
    })();

    (async () => {
      const remembered = initialLocalPath ?? localStorage.getItem(localPathStorageKey(workspaceId));
      if (remembered) {
        await local.navigate(remembered);
      }
      if (!remembered || useLocalFsStore.getState().error) {
        const home = await localFsService.homeDir();
        await local.navigate(home);
      }
    })();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [profileId, workspaceId, defaultRemote, initialLocalPath]);

  useEffect(() => {
    if (!local.cwd) return;
    localStorage.setItem(localPathStorageKey(workspaceId), local.cwd);
  }, [workspaceId, local.cwd]);

  // 工作区模式：两侧任意一边目录变化都往后端写一次。
  useEffect(() => {
    if (!onPathsChange || !local.cwd || !remote.cwd) return;
    onPathsChange(local.cwd, remote.cwd);
  }, [onPathsChange, local.cwd, remote.cwd]);

  // 只在 rememberRemotePath 打开时才存，工作区模式（默认回到工作区根目录）不受影响。
  useEffect(() => {
    if (!rememberRemotePath || !remote.cwd) return;
    localStorage.setItem(remotePathStorageKey(workspaceId), remote.cwd);
  }, [rememberRemotePath, workspaceId, remote.cwd]);

  const runTransfer = async (payload: DragPayload, targetSide: Side) => {
    if (payload.side === targetSide) return;
    const requestId = crypto.randomUUID();
    setTransfer({ requestId, count: 0, path: payload.path });
    const unlisten = await listen<SftpTransferProgressEvent>("agent:transfer-progress", (event) => {
      if (event.payload.requestId !== requestId) return;
      setTransfer((s) => (s && s.requestId === requestId ? { ...s, count: s.count + 1, path: event.payload.path } : s));
    });
    try {
      if (payload.side === "remote") {
        await agentService.downloadEntry(profileId, payload.path, payload.isDir, local.cwd, requestId);
        await local.navigate(local.cwd);
        push("success", `已下载 ${payload.name}`);
      } else {
        await agentService.uploadEntry(profileId, payload.path, payload.isDir, remote.cwd, requestId);
        await remote.navigate(profileId, remote.cwd);
        push("success", `已上传 ${payload.name}`);
      }
    } catch (e) {
      if (isCancelledTransferError(e)) {
        push("info", "已停止传输");
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
      const unlisten = await listen<SftpTransferProgressEvent>("agent:transfer-progress", (event) => {
        if (event.payload.requestId !== requestId) return;
        setTransfer((s) => (s && s.requestId === requestId ? { ...s, count: s.count + 1, path: event.payload.path } : s));
      });
      let cancelled = false;
      try {
        const isDir = await localFsService.isDir(path);
        await agentService.uploadEntry(profileId, path, isDir, remote.cwd, requestId);
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
      if (cancelled) break;
    }
    await remote.navigate(profileId, remote.cwd);
  };

  const { remoteRef, localRef, dragOverSide, beginDrag } = useDualPaneDnd({
    onInternalTransfer: (payload, targetSide) => void runTransfer(payload, targetSide),
    onExternalUpload: (paths) => void uploadExternalPaths(paths),
  });

  const importToLogSearch = async (path: string) => {
    try {
      const count = await logSearchService.importLocalFile(path, "unknown");
      push("success", `已导入 ${count} 行到本地搜索引擎`);
    } catch (e) {
      push("error", `导入失败：${formatError(e)}`);
    }
  };

  const remoteMenuItems = (entry: FileEntry): ContextMenuItem[] => [
    { label: "下载到本地", onClick: () => runTransfer({ side: "remote", path: entry.path, isDir: entry.is_dir, name: entry.name }, "local") },
    { label: "复制路径", onClick: () => navigator.clipboard.writeText(entry.path), separatorBefore: true },
    {
      label: "删除",
      danger: true,
      separatorBefore: true,
      onClick: async () => {
        try {
          await agentService.delete(profileId, entry.path, entry.is_dir);
          await remote.navigate(profileId, remote.cwd);
        } catch (e) {
          push("error", `删除失败：${formatError(e)}`);
        }
      },
    },
  ];

  const localMenuItems = (entry: FileEntry): ContextMenuItem[] => {
    const items: ContextMenuItem[] = [
      { label: "上传到远程", onClick: () => runTransfer({ side: "local", path: entry.path, isDir: entry.is_dir, name: entry.name }, "remote") },
    ];
    if (!entry.is_dir) {
      items.push({ label: "导入到本地搜索引擎", onClick: () => importToLogSearch(entry.path) });
    }
    items.push({ label: "复制路径", onClick: () => navigator.clipboard.writeText(entry.path), separatorBefore: true });
    return items;
  };

  /** 远程（Agent/Windows）一侧的面包屑：第一段永远是"此电脑"（AGENT_ROOT 虚拟层级），
   * 之后按盘符/子目录逐段展开；和本地一侧的 POSIX/Windows 双形态面包屑分开实现，
   * 不强行共用一套路径切分逻辑。 */
  const remoteBreadcrumb = (cwd: string): { label: string; path: string }[] => {
    if (isAgentRoot(cwd)) return [{ label: "此电脑", path: AGENT_ROOT }];
    const segments = cwd.split("\\").filter(Boolean); // ["C:", "Users", "Bob"]
    const crumbs = [{ label: "此电脑", path: AGENT_ROOT }];
    for (let i = 0; i < segments.length; i++) {
      crumbs.push({ label: segments[i], path: segments.slice(0, i + 1).join("\\") + "\\" });
    }
    return crumbs;
  };

  const renderRemotePane = () => {
    const state = remote;
    const paneSort = sort.remote;
    const sortedEntries = sortEntries(state.entries, paneSort);
    const atRoot = isAgentRoot(state.cwd);
    const sortIcon = (field: SortField) => {
      if (paneSort.field !== field) return null;
      const Icon = paneSort.asc ? ArrowUpNarrowWide : ArrowDownNarrowWide;
      return <Icon style={{ width: 12, height: 12 }} />;
    };
    const headerCell = (field: SortField, label: string) => (
      <span onClick={() => toggleSort("remote", field)} style={{ display: "flex", alignItems: "center", gap: 4, cursor: "pointer", userSelect: "none" }} title="点击排序">
        {label}
        {sortIcon(field)}
      </span>
    );

    return (
      <div
        ref={remoteRef}
        style={{ flex: 1, display: "flex", flexDirection: "column", minWidth: 0, minHeight: 0, background: dragOverSide === "remote" ? "var(--accent-dim)" : undefined }}
      >
        <div className="sftp-toolbar">
          <HardDrive style={{ width: 14, height: 14 }} />
          <span style={{ fontWeight: 600, marginRight: 4 }}>远程（Agent）</span>
          {editingSide === "remote" ? (
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
                  if (value) remote.navigate(profileId, value);
                  setEditingSide(null);
                } else if (e.key === "Escape") {
                  setEditingSide(null);
                }
              }}
              onBlur={() => setEditingSide(null)}
            />
          ) : (
            <div className="breadcrumb" style={{ overflow: "hidden" }}>
              {remoteBreadcrumb(state.cwd).map((crumb, i, arr) => {
                const isLast = i === arr.length - 1;
                return (
                  <span key={crumb.path}>
                    {i > 0 && <span className="sep">›</span>}{" "}
                    <span className={`crumb ${isLast ? "current" : ""}`} onClick={() => !isLast && remote.navigate(profileId, crumb.path)}>
                      {crumb.label}
                    </span>
                  </span>
                );
              })}
            </div>
          )}
          <button className="btn ghost sm" title="编辑路径" style={{ marginLeft: "auto" }} onClick={() => { setEditingSide("remote"); setEditValue(state.cwd); }}>
            <Pencil style={{ width: 12, height: 12 }} />
          </button>
          <button className="btn ghost sm" onClick={() => remote.navigate(profileId, agentParentPath(state.cwd))} disabled={atRoot}>
            <ArrowUp style={{ width: 14, height: 14 }} />
          </button>
          <button className="btn ghost sm" title="刷新" onClick={() => remote.navigate(profileId, state.cwd)}>
            <RotateCw style={{ width: 14, height: 14 }} />
          </button>
          <button className="btn ghost sm" title="传输日志" onClick={() => setShowLog(true)}>
            <History style={{ width: 14, height: 14 }} />
          </button>
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
            <div style={{ padding: 16, fontSize: 12, color: "var(--text-secondary)" }}>{atRoot ? "没有找到磁盘" : "此目录是空的"}</div>
          ) : (
            sortedEntries.map((entry) => (
              <div
                key={entry.path}
                className={`file-row ${state.selectedPath === entry.path ? "selected" : ""}`}
                style={{ cursor: atRoot ? undefined : "grab" }}
                onMouseDown={atRoot ? undefined : beginDrag({ side: "remote", path: entry.path, isDir: entry.is_dir, name: entry.name })}
                onClick={() => remote.select(entry.path)}
                onDoubleClick={() => remote.navigate(profileId, entry.path)}
                onContextMenu={(e) => {
                  if (atRoot) return;
                  e.preventDefault();
                  remote.select(entry.path);
                  setMenu({ x: e.clientX, y: e.clientY, side: "remote", entry });
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

        <div className="sftp-footer">拖到{swapSides ? "左侧" : "右侧"}下载到本地 · 也可从资源管理器拖文件到此上传</div>
      </div>
    );
  };

  const renderLocalPane = () => {
    const state = local;
    const isUnix = state.cwd.startsWith("/");
    const segments = state.cwd.split(/[/\\]/).filter(Boolean);
    const paneSort = sort.local;
    const sortedEntries = sortEntries(state.entries, paneSort);
    const sortIcon = (field: SortField) => {
      if (paneSort.field !== field) return null;
      const Icon = paneSort.asc ? ArrowUpNarrowWide : ArrowDownNarrowWide;
      return <Icon style={{ width: 12, height: 12 }} />;
    };
    const headerCell = (field: SortField, label: string) => (
      <span onClick={() => toggleSort("local", field)} style={{ display: "flex", alignItems: "center", gap: 4, cursor: "pointer", userSelect: "none" }} title="点击排序">
        {label}
        {sortIcon(field)}
      </span>
    );

    return (
      <div
        ref={localRef}
        style={{ flex: 1, display: "flex", flexDirection: "column", minWidth: 0, minHeight: 0, background: dragOverSide === "local" ? "var(--accent-dim)" : undefined }}
      >
        <div className="sftp-toolbar">
          <Laptop style={{ width: 14, height: 14 }} />
          <span style={{ fontWeight: 600, marginRight: 4 }}>本地</span>
          {editingSide === "local" ? (
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
                  if (value) local.navigate(value);
                  setEditingSide(null);
                } else if (e.key === "Escape") {
                  setEditingSide(null);
                }
              }}
              onBlur={() => setEditingSide(null)}
            />
          ) : (
            <div className="breadcrumb" style={{ overflow: "hidden" }}>
              {isUnix && <span className="crumb" onClick={() => local.navigate("/")}>/</span>}
              {segments.map((seg, i) => {
                const path = isUnix ? "/" + segments.slice(0, i + 1).join("/") : segments.slice(0, i + 1).join("/") + "/";
                const isLast = i === segments.length - 1;
                return (
                  <span key={path}>
                    <span className="sep">›</span>{" "}
                    <span className={`crumb ${isLast ? "current" : ""}`} onClick={() => !isLast && local.navigate(path)}>
                      {seg}
                    </span>
                  </span>
                );
              })}
            </div>
          )}
          <button className="btn ghost sm" title="编辑路径" style={{ marginLeft: "auto" }} onClick={() => { setEditingSide("local"); setEditValue(state.cwd); }}>
            <Pencil style={{ width: 12, height: 12 }} />
          </button>
          <button
            className="btn ghost sm"
            onClick={() => local.navigate(state.cwd.replace(/[/\\][^/\\]*[/\\]?$/, "") || (isUnix ? "/" : state.cwd))}
            disabled={state.cwd === "/" || segments.length === 0}
          >
            <ArrowUp style={{ width: 14, height: 14 }} />
          </button>
          <button className="btn ghost sm" title="刷新" onClick={() => local.navigate(state.cwd)}>
            <RotateCw style={{ width: 14, height: 14 }} />
          </button>
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
                onMouseDown={beginDrag({ side: "local", path: entry.path, isDir: entry.is_dir, name: entry.name })}
                onClick={() => local.select(entry.path)}
                onDoubleClick={() => entry.is_dir && local.navigate(entry.path)}
                onContextMenu={(e) => {
                  e.preventDefault();
                  local.select(entry.path);
                  setMenu({ x: e.clientX, y: e.clientY, side: "local", entry });
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

        <div className="sftp-footer">拖到{swapSides ? "右侧" : "左侧"}上传到远程</div>
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
        const [firstPane, secondPane] = swapSides
          ? [renderLocalPane(), renderRemotePane()]
          : [renderRemotePane(), renderLocalPane()];
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
