import React, { useEffect, useMemo, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { File as FileIcon, Folder, FolderOpen, HardDrive, Pin, RotateCw } from "lucide-react";
import { localFsService } from "../../services/localFsService";
import { localFileService } from "../../services/fsService";
import { useToastStore } from "../shared/Toast";
import { ContextMenu } from "../shared/ContextMenu";
import { ConfirmDialog } from "../shared/ConfirmDialog";
import { formatError } from "../../utils/error";
import { useFileTreeOperations, parentOf, flattenVisible, type FileTreeBackend } from "../../hooks/useFileTreeOperations";
import type { FileEntry } from "../../types/bindings";

function sortEntries(entries: FileEntry[]): FileEntry[] {
  return [...entries].sort((a, b) => {
    if (a.is_dir !== b.is_dir) return a.is_dir ? -1 : 1;
    return a.name.toLowerCase().localeCompare(b.name.toLowerCase());
  });
}

interface LocalFileTreeProps {
  root: string | null;
  onRootChange: (path: string) => void;
  onOpenFile: (path: string, opts?: { pin?: boolean }) => void;
}

/**
 * 编辑器模块左侧的本地文件树（UltraEdit 式，用户 2026-09-04 需求："编辑器桌面
 * 需要左边有个资源管理器"；"编辑器默认请展示我的电脑的所有盘，类似UE"）——
 * 默认就是"此电脑"下的全部盘符（参考 UltraEdit 左侧面板截图），不需要先手动
 * 选一个目录才有内容。
 *
 * 重命名/删除/新建/剪切复制粘贴/Shift-Ctrl 多选这套文件管理操作和工作区资源
 * 管理器（`Explorer/ExplorerTree.tsx`）共用同一份实现（`useFileTreeOperations`
 * hook，2026-09 用户反馈"和工作区一样，编辑器模式的本地文件树也需要能搜索/
 * 删除/复制"，并明确要求"真正合并成一份共用实现"）——这里只是套了本地文件
 * 系统（`localFsService`，不受工作区边界限制）的适配层。
 *
 * "打开文件夹"按钮不是替换掉盘符列表，而是在最上面钉一个快捷目录（`root`）——
 * 常用目录不需要每次都从盘符一层层展开进去。
 */
export const LocalFileTree: React.FC<LocalFileTreeProps> = ({ root, onRootChange, onOpenFile }) => {
  const push = useToastStore((s) => s.push);
  const [drives, setDrives] = useState<string[]>([]);
  const [children, setChildren] = useState<Record<string, FileEntry[]>>({});
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [selected, setSelected] = useState<string | null>(null);
  const [menu, setMenu] = useState<{ x: number; y: number; entry: FileEntry; depth: number } | null>(null);

  const loadChildren = async (path: string) => {
    try {
      const entries = await localFsService.listDir(path);
      setChildren((c) => ({ ...c, [path]: sortEntries(entries) }));
    } catch (e) {
      push("error", `读取目录失败：${formatError(e)}`);
    }
  };

  useEffect(() => {
    void localFsService.listDrives().then(setDrives).catch(() => {});
  }, []);

  useEffect(() => {
    if (root) void loadChildren(root);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [root]);

  const reveal = async (path: string) => {
    const norm = path.replace(/\\/g, "/");
    const parts = norm.split("/").filter(Boolean);
    let cur = /^[A-Za-z]:$/.test(parts[0] ?? "") ? `${parts.shift()!}/` : "/";
    setExpanded((prev) => new Set(prev).add(cur));
    await loadChildren(cur);
    // 只展开/加载"祖先目录"（最后一段是目标自己，不需要展开它）——原来这个
    // 循环连最后一段也当目录去 `loadChildren`，但从资源管理器双击跳转过来的
    // 目标通常是一个文件，对着文件路径调用列目录会报"目录名称无效"（Windows
    // os error 267，2026-09 用户反馈"双击文件跳转到编辑器模式报错"）。展开
    // 到父目录、把目标行显示出来就够了，选中状态由下面的 `setSelected` 负责。
    for (let i = 0; i < parts.length - 1; i++) {
      cur = `${cur.replace(/\/$/, "")}/${parts[i]}`;
      setExpanded((prev) => new Set(prev).add(cur));
      await loadChildren(cur);
    }
    setSelected(path);
  };

  useEffect(() => {
    const h = (e: Event) => {
      const p = (e as CustomEvent<{ path?: string }>).detail?.path;
      if (p) void reveal(p);
    };
    window.addEventListener("roc:reveal-standalone", h);
    return () => window.removeEventListener("roc:reveal-standalone", h);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const pickFolder = async () => {
    const selected = await open({ directory: true, multiple: false });
    if (!selected || Array.isArray(selected)) return;
    onRootChange(selected);
  };

  const toggle = (entry: FileEntry) => {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(entry.path)) {
        next.delete(entry.path);
      } else {
        next.add(entry.path);
        if (!children[entry.path]) void loadChildren(entry.path);
      }
      return next;
    });
  };

  // 顶层节点既有盘符（"C:/"）又有钉住的常用目录（`root`），两者都不在任何
  // `children[...]` 数组里——用一个虚拟的顶层条目数组喂给 `flattenVisible`/
  // Shift 范围多选，行为和工作区资源管理器（顶层就是 `children[rootPath]`）
  // 保持一致，不用为"没有单一根"这个本地文件树特有的情况另写一套多选逻辑。
  const topLevelEntries = useMemo<FileEntry[]>(() => {
    const rootEntry: FileEntry[] = root ? [{ name: root, path: root, is_dir: true, size: null, modified: null }] : [];
    const driveEntries: FileEntry[] = drives.map((d) => ({ name: d, path: d, is_dir: true, size: null, modified: null }));
    return [...rootEntry, ...driveEntries];
  }, [root, drives]);

  const backend: FileTreeBackend = useMemo(
    () => ({
      deleteFile: (path, isDir) => localFsService.deletePath(path, isDir),
      rename: (from, to) => localFsService.rename(from, to),
      copy: (from, to, isDir) => localFsService.copy(from, to, isDir),
      createDir: (path) => localFsService.createDir(path),
      writeFile: (path, content) => localFileService.writeFile(path, content, null).then(() => undefined),
    }),
    [],
  );
  const ops = useFileTreeOperations({
    backend,
    reloadDir: (path) => loadChildren(path),
    select: setSelected,
    getFlattenedVisible: () => flattenVisible(topLevelEntries, children, expanded),
    childrenOf: (parentPath) => children[parentPath],
    onOpenFile,
  });
  const {
    renamingPath,
    renameValue,
    setRenameValue,
    startRename,
    cancelRename,
    commitRename,
    creating,
    createValue,
    setCreateValue,
    cancelCreate,
    commitCreate,
    deleteTargets,
    requestDelete,
    cancelDelete,
    confirmDelete,
    clipboard,
    setClipboard,
    pasteInto,
    multiSelected,
    handleItemClick,
    handleContextMenuSelect,
    clearSelection,
    batchMenuItems,
  } = ops;

  /** 对着目录新建要先保证它已展开（还没懒加载过子目录列表时，`children[parentPath]`
   * 是 undefined，输入框无处挂载）。 */
  const startCreate = async (parentPath: string, depth: number, isDir: boolean) => {
    if (!expanded.has(parentPath)) {
      setExpanded((prev) => new Set(prev).add(parentPath));
      await loadChildren(parentPath);
    }
    ops.startCreate(parentPath, depth, isDir);
  };

  const openMenu = (e: React.MouseEvent, entry: FileEntry, depth: number) => {
    e.preventDefault();
    e.stopPropagation();
    handleContextMenuSelect(entry);
    setMenu({ x: e.clientX, y: e.clientY, entry, depth });
  };

  const singleMenuItems = (entry: FileEntry, depth: number) => {
    const items = [];
    if (!entry.is_dir) items.push({ label: "打开", onClick: () => onOpenFile(entry.path) });
    const createTargetDir = entry.is_dir ? entry.path : parentOf(entry.path);
    const createTargetDepth = entry.is_dir ? depth + 1 : depth;
    items.push(
      { label: "新建文件", onClick: () => startCreate(createTargetDir, createTargetDepth, false) },
      { label: "新建文件夹", onClick: () => startCreate(createTargetDir, createTargetDepth, true) },
      { label: "重命名", onClick: () => startRename(entry), separatorBefore: true },
      { label: "删除", onClick: () => requestDelete([entry]), danger: true },
      {
        label: "剪切",
        onClick: () => setClipboard({ items: [{ path: entry.path, name: entry.name, isDir: entry.is_dir }], mode: "cut" as const }),
        separatorBefore: true,
      },
      {
        label: "复制",
        onClick: () => setClipboard({ items: [{ path: entry.path, name: entry.name, isDir: entry.is_dir }], mode: "copy" as const }),
      },
    );
    if (clipboard) items.push({ label: "粘贴", onClick: () => pasteInto(entry.is_dir ? entry.path : parentOf(entry.path)) });
    items.push({ label: "复制路径", onClick: () => navigator.clipboard.writeText(entry.path), separatorBefore: true });
    return items;
  };

  const renderNode = (entry: FileEntry, depth: number): React.ReactNode => {
    const isExpanded = expanded.has(entry.path);
    const isRenaming = renamingPath === entry.path;
    return (
      <React.Fragment key={entry.path}>
        <div
          className={`tree-item ${selected === entry.path ? "active" : ""} ${multiSelected.has(entry.path) ? "multi-selected" : ""}`}
          style={{ paddingLeft: 8 + depth * 16 }}
          data-path={entry.path}
          onClick={(e) => {
            if (isRenaming) return;
            handleItemClick(e, entry, (target) => {
              if (target.is_dir) toggle(target);
              else onOpenFile(target.path);
            });
          }}
          onContextMenu={(e) => openMenu(e, entry, depth)}
        >
          {entry.is_dir ? (
            isExpanded ? <FolderOpen className="tree-icon is-dir" /> : <Folder className="tree-icon is-dir" />
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
                if (e.key === "Escape") cancelRename();
              }}
            />
          ) : (
            <span className="tree-name">{entry.name}</span>
          )}
        </div>
        {entry.is_dir && isExpanded && children[entry.path]?.map((child) => renderNode(child, depth + 1))}
        {entry.is_dir && isExpanded && creating?.parentPath === entry.path && renderCreateRow()}
      </React.Fragment>
    );
  };

  const renderCreateRow = () => {
    if (!creating) return null;
    return (
      <div className="tree-item" style={{ paddingLeft: 8 + creating.depth * 16 }}>
        {creating.isDir ? <Folder className="tree-icon is-dir" /> : <FileIcon className="tree-icon" />}
        <input
          className="tree-rename-input"
          autoFocus
          value={createValue}
          onClick={(e) => e.stopPropagation()}
          onChange={(e) => setCreateValue(e.target.value)}
          onBlur={commitCreate}
          onKeyDown={(e) => {
            if (e.key === "Enter") commitCreate();
            if (e.key === "Escape") cancelCreate();
          }}
        />
      </div>
    );
  };

  const renderDrive = (drive: string) => {
    const path = drive; // 已经是 "C:/" 这种带斜杠形式
    const entry: FileEntry = { name: drive, path, is_dir: true, size: null, modified: null };
    const isExpanded = expanded.has(path);
    const isRenaming = renamingPath === path;
    return (
      <React.Fragment key={path}>
        <div
          className={`tree-item ${selected === path ? "active" : ""} ${multiSelected.has(path) ? "multi-selected" : ""}`}
          style={{ paddingLeft: 8 }}
          data-path={path}
          onClick={(e) => {
            if (isRenaming) return;
            handleItemClick(e, entry, (target) => toggle(target));
          }}
          onContextMenu={(e) => openMenu(e, entry, 0)}
        >
          <HardDrive className="tree-icon is-dir" />
          <span className="tree-name">{drive}</span>
        </div>
        {isExpanded && children[path]?.map((child) => renderNode(child, 1))}
        {isExpanded && creating?.parentPath === path && renderCreateRow()}
      </React.Fragment>
    );
  };

  return (
    <div
      style={{ display: "flex", flexDirection: "column", height: "100%" }}
      onClick={(e) => {
        if (e.target === e.currentTarget) clearSelection();
      }}
    >
      <div style={{ display: "flex", alignItems: "center", gap: 4, height: 28, padding: "0 6px", borderBottom: "1px solid var(--border-subtle)", flexShrink: 0 }}>
        <button className="quick-tool-btn" title="钉一个常用目录到顶部" onClick={() => void pickFolder()} style={{ width: 22, height: 22 }}>
          <Pin style={{ width: 13, height: 13 }} />
        </button>
        <span style={{ fontSize: 11, color: "var(--text-secondary)", flex: 1 }}>此电脑</span>
        <button
          className="quick-tool-btn"
          title="刷新"
          onClick={() => {
            setChildren({});
            void localFsService.listDrives().then(setDrives).catch(() => {});
            if (root) void loadChildren(root);
          }}
          style={{ width: 22, height: 22 }}
        >
          <RotateCw style={{ width: 12, height: 12 }} />
        </button>
      </div>
      <div
        style={{ flex: 1, overflow: "auto" }}
        onContextMenu={(e) => {
          if (e.target !== e.currentTarget) return;
          e.preventDefault();
        }}
      >
        {root && (
          <>
            <div style={{ padding: "4px 8px 2px", fontSize: 10, color: "var(--text-secondary)", textTransform: "uppercase" }}>常用目录</div>
            {!children[root] ? (
              <div style={{ padding: "0 16px 8px", fontSize: 12, color: "var(--text-secondary)" }}>加载中…</div>
            ) : (
              <div
                className={`tree-item ${selected === root ? "active" : ""} ${multiSelected.has(root) ? "multi-selected" : ""}`}
                style={{ paddingLeft: 8 }}
                data-path={root}
                onClick={(e) => {
                  if (renamingPath === root) return;
                  handleItemClick(e, { name: root, path: root, is_dir: true, size: null, modified: null }, (target) => toggle(target));
                }}
                onContextMenu={(e) => openMenu(e, { name: root, path: root, is_dir: true, size: null, modified: null }, 0)}
              >
                {expanded.has(root) ? <FolderOpen className="tree-icon is-dir" /> : <Folder className="tree-icon is-dir" />}
                {renamingPath === root ? (
                  <input
                    className="tree-rename-input"
                    autoFocus
                    value={renameValue}
                    onClick={(e) => e.stopPropagation()}
                    onChange={(e) => setRenameValue(e.target.value)}
                    onBlur={() => commitRename({ name: root, path: root, is_dir: true, size: null, modified: null })}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") commitRename({ name: root, path: root, is_dir: true, size: null, modified: null });
                      if (e.key === "Escape") cancelRename();
                    }}
                  />
                ) : (
                  <span className="tree-name">{root.split(/[\\/]/).filter(Boolean).pop() ?? root}</span>
                )}
              </div>
            )}
            {expanded.has(root) && children[root]?.map((child) => renderNode(child, 1))}
            {expanded.has(root) && creating?.parentPath === root && renderCreateRow()}
            <div style={{ padding: "6px 8px 2px", fontSize: 10, color: "var(--text-secondary)", textTransform: "uppercase" }}>此电脑</div>
          </>
        )}
        {drives.length === 0 ? (
          <div style={{ padding: 16, fontSize: 12, color: "var(--text-secondary)" }}>加载中…</div>
        ) : (
          drives.map(renderDrive)
        )}
      </div>

      {menu && (
        <ContextMenu
          x={menu.x}
          y={menu.y}
          items={
            multiSelected.size > 1 && multiSelected.has(menu.entry.path)
              ? batchMenuItems(topLevelEntries.length ? flattenVisible(topLevelEntries, children, expanded).filter((e) => multiSelected.has(e.path)) : [])
              : singleMenuItems(menu.entry, menu.depth)
          }
          onClose={() => setMenu(null)}
        />
      )}

      {deleteTargets.length > 0 && (
        <ConfirmDialog
          open
          severity="danger"
          icon="🗑"
          title="确认删除"
          onDismiss={cancelDelete}
          actions={
            <>
              <button className="btn ghost sm" onClick={cancelDelete}>
                取消
              </button>
              <button className="btn danger-strong sm" onClick={confirmDelete}>
                删除
              </button>
            </>
          }
        >
          {deleteTargets.length === 1 ? (
            <p>
              确定要删除{deleteTargets[0].is_dir ? "目录" : "文件"} <strong>{deleteTargets[0].name}</strong> 吗？此操作不可撤销。
            </p>
          ) : (
            <p>
              确定要删除选中的 <strong>{deleteTargets.length}</strong> 项吗？此操作不可撤销。
            </p>
          )}
        </ConfirmDialog>
      )}
    </div>
  );
};
