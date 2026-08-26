import React, { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { Folder, File as FileIcon, ArrowUp, Laptop, Pencil, Server } from "lucide-react";
import { useSftpStore } from "../../stores/sftpStore";
import { useLocalFsStore } from "../../stores/localFsStore";
import { sftpService } from "../../services/sftpService";
import { localFsService } from "../../services/localFsService";
import { logSearchService } from "../../services/logSearchService";
import { useWorkspaceStore } from "../../stores/workspaceStore";
import { useToastStore } from "../shared/Toast";
import { ContextMenu, type ContextMenuItem } from "../shared/ContextMenu";
import { formatError } from "../../utils/error";
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

type Side = "remote" | "local";

interface DragPayload {
  side: Side;
  path: string;
  isDir: boolean;
  name: string;
}

const DRAG_MIME = "application/x-roc-desk-sftp-entry";

interface SftpBrowserProps {
  profileId: string;
  /** 用来记住"这个工作区上次 SFTP 浏览时本地一侧停在哪个目录"（2026-08-18 需求，
   * 见下面 localStorage 那段注释），不是后端标识，纯前端记忆用的 key。*/
  workspaceId: string;
  /** 默认远程目录——工作区模式下就是当前工作区根目录（DESIGN.md §3.3，"默认的远程
   * 目录为当前工作区目录"），只在没有记忆或记忆的目录打不开时才会用到。*/
  initialRemotePath: string;
  /** 远程一侧要不要也按 `workspaceId` 记忆上次停留的目录（2026-08-25 需求，远程工具
   * 模式下"两边目录需要按远程会话记忆，下次进入时自动恢复"）——工作区模式故意不开
   * 这个，那边的"默认回到工作区根目录"是有意为之的设计，不能被记忆覆盖掉。*/
  rememberRemotePath?: boolean;
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
 * 对应这个远程项目的本地检出目录）就丢了，得重新点几次。存 `localStorage`（key 带
 * `workspaceId`），不是后端 SQLite——这是纯前端会话便利性状态，不是需要备份/跨机器
 * 同步的业务数据，和侧边栏宽度记忆是同一类东西，犯不上为它加一次数据库迁移。
 */
export const SftpBrowser: React.FC<SftpBrowserProps> = ({
  profileId,
  workspaceId,
  initialRemotePath,
  rememberRemotePath,
  onOpenFile,
}) => {
  const remote = useSftpStore();
  const local = useLocalFsStore();
  const push = useToastStore((s) => s.push);
  const [menu, setMenu] = useState<{ x: number; y: number; side: Side; entry: FileEntry } | null>(null);
  const [dragOverSide, setDragOverSide] = useState<Side | null>(null);
  const [transfer, setTransfer] = useState<{ count: number; path: string } | null>(null);
  const [editingSide, setEditingSide] = useState<Side | null>(null);
  const [editValue, setEditValue] = useState("");

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
      const remembered = localStorage.getItem(localPathStorageKey(workspaceId));
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
  }, [profileId, initialRemotePath, workspaceId]);

  // 本地一侧每次导航成功都存一下，下次打开这个工作区的 SFTP 面板直接回到这里。
  useEffect(() => {
    if (!local.cwd) return;
    localStorage.setItem(localPathStorageKey(workspaceId), local.cwd);
  }, [workspaceId, local.cwd]);

  // 远程一侧同理（只在 rememberRemotePath 打开时才存，工作区模式不受影响）。
  useEffect(() => {
    if (!rememberRemotePath || !remote.cwd) return;
    localStorage.setItem(remotePathStorageKey(workspaceId), remote.cwd);
  }, [rememberRemotePath, workspaceId, remote.cwd]);

  const runTransfer = async (payload: DragPayload, targetSide: Side) => {
    if (payload.side === targetSide) return;
    const requestId = crypto.randomUUID();
    setTransfer({ count: 0, path: payload.path });
    // 粗粒度进度（按完成的文件数，不是字节百分比）：目录传输过程较长时至少能看出
    // "还在动"而不是卡死，见后端 emit_progress 的文档注释。
    const unlisten = await listen<SftpTransferProgressEvent>("sftp:transfer-progress", (event) => {
      if (event.payload.requestId !== requestId) return;
      setTransfer((s) => ({ count: (s?.count ?? 0) + 1, path: event.payload.path }));
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
      push("error", `传输失败：${formatError(e)}`);
    } finally {
      unlisten();
      setTransfer(null);
    }
  };

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

    return (
      <div
        style={{ flex: 1, display: "flex", flexDirection: "column", minWidth: 0, background: dragOverSide === side ? "var(--accent-dim)" : undefined }}
        onDragOver={(e) => {
          e.preventDefault();
          setDragOverSide(side);
        }}
        onDragLeave={() => setDragOverSide((s) => (s === side ? null : s))}
        onDrop={(e) => {
          e.preventDefault();
          setDragOverSide(null);
          const raw = e.dataTransfer.getData(DRAG_MIME);
          if (!raw) return;
          runTransfer(JSON.parse(raw) as DragPayload, side);
        }}
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
        </div>

        {state.error && <div className="toast error" style={{ margin: 8 }}>{state.error}</div>}

        <div style={{ flex: 1, overflowY: "auto" }}>
          <div className="file-header">
            <span>名称</span>
            <span>大小</span>
            <span>修改时间</span>
          </div>
          {state.loading ? (
            <div style={{ padding: 16, fontSize: 12, color: "var(--text-secondary)" }}>加载中…</div>
          ) : state.entries.length === 0 ? (
            <div style={{ padding: 16, fontSize: 12, color: "var(--text-secondary)" }}>此目录是空的</div>
          ) : (
            state.entries.map((entry) => (
              <div
                key={entry.path}
                className={`file-row ${state.selectedPath === entry.path ? "selected" : ""}`}
                draggable
                onDragStart={(e) => {
                  const payload: DragPayload = { side, path: entry.path, isDir: entry.is_dir, name: entry.name };
                  e.dataTransfer.setData(DRAG_MIME, JSON.stringify(payload));
                  e.dataTransfer.effectAllowed = "copy";
                }}
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
          {dirCount} 目录, {fileCount} 文件 · 拖到{side === "remote" ? "右侧下载" : "左侧上传"}
        </div>
      </div>
    );
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      {transfer && (
        <div style={{ padding: "4px 12px", fontSize: 12, color: "var(--accent)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
          传输中…{transfer.count > 0 ? ` 已完成 ${transfer.count} 项 · ${transfer.path}` : ""}
        </div>
      )}
      <div style={{ flex: 1, display: "flex", overflow: "hidden", borderTop: "1px solid var(--border-default)" }}>
        {renderPane(
          "remote",
          <Server style={{ width: 14, height: 14 }} />,
          "远程",
          remote,
          (p) => remote.navigate(profileId, p),
          remote.select,
          (entry) => (entry.is_dir ? remote.navigate(profileId, entry.path) : onOpenFile(entry)),
        )}
        <div style={{ width: 1, background: "var(--border-default)", flexShrink: 0 }} />
        {renderPane(
          "local",
          <Laptop style={{ width: 14, height: 14 }} />,
          "本地",
          local,
          (p) => local.navigate(p),
          local.select,
          (entry) => entry.is_dir && local.navigate(entry.path),
        )}
      </div>

      {menu && (
        <ContextMenu x={menu.x} y={menu.y} items={menu.side === "remote" ? remoteMenuItems(menu.entry) : localMenuItems(menu.entry)} onClose={() => setMenu(null)} />
      )}
    </div>
  );
};
