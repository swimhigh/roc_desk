import React, { useEffect, useRef, useState } from "react";
import { Folder, FolderOpen, File as FileIcon } from "lucide-react";
import { useExplorerStore } from "../../stores/explorerStore";
import { useEditorStore } from "../../stores/editorStore";
import { useTerminalStore } from "../../stores/terminalStore";
import { useWorkspaceStore } from "../../stores/workspaceStore";
import { fsService } from "../../services/fsService";
import { logSearchService } from "../../services/logSearchService";
import { useToastStore } from "../shared/Toast";
import { ContextMenu, type ContextMenuItem } from "../shared/ContextMenu";
import { ConfirmDialog } from "../shared/ConfirmDialog";
import { formatError } from "../../utils/error";
import { classifyPreview } from "../../utils/previewFile";
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
  /** 右键目录 →"在此文件夹中搜索"（2026-08-18 需求），把搜索范围收窄到这个子目录。 */
  onSearchInFolder: (path: string, relativePath: string) => void;
  /** 右键"与所选文件比较"：打开一个对比标签，并把编辑器区域切到前台
   * （App.tsx 里对应把 activeView 切回 "editor"）。 */
  onCompare: (leftPath: string, rightPath: string) => void;
}

function parentOf(path: string): string {
  const idx = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  return idx >= 0 ? path.slice(0, idx) : path;
}

function baseName(path: string): string {
  return path.split(/[/\\]/).filter(Boolean).pop() ?? path;
}

/**
 * 工作区文件树（UI_DESIGN.md §3.3）：懒加载子目录，单击=预览态标签（复用同一个
 * 预览 Tab），双击=固定为常驻标签——`pin: true` 交给 onOpenFile 的调用方去做
 * "打开完成后再 pin" 的时序处理（openPreview 是异步的，pin 太早会因为 buffer
 * 还不存在而静默失效），这里不直接碰 editorStore，避免和 App.tsx 里的
 * openPreview 调用重复触发两次读盘/读远端。
 *
 * 右键菜单（参考 VS Code）：重命名/删除/复制路径/复制相对路径；目录额外有"在此
 * 文件夹中搜索"（2026-08-18 需求），把左侧搜索面板的范围收窄到这个子目录；文件
 * 额外有"导入到本地搜索引擎"（同日需求），一步把这个文件送进日志搜索模块的 FTS5
 * 索引，不用先切到日志搜索面板再找一遍文件。
 */
