import React, { useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
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
import { useFileTreeOperations, parentOf, baseName, flattenVisible, type FileTreeBackend } from "../../hooks/useFileTreeOperations";
import type { FileEntry } from "../../types/bindings";

/** 常见脚本类型 → 运行命令（右键"运行脚本"，DESIGN.md §3.2 终端面板复用）。
 * 只是个方便入口，不是通用的"运行配置"系统——命令写死，用户要跑别的解释器
 * 自己在终端里敲就行，不需要为这个做成可配置项。 */
function runCommandFor(path: string, remote = false): string | null {
  const ext = path.split(".").pop()?.toLowerCase();
  switch (ext) {
    case "sh":
      return `bash "${path}"`;
    case "py":
      return `${remote ? "python3" : "py -3"} "${path}"`;
    case "ps1":
      return `${remote ? "powershell" : "powershell.exe"} -File "${path}"`;
    case "bat":
    case "cmd":
      return `cmd /c "${path}"`;
    case "exe":
      return `"${path}"`;
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
  const createRowRef = useRef<HTMLDivElement>(null);
  const treeContainerRef = useRef<HTMLDivElement>(null);
  const [dragPath, setDragPath] = useState<string | null>(null);
  const [dropPath, setDropPath] = useState<string | null>(null);

  const rootEntries = children[rootPath] ?? [];

  // 重命名/新建/删除/剪切复制粘贴/多选/按名过滤——这套逻辑和编辑器模块左侧的
  // 本地文件树（`LocalFileTree.tsx`）共用同一份实现（2026-09 用户要求"真正
  // 合并成一份共用实现"，不是照抄一遍）。拖拽移动（`dragPath`/`dropPath`）和
  // 上面的 `menu`/右键菜单结构两边差异较大，留在各自组件里。
  const backend: FileTreeBackend = useMemo(
    () => ({
      deleteFile: (path, isDir) => fsService.deleteFile(workspaceId, path, isDir),
      rename: (from, to) => fsService.rename(workspaceId, from, to),
      copy: (from, to, isDir) => fsService.copy(workspaceId, from, to, isDir),
      createDir: (path) => fsService.createDir(workspaceId, path),
      writeFile: (path, content) => fsService.writeFile(workspaceId, path, content, null).then(() => undefined),
    }),
    [workspaceId],
  );
  const ops = useFileTreeOperations({
    backend,
    // `parentOf(entry.path)` 用的是后端统一正规化过的 `/` 分隔符，`rootPath`
    // （原生目录选择器选出来的）可能是系统原样的 `\`——不做这层归一化比较，
    // "重命名/新建/删除根目录下的一级文件"算出来的 parent 和 `rootPath` 是
    // 两个不同的字符串，会当成一个从没请求过的新目录去 reloadDir，根目录本身
    // 反而没刷新（2026-09 之前 `commitRename` 专门为这个额外重载一次 rootPath
    // 的写法就是绕开这个问题，这里直接在归一化层面修掉，新建/删除/粘贴也一并
    // 受益，不用再各自重载两次）。
    reloadDir: (path) => {
      const normalized = path.replace(/\\/g, "/").replace(/\/$/, "");
      const normalizedRoot = rootPath.replace(/\\/g, "/").replace(/\/$/, "");
      const target = path === "" || normalized.toLowerCase() === normalizedRoot.toLowerCase() ? rootPath : path;
      return reloadDir(workspaceId, target);
    },
    select,
    getFlattenedVisible: () => flattenVisible(rootEntries, children, expanded),
    childrenOf: (parentPath) => children[parentPath],
    onOpenFile,
    onFileDeleted: (path) => {
      if (useEditorStore.getState().buffers[path]) useEditorStore.getState().close(path);
    },
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

  /** 对着目录新建要先保证它已展开（懒加载的子目录列表还没拉过时，`children[parentPath]`
   * 是 undefined，输入框无处挂载）——这一步是 explorerStore 懒加载特有的，通用 hook
   * 不关心"目录展开"这个概念，包一层。 */
  const startCreate = async (parentPath: string, depth: number, isDir: boolean) => {
    if (!expanded.has(parentPath)) {
      await toggleDir(workspaceId, parentPath);
    }
    ops.startCreate(parentPath, depth, isDir);
  };

  useEffect(() => {
    loadRoot(workspaceId, rootPath);
  }, [workspaceId, rootPath, loadRoot]);

  useEffect(() => {
    let disposed = false;
    let refreshTimer: ReturnType<typeof setTimeout> | undefined;
    let unlisten: (() => void) | undefined;

    void listen<{ workspaceId: string }>("fs:changed", (event) => {
      if (event.payload.workspaceId !== workspaceId) return;
      clearTimeout(refreshTimer);
      refreshTimer = setTimeout(() => {
        void refreshAll(workspaceId, rootPath);
      }, 100);
    }).then((stop) => {
      if (disposed) stop();
      else unlisten = stop;
    });

    return () => {
      disposed = true;
      clearTimeout(refreshTimer);
      unlisten?.();
    };
  }, [workspaceId, rootPath, refreshAll]);

  useEffect(() => {
    const handler = async (ev: Event) => {
      const path = (ev as CustomEvent<{ path?: string }>).detail?.path;
      if (!path) return;
      const normalized = path.replace(/\\/g, "/");
      const root = rootPath.replace(/\\/g, "/").replace(/\/$/, "");
      if (!normalized.toLowerCase().startsWith(root.toLowerCase())) return;
      const parts = normalized.split("/").filter(Boolean);
      const rootParts = root.split("/").filter(Boolean);
      for (let i = rootParts.length; i < parts.length - 1; i++) {
        const dir = parts.slice(0, i + 1).join("/");
        if (!useExplorerStore.getState().expanded.has(dir)) await toggleDir(workspaceId, dir);
      }
      select(path);
    };
    window.addEventListener("roc:reveal-explorer", handler);
    return () => window.removeEventListener("roc:reveal-explorer", handler);
  }, [workspaceId, rootPath, toggleDir, select]);

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

  const runScript = async (entry: FileEntry) => {
    const current = useWorkspaceStore.getState().current;
    if (!current) return;
    // .exe 不走终端——2026-09 用户反馈："右键 EXE 运行时，需要类似本地 Windows
    // 下双击 EXE 程序，而不是把命令放终端里"。脚本类型（.py/.sh/.ps1/...）
    // 放进终端是对的（用户往往想看交互输出、能继续在终端里敲下一条命令），但
    // 编译好的可执行文件更多是"启动一个独立程序"，用户期望的是它弹出自己的
    // 窗口（控制台程序自己的控制台/GUI 程序自己的窗口），不是把它的 stdio
    // 接进 roc_desk 自带的终端面板——那样体验和双击完全不一样，还会因为
    // 终端换了别的用途被误杀。复用已有的"外部程序打开"命令
    // （`fs_open_externally`，Tauri opener 插件的 `open_path`）——对可执行
    // 文件来说，"用系统默认方式打开"就是"启动这个进程"，效果就是双击；远程
    // 工作区会先下载到本地临时目录再打开，和这个命令打开其它文件类型时的
    // 既有行为一致，不是这里专门加的特殊分支。
    if (entry.path.split(".").pop()?.toLowerCase() === "exe") {
      try {
        await fsService.openExternally(workspaceId, entry.path);
      } catch (e) {
        push("error", `运行失败：${formatError(e)}`);
      }
      return;
    }
    const remote = current.kind === "remote";
    const cmd = runCommandFor(entry.path, remote);
    if (!cmd) return;
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
   * 文件可以将他导入本地搜索引擎进行搜索"）——复用日志搜索模块已有的批量导入命令（`log_import_
   * local_paths`/`log_import_remote_paths`，这里只传一项），之前只能从"日志搜索"面板里点"导入本地/远程文件"再
   * 弹文件选择框去找，这里是从 Explorer 直接对着已经在看的文件走这条路径，少绕一圈。
   * 不限制文件扩展名——导入命令本身就是按行读文本进 FTS5 索引，不是"专属 .log"的能力，
   * 限制成只对 .log 显示反而人为缩小了这个入口的适用范围。 */
  const importToLogSearch = async (entry: FileEntry) => {
    const current = useWorkspaceStore.getState().current;
    if (!current) return;
    try {
      const requestId = crypto.randomUUID();
      const outcome =
        current.kind === "remote" && current.connection_id
          ? await logSearchService.importRemotePaths(current.connection_id, [entry.path], false, current.display_name, requestId)
          : await logSearchService.importLocalPaths([entry.path], false, current.display_name, requestId);
      if (outcome.failed.length > 0) {
        push("error", `导入失败：${outcome.failed[0].error}`);
      } else {
        push("success", `已导入 ${outcome.lines_imported} 行到本地搜索引擎`);
      }
    } catch (e) {
      push("error", `导入失败：${formatError(e)}`);
    }
  };

  const moveDraggedInto = async (targetDir: string) => {
    if (!dragPath) return;
    const source = dragPath;
    const sourceParent = parentOf(source);
    const normalizedSource = source.replace(/\\/g, "/").replace(/\/$/, "").toLowerCase();
    const normalizedTarget = targetDir.replace(/\\/g, "/").replace(/\/$/, "").toLowerCase();
    if (normalizedSource === normalizedTarget || normalizedTarget.startsWith(`${normalizedSource}/`)) {
      push("error", "不能把文件夹移动到自身或其子目录中");
      return;
    }
    const name = baseName(source);
    const destination = `${targetDir}/${name}`;
    try {
      await fsService.rename(workspaceId, source, destination);
      setDragPath(null);
      setDropPath(null);
      await reloadDir(workspaceId, targetDir);
      if (sourceParent !== targetDir) await reloadDir(workspaceId, sourceParent);
      select(destination);
    } catch (e) {
      push("error", `移动失败：${formatError(e)}`);
    }
  };

  const menuItems = (entry: FileEntry, depth: number): ContextMenuItem[] => {
    // Windows 本地工作区下 `entry.path` 和 `rootPath` 分隔符不一致：后端 `list_dir`
    // 统一把路径正规化成 `/`（见 fsops/local.rs），但 `rootPath`（原生目录选择器
    // 选出来的）保留系统原样的 `\`——直接 `startsWith` 永远不命中，"复制相对路径"
    // 拿到的其实是 `entry.path` 这个 fallback，也就是完整路径（2026-09 用户反馈）。
    // 和上面 `roc:reveal-explorer` 处理器（116 行）同样的思路：两边都正规化成 `/`
    // 再比较，顺带按小写比对——Windows 路径大小写不敏感，`rootPath` 和实际列出来的
    // 盘符/目录名大小写不一定完全一致。
    const normalizedRoot = rootPath.replace(/\\/g, "/").replace(/\/$/, "");
    const normalizedEntryPath = entry.path.replace(/\\/g, "/");
    const relativePath = normalizedEntryPath.toLowerCase().startsWith(normalizedRoot.toLowerCase())
      ? normalizedEntryPath.slice(normalizedRoot.length).replace(/^\//, "")
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
    if (!entry.is_dir && runCommandFor(entry.path, useWorkspaceStore.getState().current?.kind === "remote")) {
      items.push({ label: "运行", onClick: () => runScript(entry) });
    }
    if (!entry.is_dir) {
      items.push({ label: "导入到本地搜索引擎", onClick: () => importToLogSearch(entry) });
    }
    items.push(
      { label: "重命名", onClick: () => startRename(entry), separatorBefore: !entry.is_dir },
      { label: "删除", onClick: () => requestDelete([entry]), danger: true },
      {
        label: "剪切",
        onClick: () => setClipboard({ items: [{ path: entry.path, name: entry.name, isDir: entry.is_dir }], mode: "cut" }),
        separatorBefore: true,
      },
    );
    items.push({
      label: "复制",
      onClick: () => setClipboard({ items: [{ path: entry.path, name: entry.name, isDir: entry.is_dir }], mode: "copy" }),
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
          className={`tree-item ${selectedPath === entry.path ? "active" : ""} ${multiSelected.has(entry.path) ? "multi-selected" : ""} ${dropPath === entry.path ? "drop-target" : ""}`}
          draggable
          style={{ paddingLeft: 8 + depth * 16 }}
          data-path={entry.path}
          onClick={(e) => {
            if (isRenaming) return;
            handleItemClick(e, entry, (target) => {
              if (target.is_dir) {
                toggleDir(workspaceId, target.path);
              } else {
                onOpenFile(target.path);
              }
            });
          }}
          onDoubleClick={() => {
            if (!entry.is_dir && !isRenaming) {
              onOpenFile(entry.path, { pin: true });
            }
          }}
          onContextMenu={(e) => {
            e.preventDefault();
            e.stopPropagation();
            handleContextMenuSelect(entry);
            setMenu({ x: e.clientX, y: e.clientY, entry, depth });
          }}
          onDragStart={(e) => {
            setDragPath(entry.path);
            e.dataTransfer.effectAllowed = "move";
            e.dataTransfer.setData("text/plain", entry.path);
          }}
          onDragEnd={() => {
            setDragPath(null);
            setDropPath(null);
          }}
          onDragOver={(e) => {
            if (!entry.is_dir || !dragPath || dragPath === entry.path) return;
            e.preventDefault();
            e.dataTransfer.dropEffect = "move";
            setDropPath(entry.path);
          }}
          onDragLeave={() => {
            if (dropPath === entry.path) setDropPath(null);
          }}
          onDrop={(e) => {
            e.preventDefault();
            if (entry.is_dir) void moveDraggedInto(entry.path);
            else setDropPath(null);
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

  return (
    <div
      ref={treeContainerRef}
      className="project-tree"
      onDragOver={(e) => {
        if (!dragPath) return;
        e.preventDefault();
        e.dataTransfer.dropEffect = "move";
        setDropPath(rootPath);
      }}
      onDragLeave={(e) => {
        if (e.currentTarget === e.target) setDropPath(null);
      }}
      onDrop={(e) => {
        e.preventDefault();
        if (e.target === e.currentTarget) void moveDraggedInto(rootPath);
      }}
      onClick={(e) => {
        // 点到空白背景（没冒泡自某一行）——清空多选，和资源管理器一致；单个
        // `selectedPath` 故意保留，点空白不应该连"当前焦点在哪一行"都清掉。
        if (e.target === e.currentTarget) clearSelection();
      }}
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
              ? multiSelected.size > 1 && multiSelected.has(menu.entry.path)
                ? batchMenuItems(flattenVisible(rootEntries, children, expanded).filter((e) => multiSelected.has(e.path)))
                : menuItems(menu.entry, menu.depth)
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
