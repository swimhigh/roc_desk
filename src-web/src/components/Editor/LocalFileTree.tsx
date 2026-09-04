import React, { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { File as FileIcon, Folder, FolderOpen, HardDrive, Pin, RotateCw } from "lucide-react";
import { localFsService } from "../../services/localFsService";
import { useToastStore } from "../shared/Toast";
import { formatError } from "../../utils/error";
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
  onOpenFile: (path: string) => void;
}

/**
 * 编辑器模块左侧的本地文件树（UltraEdit 式，用户 2026-09-04 需求："编辑器桌面
 * 需要左边有个资源管理器"；"编辑器默认请展示我的电脑的所有盘，类似UE"）——
 * 默认就是"此电脑"下的全部盘符（参考 UltraEdit 左侧面板截图），不需要先手动
 * 选一个目录才有内容。纯浏览+打开，不做重命名/删除/新建这类文件管理操作
 * （那是资源管理器模块的职责，见 `LocalExplorer/LocalExplorerScreen.tsx`）。
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

  const renderNode = (entry: FileEntry, depth: number): React.ReactNode => {
    const isExpanded = expanded.has(entry.path);
    return (
      <React.Fragment key={entry.path}>
        <div
          className={`tree-item ${selected === entry.path ? "active" : ""}`}
          style={{ paddingLeft: 8 + depth * 16 }}
          onClick={() => {
            setSelected(entry.path);
            if (entry.is_dir) toggle(entry);
            else onOpenFile(entry.path);
          }}
        >
          {entry.is_dir ? (
            isExpanded ? <FolderOpen className="tree-icon is-dir" /> : <Folder className="tree-icon is-dir" />
          ) : (
            <FileIcon className="tree-icon" />
          )}
          <span className="tree-name">{entry.name}</span>
        </div>
        {entry.is_dir && isExpanded && children[entry.path]?.map((child) => renderNode(child, depth + 1))}
      </React.Fragment>
    );
  };

  const renderDrive = (drive: string) => {
    const path = drive; // 已经是 "C:/" 这种带斜杠形式
    const isExpanded = expanded.has(path);
    return (
      <React.Fragment key={path}>
        <div
          className={`tree-item ${selected === path ? "active" : ""}`}
          style={{ paddingLeft: 8 }}
          onClick={() => {
            setSelected(path);
            toggle({ name: drive, path, is_dir: true, size: null, modified: null });
          }}
        >
          <HardDrive className="tree-icon is-dir" />
          <span className="tree-name">{drive}</span>
        </div>
        {isExpanded && children[path]?.map((child) => renderNode(child, 1))}
      </React.Fragment>
    );
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
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
      <div style={{ flex: 1, overflow: "auto" }}>
        {root && (
          <>
            <div style={{ padding: "4px 8px 2px", fontSize: 10, color: "var(--text-secondary)", textTransform: "uppercase" }}>常用目录</div>
            {!children[root] ? (
              <div style={{ padding: "0 16px 8px", fontSize: 12, color: "var(--text-secondary)" }}>加载中…</div>
            ) : (
              <div
                className={`tree-item ${selected === root ? "active" : ""}`}
                style={{ paddingLeft: 8 }}
                onClick={() => {
                  setSelected(root);
                  toggle({ name: root, path: root, is_dir: true, size: null, modified: null });
                }}
              >
                {expanded.has(root) ? <FolderOpen className="tree-icon is-dir" /> : <Folder className="tree-icon is-dir" />}
                <span className="tree-name">{root.split(/[\\/]/).filter(Boolean).pop() ?? root}</span>
              </div>
            )}
            {expanded.has(root) && children[root]?.map((child) => renderNode(child, 1))}
            <div style={{ padding: "6px 8px 2px", fontSize: 10, color: "var(--text-secondary)", textTransform: "uppercase" }}>此电脑</div>
          </>
        )}
        {drives.length === 0 ? (
          <div style={{ padding: 16, fontSize: 12, color: "var(--text-secondary)" }}>加载中…</div>
        ) : (
          drives.map(renderDrive)
        )}
      </div>
    </div>
  );
};