export const ExplorerTree: React.FC<ExplorerTreeProps> = ({ workspaceId, rootPath, onOpenFile, onSearchInFolder, onCompare }) => {
  const { children, expanded, loadRoot, toggleDir, reloadDir, refreshAll, selectedPath, select, compareSource, setCompareSource, rootError } =
    useExplorerStore();
  const push = useToastStore((s) => s.push);
  const [menu, setMenu] = useState<{ x: number; y: number; entry: FileEntry | null; depth: number } | null>(null);
  const [renamingPath, setRenamingPath] = useState<string | null>(null);
  const [renameValue, setRenameValue] = useState("");
  // "新建文件"/"新建文件夹"（2026-09-03 需求）：和重命名同一种"内联输入框"交互
  // （参考 VS Code），但没有现成的 FileEntry 可以套，所以单独一份状态——
  // `parentPath` 决定新建在哪个目录下、`depth` 只用来算缩进对齐，`isDir` 决定
  // 提交时走 `createDir` 还是 `writeFile`。
  const [creating, setCreating] = useState<{ parentPath: string; depth: number; isDir: boolean } | null>(null);
  const [createValue, setCreateValue] = useState("");
  const createRowRef = useRef<HTMLDivElement>(null);
  const treeContainerRef = useRef<HTMLDivElement>(null);
  const [deleteTarget, setDeleteTarget] = useState<FileEntry | null>(null);
  const [clipboard, setClipboard] = useState<{ path: string; name: string; isDir: boolean; mode: "cut" | "copy" } | null>(null);

  useEffect(() => {
    loadRoot(workspaceId, rootPath);
  }, [workspaceId, rootPath, loadRoot]);

  // 新建文件/文件夹的输入框可能出现在当前滚动区域之外（比如在一个很长的列表
  // 末尾新建），不滚过去用户根本看不到刚弹出来的输入框在哪（2026-09-03 用户
  // 反馈）。`creating` 一旦非空就意味着输入框刚挂载，"nearest" 是刚好够看见就
  // 停，不会像 "center" 那样把已经在视野内的情况也强制重新滚动一下。
  useEffect(() => {
    if (creating) createRowRef.current?.scrollIntoView({ block: "nearest" });
  }, [creating]);

  // 新建完成后，输入框所在的位置（列表末尾）和新文件实际排好序后的位置（按目录
  // 优先、字母序）往往不是同一个地方——列表很长、当前视口只覆盖其中一段时，
  // 新文件排到了视口外，用户看不到"新建成功了"（2026-09-03 用户反馈：文件树
  // 内容多、出现滚动条时看不到新建的文件；内容少不需要滚动时是正常的，说明
  // 新建本身是成功的，只是没滚过去）。`commitCreate`/`commitRename` 都会调
  // `select(path)`，这里统一在选中项变化时把对应行滚进视口，不用在每个改
  // `selectedPath` 的地方各自处理一遍滚动。
  useEffect(() => {
    if (!selectedPath) return;
    const el = treeContainerRef.current?.querySelector<HTMLElement>(`[data-path="${CSS.escape(selectedPath)}"]`);
    el?.scrollIntoView({ block: "nearest" });
  }, [selectedPath]);

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

  /** "新建文件"/"新建文件夹"（2026-09-03 需求，参考 VS Code）：先保证目标目录已展开
   * （懒加载的子目录列表还没拉过时，`children[parentPath]` 是 undefined，输入框无处
   * 挂载），再进入内联输入态。`toggleDir` 是"切换"语义，只有还没展开时才调用，
   * 不然对着已展开的目录新建文件反而会把它折叠起来。 */
  const startCreate = async (parentPath: string, depth: number, isDir: boolean) => {
    if (!expanded.has(parentPath)) {
      await toggleDir(workspaceId, parentPath);
    }
    setCreateValue("");
    setCreating({ parentPath, depth, isDir });
  };

  const cancelCreate = () => setCreating(null);

  const commitCreate = async () => {
    if (!creating) return;
    const { parentPath, isDir } = creating;
    const name = createValue.trim();
    setCreating(null);
    if (!name) return;
    // 客户端先查一遍重名——`fs_write_file` 在 `expected_mtime: null` 时是"新建
    // 或覆盖"语义（保存冲突检测的既有约定），`create_dir` 对已存在目录也不报错，
    // 两者都不会自然地给出"已存在"提示，这里主动拦一下，避免用户以为在新建、
    // 实际上悄悄覆盖/合并了一个同名文件/目录。
    if ((children[parentPath] ?? []).some((e) => e.name === name)) {
      push("error", `${name} 已存在`);
      return;
    }
    const path = `${parentPath}/${name}`;
    try {
      if (isDir) {
        await fsService.createDir(workspaceId, path);
      } else {
        await fsService.writeFile(workspaceId, path, "", null);
      }
      await reloadDir(workspaceId, parentPath);
      select(path);
      if (!isDir) onOpenFile(path, { pin: true });
    } catch (e) {
      push("error", `新建${isDir ? "文件夹" : "文件"}失败：${formatError(e)}`);
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

  /** 右键"导入到本地搜索引擎"（2026-08-18 需求，用户原话："右键选中.LOG等文本类型的
   * 文件可以将他导入本地搜索引擎进行搜索"）——复用日志搜索模块已有的导入命令（`log_import_
   * local_file`/`log_import_file`），之前只能从"日志搜索"面板里点"导入本地/远程文件"再
   * 弹文件选择框去找，这里是从 Explorer 直接对着已经在看的文件走这条路径，少绕一圈。
   * 不限制文件扩展名——导入命令本身就是按行读文本进 FTS5 索引，不是"专属 .log"的能力，
   * 限制成只对 .log 显示反而人为缩小了这个入口的适用范围。 */
  const importToLogSearch = async (entry: FileEntry) => {
    const current = useWorkspaceStore.getState().current;
    if (!current) return;
    try {
      const count =
        current.kind === "remote" && current.connection_id
          ? await logSearchService.importFile(current.connection_id, entry.path, current.display_name)
          : await logSearchService.importLocalFile(entry.path, current.display_name);
      push("success", `已导入 ${count} 行到本地搜索引擎`);
    } catch (e) {
      push("error", `导入失败：${formatError(e)}`);
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

  const menuItems = (entry: FileEntry, depth: number): ContextMenuItem[] => {
    const relativePath = entry.path.startsWith(rootPath)
      ? entry.path.slice(rootPath.length).replace(/^[/\\]/, "")
      : entry.path;
    const items: ContextMenuItem[] = [];
    if (!entry.is_dir) {
      items.push({ label: "打开", onClick: () => onOpenFile(entry.path) });
    } else {
      items.push(
        { label: "在此文件夹中搜索", onClick: () => onSearchInFolder(entry.path, relativePath) },
        { label: "刷新", onClick: () => reloadDir(workspaceId, entry.path) },
      );
    }
    // 对着目录新建=新建在这个目录里（深一层）；对着文件新建=新建成它的同级兄弟
    // （还是当前这层）——参考 VS Code 右键任意条目都能新建，不需要非得点中目录。
    const createTargetDir = entry.is_dir ? entry.path : parentOf(entry.path);
    const createTargetDepth = entry.is_dir ? depth + 1 : depth;
    items.push(
      { label: "新建文件", onClick: () => startCreate(createTargetDir, createTargetDepth, false) },
      { label: "新建文件夹", onClick: () => startCreate(createTargetDir, createTargetDepth, true) },
    );
    if (runCommandFor(entry.path)) {
      items.push({ label: "运行脚本", onClick: () => runScript(entry) });
    }
    if (!entry.is_dir) {
      items.push({ label: "导入到本地搜索引擎", onClick: () => importToLogSearch(entry) });
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
    // 对比（参考 VS Code 的 "Select for Compare" / "Compare with Selected"）：只对文本类
    // 文件开放——图片/PDF/可执行文件等走 Monaco 对比没有意义，classifyPreview 已经有
    // 现成的分类可以复用。
    if (!entry.is_dir && classifyPreview(entry.path) === "text") {
      items.push({ label: "选择进行比较", onClick: () => setCompareSource(entry.path), separatorBefore: true });
      if (compareSource && compareSource !== entry.path && classifyPreview(compareSource) === "text") {
        items.push({ label: `与"${baseName(compareSource)}"比较`, onClick: () => onCompare(compareSource, entry.path) });
      }
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
          data-path={entry.path}
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
            setMenu({ x: e.clientX, y: e.clientY, entry, depth });
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
        {entry.is_dir && isExpanded && creating?.parentPath === entry.path && renderCreateRow()}
      </React.Fragment>
    );
  };

  /** "新建文件"/"新建文件夹"的内联输入行——和 `isRenaming` 那个输入框共用同一套
   * `.tree-rename-input` 样式，视觉上是同一种交互，只是没有对应的 `FileEntry`
   * 可以复用整个 `tree-item` 渲染分支，单独写一份。 */
  const renderCreateRow = () => {
    if (!creating) return null;
    return (
      <div ref={createRowRef} className="tree-item" style={{ paddingLeft: 8 + creating.depth * 16 }}>
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

  const rootEntries = children[rootPath] ?? [];

  return (
    <div
      ref={treeContainerRef}
      className="project-tree"
      onContextMenu={(e) => {
        // 只在真正点到空白背景（没冒泡自某一行，那些行已经 stopPropagation 了）时
        // 才处理——但不管有没有剪贴板内容都要先 preventDefault，不然空剪贴板时
        // 直接 return 会漏掉这一步，让 WebView2 自己的原生右键菜单（含"刷新"，
        // 效果等于 F5 重载整个应用）露出来（2026-09-01 真实 bug）。这里始终弹菜单
        // （至少有"刷新"，2026-09-01 用户反馈目录树完全没有刷新入口），有剪贴板
        // 内容时再加一条"粘贴"，目标是工作区根目录。
        if (e.target !== e.currentTarget) return;
        e.preventDefault();
        setMenu({ x: e.clientX, y: e.clientY, entry: null, depth: 0 });
      }}
    >
      {rootError ? (
        <div style={{ padding: 16, fontSize: 12, color: "var(--text-secondary)" }}>
          <div style={{ color: "var(--danger, #e5484d)", marginBottom: 8 }}>加载失败：{rootError}</div>
          <button className="btn ghost sm" onClick={() => loadRoot(workspaceId, rootPath)}>
            重试
          </button>
        </div>
      ) : rootEntries.length === 0 && !creating ? (
        <div style={{ padding: 16, fontSize: 12, color: "var(--text-secondary)" }}>此文件夹是空的</div>
      ) : (
        rootEntries.map((entry) => renderNode(entry, 0))
      )}
      {creating?.parentPath === rootPath && renderCreateRow()}

      {menu && (
        <ContextMenu
          x={menu.x}
          y={menu.y}
          items={
            menu.entry
              ? menuItems(menu.entry, menu.depth)
              : [
                  { label: "新建文件", onClick: () => startCreate(rootPath, 0, false) },
                  { label: "新建文件夹", onClick: () => startCreate(rootPath, 0, true) },
                  { label: "刷新", onClick: () => refreshAll(workspaceId, rootPath), separatorBefore: true },
                  ...(clipboard ? [{ label: "粘贴", onClick: () => pasteInto(rootPath), separatorBefore: true }] : []),
                ]
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
