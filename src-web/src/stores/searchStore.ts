import { create } from "zustand";
import { listen } from "@tauri-apps/api/event";
import { fsService } from "../services/fsService";
import { formatError } from "../utils/error";
import type {
  SearchDoneEvent,
  SearchErrorEvent,
  SearchFileResult,
  SearchFileResultEvent,
  SearchMode,
  SearchOptions,
} from "../types/bindings";

interface SearchState {
  query: string;
  replacement: string;
  replaceOpen: boolean;
  mode: SearchMode;
  options: SearchOptions;
  /** 右键子目录"在此文件夹中搜索"时设置；null 表示搜整个工作区。 */
  scopePath: string | null;
  /** 展示给用户看的范围说明（比如相对路径 "kgms/conf/"），和 scopePath 一起设置。 */
  scopeLabel: string | null;
  results: SearchFileResult[];
  truncated: boolean;
  loading: boolean;
  replacing: boolean;
  error: string | null;
  /** 折叠状态按文件路径记，默认展开。 */
  collapsed: Set<string>;
  /** 当前这次搜索的 request id，用来在流式事件里过滤掉被新搜索取代的旧请求的尾巴。 */
  requestId: string | null;

  setQuery: (q: string) => void;
  setReplacement: (r: string) => void;
  setReplaceOpen: (open: boolean) => void;
  setMode: (mode: SearchMode) => void;
  setOption: (key: keyof SearchOptions, value: boolean) => void;
  setScope: (path: string | null, label: string | null) => void;
  toggleCollapsed: (path: string) => void;
  runSearch: (workspaceId: string) => Promise<void>;
  replaceAll: (workspaceId: string) => Promise<void>;
  replaceInFile: (workspaceId: string, path: string) => Promise<void>;
  clear: () => void;
}

let listenersRegistered = false;

export const useSearchStore = create<SearchState>((set, get) => ({
  query: "",
  replacement: "",
  replaceOpen: false,
  mode: "content",
  options: { case_sensitive: false, whole_word: false, use_regex: false },
  scopePath: null,
  scopeLabel: null,
  results: [],
  truncated: false,
  loading: false,
  replacing: false,
  error: null,
  collapsed: new Set(),
  requestId: null,

  setQuery: (query) => set({ query }),
  setReplacement: (replacement) => set({ replacement }),
  setReplaceOpen: (replaceOpen) => set({ replaceOpen }),
  setMode: (mode) => set({ mode }),
  setOption: (key, value) => set((s) => ({ options: { ...s.options, [key]: value } })),
  setScope: (scopePath, scopeLabel) => set({ scopePath, scopeLabel }),
  toggleCollapsed: (path) =>
    set((s) => {
      const collapsed = new Set(s.collapsed);
      if (collapsed.has(path)) collapsed.delete(path);
      else collapsed.add(path);
      return { collapsed };
    }),

  // 流式：这个方法只负责"发起"这次搜索并把 requestId 记下来，真正的结果由
  // registerSearchListeners 里的事件监听器陆续写进 results（2026-08-18 需求：
  // "能否一个一个目录搜，搜到一部分先展示一部分"）。开始新搜索时换一个新
  // requestId，后端那边发现自己不再是"当前"请求会尽快中止，天然实现了
  // "输入新关键词自动取消上一次还没搜完的慢搜索"。
  runSearch: async (workspaceId) => {
    const { query, mode, options, scopePath } = get();
    if (!query.trim()) {
      set({ results: [], truncated: false, error: null, requestId: null, loading: false });
      return;
    }
    const requestId = crypto.randomUUID();
    set({ loading: true, error: null, results: [], truncated: false, collapsed: new Set(), requestId });
    try {
      await fsService.searchStream(workspaceId, requestId, scopePath, query, mode, options);
    } catch (e) {
      if (get().requestId === requestId) {
        set({ loading: false, error: formatError(e) });
      }
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

  clear: () =>
    set({
      query: "",
      replacement: "",
      results: [],
      truncated: false,
      error: null,
      collapsed: new Set(),
      requestId: null,
      loading: false,
    }),
}));

/** 全局注册一次流式事件监听（App.tsx 挂载时调用，和 registerAiChatListeners 同款模式）。*/
export function registerSearchListeners(): Promise<() => void> {
  if (listenersRegistered) return Promise.resolve(() => {});
  listenersRegistered = true;

  const unlistenPromises = [
    listen<SearchFileResultEvent>("search:file-result", (event) => {
      const { requestId } = useSearchStore.getState();
      if (event.payload.requestId !== requestId) return;
      useSearchStore.setState((s) => {
        const idx = s.results.findIndex((f) => f.path === event.payload.file.path);
        if (idx >= 0) {
          // 防御性合并：后端已经按完整路径分组，正常不会对同一路径推两次事件；
          // 这里兜底保证万一出现也不会在界面上裂成两行——用户明确反馈过"同一个
          // 文件的多个位置需要合并在一条"。
          const merged = [...s.results];
          merged[idx] = { ...merged[idx], matches: [...merged[idx].matches, ...event.payload.file.matches] };
          return { results: merged };
        }
        return { results: [...s.results, event.payload.file] };
      });
    }),
    listen<SearchDoneEvent>("search:done", (event) => {
      const { requestId } = useSearchStore.getState();
      if (event.payload.requestId !== requestId) return;
      useSearchStore.setState({ loading: false, truncated: event.payload.truncated });
    }),
    listen<SearchErrorEvent>("search:error", (event) => {
      const { requestId } = useSearchStore.getState();
      if (event.payload.requestId !== requestId) return;
      useSearchStore.setState({ loading: false, error: event.payload.message });
    }),
  ];

  return Promise.all(unlistenPromises).then((unlistens) => () => unlistens.forEach((u) => u()));
}
