import React, { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { Folder, File as FileIcon, ArrowUp, Laptop, Server } from "lucide-react";
import { useSftpStore } from "../../stores/sftpStore";
import { useLocalFsStore } from "../../stores/localFsStore";
import { sftpService } from "../../services/sftpService";
import { localFsService } from "../../services/localFsService";
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
  /** 默认远程目录，就是当前工作区根目录（DESIGN.md §3.3，用户要求"默认的远程目录为当前工作区目录"）。*/
  initialRemotePath: string;
  onOpenFile: (entry: FileEntry) => void;
}

/**
 * SFTP 双栏浏览器（DESIGN.md §3.3）：左远程/右本地，互相拖拽即下载/上传，
 * 也可以右键单条操作。远程侧默认停在当前工作区目录，本地侧默认停在用户主目录——
 * 和大多数 SFTP 客户端（WinSCP/FileZilla）的双栏习惯一致。
 */
export const SftpBrowser: React.FC<SftpBrowserProps> = ({ profileId, initialRemotePath, onOpenFile }) => {
  const remote = useSftpStore();
  const local = useLocalFsStore();
  const push = useToastStore((s) => s.push);
  const [menu, setMenu] = useState<{ x: number; y: number; side: Side; entry: FileEntry } | null>(null);
  const [dragOverSide, setDragOverSide] = useState<Side | null>(null);
  const [transfer, setTransfer] = useState<{ count: number; path: string } | null>(null);

  useEffect(() => {
    remote.navigate(profileId, initialRemotePath);
    localFsService.homeDir().then((home) => local.navigate(home));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [profileId, initialRemotePath]);

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

  const remoteMenuItems = (entry: FileEntry): ContextMenuItem[] => {
    const items: ContextMenuItem[] = [];
    if (!entry.is_dir) items.push({ label: "打开", onClick: () => onOpenFile(entry) });
    items.push(
      { label: "下载到本地", onClick: () => runTransfer({ side: "remote", path: entry.path, isDir: entry.is_dir, name: entry.name }, "local") },
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

  const localMenuItems = (entry: FileEntry): ContextMenuItem[] => [
    { label: "上传到远程", onClick: () => runTransfer({ side: "local", path: entry.path, isDir: entry.is_dir, name: entry.name }, "remote") },
    { label: "复制路径", onClick: () => navigator.clipboard.writeText(entry.path), separatorBefore: true },
  ];

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
          <button
            className="btn ghost sm"
            style={{ marginLeft: "auto" }}
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
