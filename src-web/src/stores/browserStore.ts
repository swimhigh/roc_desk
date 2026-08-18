import { create } from "zustand";
import { browserService } from "../services/browserService";
import { formatError } from "../utils/error";
import type { BrowserHistoryEntry } from "../types/bindings";

interface BrowserState {
  history: BrowserHistoryEntry[];
  loading: boolean;
  error: string | null;

  loadHistory: () => Promise<void>;
  removeEntry: (id: string) => Promise<void>;
  clearHistory: () => Promise<void>;
}

export const useBrowserStore = create<BrowserState>((set) => ({
  history: [],
  loading: false,
  error: null,

  loadHistory: async () => {
    set({ loading: true, error: null });
    try {
      const history = await browserService.historyList();
      set({ history, loading: false });
    } catch (e) {
      set({ loading: false, error: formatError(e) });
    }
  },

  removeEntry: async (id) => {
    await browserService.historyRemove(id);
    set((s) => ({ history: s.history.filter((h) => h.id !== id) }));
  },

  clearHistory: async () => {
    await browserService.historyClear();
    set({ history: [] });
  },
}));
