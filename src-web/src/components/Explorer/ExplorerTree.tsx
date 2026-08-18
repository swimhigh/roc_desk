import React, { useEffect, useState } from "react";
import { Folder, FolderOpen, File as FileIcon } from "lucide-react";
import { useExplorerStore } from "../../stores/explorerStore";
import { useEditorStore } from "../../stores/editorStore";
import { useTerminalStore } from "../../stores/terminalStore";
import { useWorkspaceStore } from "../../stores/workspaceStore";
import { fsService } from "../../services/fsService";
import { useToastStore } from "../shared/Toast";
import { ContextMenu, type ContextMenuItem } from "../shared/ContextMenu";
import { ConfirmDialog } from "../shared/ConfirmDialog";
import { formatError } from "../../utils/error";
import type { FileEntry } from "../../types/bindings";

/** 常见脚本类型 → 运行命令（右键"运行脚本"，DESIGN.md §3.2 终端面板复用）。
 * 只是个方便入口，不是通用的"运行配置"系统——命令写死，用户要跑别的解释器
 * 自己在终端里敲就行，不需要为这个做成可配置项。 */
function runCommandFor(path: string): string | null {
  const ext = path.split(".").pop()?.toLowerCase();
  switch (ext) {
    case "sh":
      return `bash "${path}"`;
    case "py":
      return `python3 "${path}"`;
    case "ps1":
      return `powershell -File "${path}"`;
    default:
      return null;
  }
}

interface ExplorerTreeProps {
  workspaceId: string;
  rootPath: string;
  onOpenFile: (path: string, opts?: { pin?: boolean }) => void;
}

function parentOf(path: string): string {
  const idx = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  return idx >= 0 ? path.slice(0, idx) : path;
}

/**
 * 工作区文件树（UI_DESIGN.md §3.3）：懒加载子目录，单击=预览态标签（复用同一个
 * 预览 Tab），双击=固定为常驻标签——`pin: true` 交给 onOpenFile 的调用方去做
 * "打开完成后再 pin" 的时序处理（openPreview 是异步的，pin 太早会因为 buffer
 * 还不存在而静默失效），这里不直接碰 editorStore，避免和 App.tsx 里的
 * openPreview 调用重复触发两次读盘/读远端。
 *
 * 右键菜单（参考 VS Code）：重命名/删除/复制路径/复制相对路径。
 */
