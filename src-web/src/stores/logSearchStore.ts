import { create } from "zustand";
import { logSearchService } from "../services/logSearchService";
import { formatError } from "../utils/error";
import type { IndexStats, LiveSearchResult, LogSearchResult } from "../types/bindings";

export type LogSearchMode = "index" | "live";

interface LogSearchState {
  mode: LogSearchMode;
  query: string;
  livePath: string;
  isRegex: boolean;
  loading: boolean;
  error: string | null;
  indexResults: LogSearchResult[];
  liveResults: LiveSearchResult[];
  stats: IndexStats | null;
  selectedIndex: number | null;
  importing: boolean;

  setMode: (mode: LogSearchMode) => void;
  setQuery: (q: string) => void;
  setLivePath: (p: string) => void;
  setIsRegex: (v: boolean) => void;
  select: (index: number | null) => void;
  runSearch: (workspaceKind: "local" | "remote", profileId: string | null) => Promise<void>;
  loadStats: () => Promise<void>;
  importLocalFile: (path: string, hostName: string) => Promise<number>;
  importRemoteFile: (profileId: string, remotePath: string, hostName: string) => Promise<number>;
  clearOlderThan: (days: number) => Promise<number>;
  reset: () => void;
}

export const useLogSearchStore = create<LogSearchState>((set, get) => ({
  mode: "index",
  query: "",
  livePath: "/",
  isRegex: false,
  loading: false,
  error: null,
  indexResults: [],
  liveResults: [],
  stats: null,
  selectedIndex: null,
  importing: false,

  setMode: (mode) => set({ mode, selectedIndex: null }),
  setQuery: (query) => set({ query }),
  setLivePath: (livePath) => set({ livePath }),
  setIsRegex: (isRegex) => set({ isRegex }),
  select: (selectedIndex) => set({ selectedIndex }),

  runSearch: async (workspaceKind, profileId) => {
    const { mode, query, livePath, isRegex } = get();
    if (!query.trim()) return;
    set({ loading: true, error: null, selectedIndex: null });
    try {
      if (mode === "index") {
        const results = await logSearchService.searchIndex({ query, limit: 200 });
        set({ indexResults: results, loading: false });
      } else {
        if (workspaceKind !== "remote" || !profileId) {
          set({ loading: false, error: "实时搜索仅支持远程工作区" });
          return;
        }
        const results = await logSearchService.searchLive(profileId, query, livePath, isRegex);
        set({ liveResults: results, loading: false });
      }
    } catch (e) {
      set({ loading: false, error: formatError(e) });
    }
  },

  loadStats: async () => {
    try {
      const stats = await logSearchService.indexStats();
      set({ stats });
    } catch (e) {
      set({ error: formatError(e) });
    }
  },

  importLocalFile: async (path, hostName) => {
    set({ importing: true, error: null });
    try {
      const count = await logSearchService.importLocalFile(path, hostName);
      await get().loadStats();
      return count;
    } catch (e) {
      set({ error: formatError(e) });
      throw e;
    } finally {
      set({ importing: false });
    }
  },

  importRemoteFile: async (profileId, remotePath, hostName) => {
    set({ importing: true, error: null });
    try {
      const count = await logSearchService.importFile(profileId, remotePath, hostName);
      await get().loadStats();
      return count;
    } catch (e) {
      set({ error: formatError(e) });
      throw e;
    } finally {
      set({ importing: false });
    }
  },

  clearOlderThan: async (days) => {
    try {
      const removed = await logSearchService.indexClear(days);
      await get().loadStats();
      return removed;
    } catch (e) {
      set({ error: formatError(e) });
      throw e;
    }
  },

  reset: () =>
    set({
      mode: "index",
      query: "",
      livePath: "/",
      isRegex: false,
      loading: false,
      error: null,
      indexResults: [],
      liveResults: [],
      stats: null,
      selectedIndex: null,
      importing: false,
    }),
}));
