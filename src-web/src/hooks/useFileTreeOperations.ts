import React, { useState } from "react";
import { useToastStore } from "../components/shared/Toast";
import type { ContextMenuItem } from "../components/shared/ContextMenu";
import { formatError } from "../utils/error";
import type { FileEntry } from "../types/bindings";

export function parentOf(path: string): string {
  const idx = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  return idx >= 0 ? path.slice(0, idx) : path;
}

export function baseName(path: string): string {
  return path.split(/[/\\]/).filter(Boolean).pop() ?? path;
}

/** 按当前展开状态把树"拍平"成屏幕上从上到下的实际顺序——Shift 范围多选需要
 * 知道两次点击之间"中间还有哪些行"，而这些行分布在不同深度的嵌套目录里，
 * 不是简单的数组切片能算出来的，必须按渲染时同样的规则（只有展开的目录才
 * 把子项接进来）递归展平一遍。两棵树（工作区资源管理器/编辑器本地文件树）
 * 用的是同一份实现。 */
export function flattenVisible(
  entries: FileEntry[],
  childrenMap: Record<string, FileEntry[] | undefined>,
  expandedSet: Set<string>,
): FileEntry[] {
  const result: FileEntry[] = [];
  for (const entry of entries) {
    result.push(entry);
    if (entry.is_dir && expandedSet.has(entry.path)) {
      const kids = childrenMap[entry.path];
      if (kids) result.push(...flattenVisible(kids, childrenMap, expandedSet));
    }
  }
  return result;
}

/** 一棵文件树要接进这套共用操作逻辑，需要提供的最小后端能力——工作区资源
 * 管理器用的是 `fsService`（按 workspace_id 校验边界），编辑器本地文件树用
 * 的是 `localFsService`（直接对着本机任意路径），两者签名形状不同，这里
 * 只要求调用方各自包一层适配。 */
export interface FileTreeBackend {
  deleteFile(path: string, isDir: boolean): Promise<void>;
  rename(from: string, to: string): Promise<void>;
  copy(from: string, to: string, isDir: boolean): Promise<void>;
  createDir(path: string): Promise<void>;
  writeFile(path: string, content: string): Promise<void>;
}

export interface UseFileTreeOperationsOptions {
  backend: FileTreeBackend;
  /** 目录内容变化后重新拉取——调用方各自管理自己的 children 状态形状（Zustand
   * store / 组件内 useState 都行），这个 hook 不关心具体存在哪里。 */
  reloadDir: (path: string) => Promise<void>;
  /** 更新"当前焦点项"（不是多选状态，是驱动预览打开/滚动定位的单个路径）。 */
  select: (path: string) => void;
  /** 按当前展开状态取一份屏幕顺序的可见条目——Shift 范围多选、批量右键取
   * 选中项集合都要用，随点击时的最新状态实时算，不缓存。 */
  getFlattenedVisible: () => FileEntry[];
  /** 判断某个父目录下是否已经有同名条目——新建文件/文件夹时拦一下重名。 */
  childrenOf: (parentPath: string) => FileEntry[] | undefined;
  /** 新建文件成功后直接打开（`pin: true`）——可选，不是所有调用方都需要。 */
  onOpenFile?: (path: string, opts?: { pin?: boolean }) => void;
  /** 删除一个文件成功后顺带关掉它对应的已打开编辑器 buffer——可选。 */
  onFileDeleted?: (path: string) => void;
}

/**
 * 文件树的"操作"逻辑——多选（Shift 范围/Ctrl 点选）、剪切/复制/粘贴、删除
 * （批量，逐项独立 try/catch，一项失败不影响其它项）、重命名、新建文件/
 * 文件夹、按文件名过滤——这套逻辑原本在 `ExplorerTree.tsx`（工作区资源
 * 管理器）里独立实现，`LocalFileTree.tsx`（编辑器左侧本地文件树）完全没有
 * （2026-09 用户反馈"和工作区一样，编辑器模式的本地文件树也需要能搜索/删除/
 * 复制"，并且要求"真正合并成一份共用实现"，不是照抄一遍）。
 *
 * 故意不管的部分：具体怎么"列目录"（`children`/`expanded` 的状态形状）、
 * 拖拽移动、运行脚本、导入日志搜索、Diff 对比——这些要么两边状态管理方式
 * 本来就不同（前者），要么是工作区场景特有、本地文件树没有对应概念（后
 * 几个），硬塞进一个"万能 hook"只会增加一层没有共同受益者的抽象，各自在
 * 组件里按需要另外加。
 */
