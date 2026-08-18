import { create } from "zustand";
import { fsService } from "../services/fsService";
import { formatError } from "../utils/error";
import type { SearchFileResult, SearchOptions } from "../types/bindings";

interface SearchState {
  query: string;
  replacement: string;
  replaceOpen: boolean;
  options: SearchOptions;
  results: SearchFileResult[];
  truncated: boolean;
  loading: boolean;
  replacing: boolean;
  error: string | null;
  /** 折叠状态按文件路径记，默认展开（Set 里记的是"已折叠"的路径，新结果默认展开更符合
   * "刚搜完就想看内容"的直觉，不用先手动展开）。 */
  collapsed: Set<string>;

  setQuery: (q: string) => void;
  setReplacement: (r: string) => void;
  setReplaceOpen: (open: boolean) => void;
  setOption: (key: keyof SearchOptions, value: boolean) => void;
  toggleCollapsed: (path: string) => void;
  runSearch: (workspaceId: string) => Promise<void>;
  replaceAll: (workspaceId: string) => Promise<void>;
  replaceInFile: (workspaceId: string, path: string) => Promise<void>;
  clear: () => void;
}

export const useSearchStore = create<SearchState>((set, get) => ({
  query: "",
  replacement: "",
  replaceOpen: false,
  options: { case_sensitive: false, whole_word: false, use_regex: false },
  results: [],
  truncated: false,
  loading: false,
  replacing: false,
  error: null,
  collapsed: new Set(),

  setQuery: (query) => set({ query }),
  setReplacement: (replacement) => set({ replacement }),
  setReplaceOpen: (replaceOpen) => set({ replaceOpen }),
  setOption: (key, value) => set((s) => ({ options: { ...s.options, [key]: value } })),
  toggleCollapsed: (path) =>
    set((s) => {
      const collapsed = new Set(s.collapsed);
      if (collapsed.has(path)) collapsed.delete(path);
      else collapsed.add(path);
      return { collapsed };
    }),

  runSearch: async (workspaceId) => {
    const { query, options } = get();
    if (!query.trim()) {
      set({ results: [], truncated: false, error: null });
      return;
    }
    set({ loading: true, error: null });
    try {
      const summary = await fsService.search(workspaceId, query, options);
      set({ results: summary.files, truncated: summary.truncated, loading: false, collapsed: new Set() });
    } catch (e) {
      set({ loading: false, error: formatError(e), results: [] });
    }
  },

  replaceAll: async (workspaceId) => {
    const { query, replacement, options, results } = get();
    const paths = results.map((f) => f.path);
    if (paths.length === 0) return;
    set({ replacing: true, error: null });
    try {
      await fsService.replace(workspaceId, paths, query, replacement, options);
      await get().runSearch(workspaceId);
    } catch (e) {
      set({ error: formatError(e) });
      throw e;
    } finally {
      set({ replacing: false });
    }
  },

  replaceInFile: async (workspaceId, path) => {
    const { query, replacement, options } = get();
    set({ replacing: true, error: null });
    try {
      await fsService.replace(workspaceId, [path], query, replacement, options);
      await get().runSearch(workspaceId);
    } catch (e) {
      set({ error: formatError(e) });
      throw e;
    } finally {
      set({ replacing: false });
    }
  },

  clear: () => set({ query: "", replacement: "", results: [], truncated: false, error: null, collapsed: new Set() }),
}));