export const ExplorerTree: React.FC<ExplorerTreeProps> = ({ workspaceId, rootPath, onOpenFile }) => {
  const { children, expanded, loadRoot, toggleDir, reloadDir, selectedPath, select, rootError } = useExplorerStore();
  const push = useToastStore((s) => s.push);
  const [menu, setMenu] = useState<{ x: number; y: number; entry: FileEntry | null } | null>(null);
  const [renamingPath, setRenamingPath] = useState<string | null>(null);
  const [renameValue, setRenameValue] = useState("");
  const [deleteTarget, setDeleteTarget] = useState<FileEntry | null>(null);
  const [clipboard, setClipboard] = useState<{ path: string; name: string; isDir: boolean; mode: "cut" | "copy" } | null>(null);

  useEffect(() => {
    loadRoot(workspaceId, rootPath);
  }, [workspaceId, rootPath, loadRoot]);

  const startRename = (entry: FileEntry) => {
    setRenamingPath(entry.path);
    setRenameValue(entry.name);
  };

  const commitRename = async (entry: FileEntry) => {
    const newName = renameValue.trim();
    setRenamingPath(null);
    if (!newName || newName === entry.name) return;
    const parent = parentOf(entry.path);
    const to = `${parent}/${newName}`;
    try {
      await fsService.rename(workspaceId, entry.path, to);
      await reloadDir(workspaceId, parent === rootPath || parent === "" ? rootPath : parent);
      if (parent !== rootPath) await reloadDir(workspaceId, parent);
    } catch (e) {
      push("error", `重命名失败：${formatError(e)}`);
    }
  };

  const confirmDelete = async () => {
    if (!deleteTarget) return;
    const entry = deleteTarget;
    setDeleteTarget(null);
    try {
      await fsService.deleteFile(workspaceId, entry.path, entry.is_dir);
      const parent = parentOf(entry.path);
      await reloadDir(workspaceId, parent);
      if (!entry.is_dir && useEditorStore.getState().buffers[entry.path]) {
        useEditorStore.getState().close(entry.path);
      }
      push("success", `已删除 ${entry.name}`);
    } catch (e) {
      push("error", `删除失败：${formatError(e)}`);
    }
  };

  const runScript = async (entry: FileEntry) => {
    const cmd = runCommandFor(entry.path);
    if (!cmd) return;
    const current = useWorkspaceStore.getState().current;
    if (!current) return;
    const term = useTerminalStore.getState();
    try {
      let targetId = term.activeId;
      if (!targetId || term.tabs.length === 0) {
        targetId =
          current.kind === "remote" && current.connection_id
            ? await term.openTerminal({ kind: "ssh", profileId: current.connection_id, cwd: current.root_path })
            : await term.openTerminal({ kind: "local", cwd: current.root_path });
      } else {
        term.setPanelOpen(true);
      }
      await term.writeToTerminal(targetId, `${cmd}\n`);
    } catch (e) {
      push("error", `运行脚本失败：${formatError(e)}`);
    }
  };

  /** 剪切=记下来源+等粘贴时挪过去（复用 rename）；复制=复用 `FileOps::copy`
   * （文件/目录都支持，目录会在后端递归复制）。 */
  const pasteInto = async (targetDir: string) => {
    if (!clipboard) return;
    const dest = `${targetDir}/${clipboard.name}`;
    try {
      if (clipboard.mode === "cut") {
        await fsService.rename(workspaceId, clipboard.path, dest);
        setClipboard(null);
      } else {
        await fsService.copy(workspaceId, clipboard.path, dest, clipboard.isDir);
      }
      await reloadDir(workspaceId, targetDir);
      const srcParent = parentOf(clipboard.path);
      if (clipboard.mode === "cut" && srcParent !== targetDir) await reloadDir(workspaceId, srcParent);
    } catch (e) {
      push("error", `粘贴失败：${formatError(e)}`);
    }
  };

  const menuItems = (entry: FileEntry): ContextMenuItem[] => {
    const relativePath = entry.path.startsWith(rootPath)
      ? entry.path.slice(rootPath.length).replace(/^[/\\]/, "")
      : entry.path;
    const items: ContextMenuItem[] = [];
    if (!entry.is_dir) {
      items.push({ label: "打开", onClick: () => onOpenFile(entry.path) });
    }
    if (runCommandFor(entry.path)) {
      items.push({ label: "运行脚本", onClick: () => runScript(entry) });
    }
    items.push(
      { label: "重命名", onClick: () => startRename(entry), separatorBefore: !entry.is_dir },
      { label: "删除", onClick: () => setDeleteTarget(entry), danger: true },
      {
        label: "剪切",
        onClick: () => setClipboard({ path: entry.path, name: entry.name, isDir: entry.is_dir, mode: "cut" }),
        separatorBefore: true,
      },
    );
    items.push({
      label: "复制",
      onClick: () => setClipboard({ path: entry.path, name: entry.name, isDir: entry.is_dir, mode: "copy" }),
    });
    if (clipboard) {
      items.push({ label: "粘贴", onClick: () => pasteInto(entry.is_dir ? entry.path : parentOf(entry.path)) });
    }
    items.push(
      { label: "复制路径", onClick: () => navigator.clipboard.writeText(entry.path), separatorBefore: true },
      { label: "复制相对路径", onClick: () => navigator.clipboard.writeText(relativePath) },
    );
    return items;
  };

  const renderNode = (entry: FileEntry, depth: number) => {
    const isExpanded = expanded.has(entry.path);
    const isRenaming = renamingPath === entry.path;
    return (
      <React.Fragment key={entry.path}>
        <div
          className={`tree-item ${selectedPath === entry.path ? "active" : ""}`}
          style={{ paddingLeft: 8 + depth * 16 }}
          onClick={() => {
            if (isRenaming) return;
            select(entry.path);
            if (entry.is_dir) {
              toggleDir(workspaceId, entry.path);
            } else {
              onOpenFile(entry.path);
            }
          }}
          onDoubleClick={() => {
            if (!entry.is_dir && !isRenaming) {
              onOpenFile(entry.path, { pin: true });
            }
          }}
          onContextMenu={(e) => {
            e.preventDefault();
            e.stopPropagation();
            select(entry.path);
            setMenu({ x: e.clientX, y: e.clientY, entry });
          }}
        >
          {entry.is_dir ? (
            isExpanded ? (
              <FolderOpen className="tree-icon is-dir" />
            ) : (
              <Folder className="tree-icon is-dir" />
            )
          ) : (
            <FileIcon className="tree-icon" />
          )}
          {isRenaming ? (
            <input
              className="tree-rename-input"
              autoFocus
              value={renameValue}
              onClick={(e) => e.stopPropagation()}
              onChange={(e) => setRenameValue(e.target.value)}
              onBlur={() => commitRename(entry)}
              onKeyDown={(e) => {
                if (e.key === "Enter") commitRename(entry);
                if (e.key === "Escape") setRenamingPath(null);
              }}
            />
          ) : (
            <span className="tree-name">{entry.name}</span>
          )}
        </div>
        {entry.is_dir && isExpanded && children[entry.path]?.map((child) => renderNode(child, depth + 1))}
      </React.Fragment>
    );
  };

  const rootEntries = children[rootPath] ?? [];

  return (
    <div
      className="project-tree"
      onContextMenu={(e) => {
        // 只在真正点到空白背景（没冒泡自某一行，那些行已经 stopPropagation 了）时
        // 才弹这个"根目录"菜单，粘贴目标是工作区根目录。
        if (e.target !== e.currentTarget || !clipboard) return;
        e.preventDefault();
        setMenu({ x: e.clientX, y: e.clientY, entry: null });
      }}
    >
      {rootError ? (
        <div style={{ padding: 16, fontSize: 12, color: "var(--text-secondary)" }}>
          <div style={{ color: "var(--danger, #e5484d)", marginBottom: 8 }}>加载失败：{rootError}</div>
          <button className="btn ghost sm" onClick={() => loadRoot(workspaceId, rootPath)}>
            重试
          </button>
        </div>
      ) : rootEntries.length === 0 ? (
        <div style={{ padding: 16, fontSize: 12, color: "var(--text-secondary)" }}>此文件夹是空的</div>
      ) : (
        rootEntries.map((entry) => renderNode(entry, 0))
      )}

      {menu && (
        <ContextMenu
          x={menu.x}
          y={menu.y}
          items={
            menu.entry
              ? menuItems(menu.entry)
              : clipboard
                ? [{ label: "粘贴", onClick: () => pasteInto(rootPath) }]
                : []
          }
          onClose={() => setMenu(null)}
        />
      )}

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
              <button className="btn danger-strong sm" onClick={confirmDelete}>
                删除
              </button>
            </>
          }
        >
          <p>
            确定要删除{deleteTarget.is_dir ? "目录" : "文件"} <strong>{deleteTarget.name}</strong> 吗？此操作不可撤销。
          </p>
        </ConfirmDialog>
      )}
    </div>
  );
};