export function useFileTreeOperations({
  backend,
  reloadDir,
  select,
  getFlattenedVisible,
  childrenOf,
  onOpenFile,
  onFileDeleted,
}: UseFileTreeOperationsOptions) {
  const push = useToastStore((s) => s.push);
  const [renamingPath, setRenamingPath] = useState<string | null>(null);
  const [renameValue, setRenameValue] = useState("");
  const [creating, setCreating] = useState<{ parentPath: string; depth: number; isDir: boolean } | null>(null);
  const [createValue, setCreateValue] = useState("");
  const [deleteTargets, setDeleteTargets] = useState<FileEntry[]>([]);
  const [clipboard, setClipboard] = useState<{ items: { path: string; name: string; isDir: boolean }[]; mode: "cut" | "copy" } | null>(
    null,
  );
  // 多选——和"当前焦点项"（调用方自己的 `selectedPath`）并存：普通点击时
  // `multiSelected` 最多一项，行为和没有多选功能时完全一样；只有按住
  // Ctrl/Shift 点击才会真正超过一项。`selectionAnchor` 是 Shift 范围选的
  // 起点，普通点击/Ctrl 点击都会把它更新成当前项，Shift 点击本身不动它——
  // 连续按 Shift 点几个不同的行，范围始终从同一个起点重新计算，和资源
  // 管理器的行为一致。
  const [multiSelected, setMultiSelected] = useState<Set<string>>(new Set());
  const [selectionAnchor, setSelectionAnchor] = useState<string | null>(null);
  const [filterQuery, setFilterQuery] = useState("");

  const startRename = (entry: FileEntry) => {
    setRenamingPath(entry.path);
    setRenameValue(entry.name);
  };
  const cancelRename = () => setRenamingPath(null);

  const commitRename = async (entry: FileEntry) => {
    const newName = renameValue.trim();
    setRenamingPath(null);
    if (!newName || newName === entry.name) return;
    const parent = parentOf(entry.path);
    const to = `${parent}/${newName}`;
    try {
      await backend.rename(entry.path, to);
      await reloadDir(parent);
    } catch (e) {
      push("error", `重命名失败：${formatError(e)}`);
    }
  };

  const startCreate = (parentPath: string, depth: number, isDir: boolean) => {
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
    // 客户端先查一遍重名——后端"新建"接口对已存在的文件/目录不一定报错
    // （可能是覆盖/合并语义），这里主动拦一下，避免用户以为在新建、实际上
    // 悄悄覆盖了一个同名文件/目录。
    if ((childrenOf(parentPath) ?? []).some((e) => e.name === name)) {
      push("error", `${name} 已存在`);
      return;
    }
    const path = `${parentPath}/${name}`;
    try {
      if (isDir) await backend.createDir(path);
      else await backend.writeFile(path, "");
      await reloadDir(parentPath);
      select(path);
      if (!isDir) onOpenFile?.(path, { pin: true });
    } catch (e) {
      push("error", `新建${isDir ? "文件夹" : "文件"}失败：${formatError(e)}`);
    }
  };

  const requestDelete = (entries: FileEntry[]) => setDeleteTargets(entries);
  const cancelDelete = () => setDeleteTargets([]);

  const confirmDelete = async () => {
    if (deleteTargets.length === 0) return;
    const entries = deleteTargets;
    setDeleteTargets([]);
    // 逐项单独 try/catch——一个删除失败（比如文件正被占用）不影响继续删除
    // 其它项，处理完之后用 toast 汇总报告失败的数量和原因（参考 AI 编程
    // 助手"全部应用"批量操作同样的教训：之前中途失败会直接中断整个循环，
    // 剩下的项完全没被尝试）。
    const parentsToReload = new Set<string>();
    let failed = 0;
    let lastError: unknown = null;
    for (const entry of entries) {
      try {
        await backend.deleteFile(entry.path, entry.is_dir);
        parentsToReload.add(parentOf(entry.path));
        onFileDeleted?.(entry.path);
      } catch (e) {
        failed += 1;
        lastError = e;
      }
    }
    for (const parent of parentsToReload) await reloadDir(parent);
    setMultiSelected(new Set());
    const succeeded = entries.length - failed;
    if (succeeded > 0) {
      push("success", entries.length === 1 ? `已删除 ${entries[0].name}` : `已删除 ${succeeded} 项`);
    }
    if (failed > 0) {
      push("error", `${entries.length} 项中有 ${failed} 项删除失败：${formatError(lastError)}`);
    }
  };

  /** 剪切=记下来源+等粘贴时挪过去（复用 rename）；复制=复用后端的 copy（文件/
   * 目录都支持，目录会在后端递归复制）。`clipboard.items` 可能有多项（多选
   * 后批量剪切/复制）——逐项单独 try/catch，和 `confirmDelete` 同样的健壮性
   * 考虑。 */
  const pasteInto = async (targetDir: string) => {
    if (!clipboard) return;
    const items = clipboard.items;
    const srcParents = new Set<string>();
    const reloadTargets = new Set<string>([targetDir]);
    let failed = 0;
    let lastError: unknown = null;
    for (const item of items) {
      const dest = `${targetDir}/${item.name}`;
      try {
        if (clipboard.mode === "cut") {
          await backend.rename(item.path, dest);
          srcParents.add(parentOf(item.path));
        } else {
          await backend.copy(item.path, dest, item.isDir);
        }
      } catch (e) {
        failed += 1;
        lastError = e;
      }
    }
    if (clipboard.mode === "cut") setClipboard(null);
    for (const parent of srcParents) reloadTargets.add(parent);
    for (const dir of reloadTargets) await reloadDir(dir);
    if (failed > 0) {
      push("error", `${items.length} 项中有 ${failed} 项粘贴失败：${formatError(lastError)}`);
    }
  };

  /** 单击一个条目的完整选中逻辑（普通/Shift 范围/Ctrl 点选）——`onPlainOpen`
   * 是"普通点击、且不是修饰键多选"时要做的事（目录展开/文件打开预览），
   * 不同调用方这一步不一样，所以作为回调传进来，这个 hook 本身不关心"打开"
   * 具体是什么含义。 */
  const handleItemClick = (e: React.MouseEvent, entry: FileEntry, onPlainOpen: (entry: FileEntry) => void) => {
    if (e.shiftKey) {
      const flat = getFlattenedVisible().map((x) => x.path);
      const anchor = selectionAnchor ?? entry.path;
      const ai = flat.indexOf(anchor);
      const bi = flat.indexOf(entry.path);
      if (ai === -1 || bi === -1) {
        setMultiSelected(new Set([entry.path]));
      } else {
        const [lo, hi] = ai < bi ? [ai, bi] : [bi, ai];
        setMultiSelected(new Set(flat.slice(lo, hi + 1)));
      }
      select(entry.path);
      return;
    }
    if (e.ctrlKey || e.metaKey) {
      setMultiSelected((prev) => {
        const next = new Set(prev);
        if (next.has(entry.path)) next.delete(entry.path);
        else next.add(entry.path);
        return next;
      });
      setSelectionAnchor(entry.path);
      select(entry.path);
      return;
    }
    setMultiSelected(new Set());
    setSelectionAnchor(entry.path);
    select(entry.path);
    onPlainOpen(entry);
  };

  /** 右键一个不在当前多选范围内的项——先重置成只选中这一项，和资源管理器的
   * 行为一致（右键从来不会保留一份"和这次右键无关"的旧选区）。 */
  const handleContextMenuSelect = (entry: FileEntry) => {
    if (!multiSelected.has(entry.path)) {
      setMultiSelected(new Set());
      setSelectionAnchor(entry.path);
    }
    select(entry.path);
  };

  const clearSelection = () => setMultiSelected(new Set());

  /** 多选时右键其中一项——菜单收窄成只有"删除/剪切/复制"这几个天然支持批量
   * 的操作，不逐项列出"打开"/"重命名"这些只对单个文件有意义的动作（重命名
   * 多个文件重命名成什么？改成批量重命名是完全不同的功能）。粘贴的目标
   * 目录取"这些选中项的公共父目录"——多选通常发生在同一层，跨目录多选粘贴
   * 去哪里没有唯一合理的答案，简化成统一用第一项的父目录。 */
  const batchMenuItems = (entries: FileEntry[]): ContextMenuItem[] => {
    const items: ContextMenuItem[] = [
      { label: `删除选中的 ${entries.length} 项`, onClick: () => requestDelete(entries), danger: true },
      {
        label: `剪切 ${entries.length} 项`,
        onClick: () => setClipboard({ items: entries.map((e) => ({ path: e.path, name: e.name, isDir: e.is_dir })), mode: "cut" }),
        separatorBefore: true,
      },
      {
        label: `复制 ${entries.length} 项`,
        onClick: () => setClipboard({ items: entries.map((e) => ({ path: e.path, name: e.name, isDir: e.is_dir })), mode: "copy" }),
      },
    ];
    if (clipboard) items.push({ label: "粘贴", onClick: () => pasteInto(parentOf(entries[0].path)) });
    return items;
  };

  /** 按文件名做大小写不敏感的子串过滤——纯前端过滤已经加载到内存里的条目，
   * 不是重新发请求；目录懒加载还没展开的子树自然搜不到，这是"过滤已知内容"
   * 而不是"全局搜索"，和 LocalExplorerScreen 的"过滤文件名"是同一个定位。 */
  const matchesFilter = (name: string): boolean => {
    if (!filterQuery.trim()) return true;
    return name.toLowerCase().includes(filterQuery.trim().toLowerCase());
  };

  return {
    renamingPath,
    renameValue,
    setRenameValue,
    startRename,
    cancelRename,
    commitRename,
    creating,
    createValue,
    setCreateValue,
    startCreate,
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
    filterQuery,
    setFilterQuery,
    matchesFilter,
  };
}
