import { create } from "zustand";
import { fsService } from "../services/fsService";
import { formatError } from "../utils/error";
import type { FileEntry } from "../types/bindings";

interface ExplorerState {
  /** 每个目录路径 → 其子项列表（懒加载，UI_DESIGN.md §3.3） */
  children: Record<string, FileEntry[]>;
  expanded: Set<string>;
  loading: Set<string>;
  selectedPath: string | null;
  /** 右键"选择进行比较"记下的源文件路径（参考 VS Code 的 Select for Compare/Compare
   * with Selected 两步式对比）——放在 store 里而不是 ExplorerTree 组件本地 state，
   * 是因为它要跨越两次独立的右键菜单交互存活，组件重渲染/滚动不应该把它清掉。 */
  compareSource: string | null;
  /** 根目录加载失败时的错误信息——之前这里静默吞掉异常，界面上和"空文件夹"完全
   * 看不出区别，用户没法知道要不要重试还是工作区本身就是空的（真实 bug：重新打开
   * 最近工作区时后端 handle 没注册，Explorer 一直显示"空"，但其实是请求失败了）。 */
  rootError: string | null;

  toggleDir: (workspaceId: string, path: string) => Promise<void>;
  loadRoot: (workspaceId: string, rootPath: string) => Promise<void>;
  /** 重命名/删除之后强制重新拉取某个目录的子项，忽略缓存（右键菜单用）。*/
  reloadDir: (workspaceId: string, path: string) => Promise<void>;
  select: (path: string) => void;
  setCompareSource: (path: string | null) => void;
  reset: () => void;
}

export const useExplorerStore = create<ExplorerState>((set, get) => ({
  children: {},
  expanded: new Set(),
  loading: new Set(),
  selectedPath: null,
  compareSource: null,
  rootError: null,

  loadRoot: async (workspaceId, rootPath) => {
    set({ rootError: null });
    try {
      const entries = await fsService.listDir(workspaceId, rootPath);
      set((s) => ({
        children: { ...s.children, [rootPath]: entries },
        expanded: new Set([rootPath]),
      }));
    } catch (e) {
      set({ rootError: formatError(e) });
    }
  },

  toggleDir: async (workspaceId, path) => {
    const isExpanded = get().expanded.has(path);
    if (isExpanded) {
      set((s) => {
        const next = new Set(s.expanded);
        next.delete(path);
        return { expanded: next };
      });
      return;
    }

    set((s) => ({ expanded: new Set(s.expanded).add(path) }));

    if (!get().children[path]) {
      set((s) => ({ loading: new Set(s.loading).add(path) }));
      try {
        const entries = await fsService.listDir(workspaceId, path);
        set((s) => ({ children: { ...s.children, [path]: entries } }));
      } finally {
        set((s) => {
          const next = new Set(s.loading);
          next.delete(path);
          return { loading: next };
        });
      }
    }
  },

  reloadDir: async (workspaceId, path) => {
    try {
      const entries = await fsService.listDir(workspaceId, path);
      set((s) => ({ children: { ...s.children, [path]: entries } }));
    } catch (e) {
      set({ rootError: formatError(e) });
    }
  },

  select: (path) => set({ selectedPath: path }),
  setCompareSource: (path) => set({ compareSource: path }),

  reset: () =>
    set({ children: {}, expanded: new Set(), loading: new Set(), selectedPath: null, compareSource: null, rootError: null }),
}));
