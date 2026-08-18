import { create } from "zustand";
import { open } from "@tauri-apps/plugin-dialog";
import { workspaceService } from "../services/workspaceService";
import { formatError } from "../utils/error";
import type { WorkspaceProfile } from "../types/bindings";

interface WorkspaceState {
  current: WorkspaceProfile | null;
  recent: WorkspaceProfile[];
  loading: boolean;
  error: string | null;

  loadRecent: () => Promise<void>;
  openLocalFolder: () => Promise<void>;
  openLocalPath: (path: string) => Promise<void>;
  openRemoteWorkspace: (connectionId: string, remotePath: string) => Promise<void>;
  removeFromRecent: (id: string) => Promise<void>;
  backToPicker: () => void;
}

export const useWorkspaceStore = create<WorkspaceState>((set, get) => ({
  current: null,
  recent: [],
  loading: false,
  error: null,

  loadRecent: async () => {
    try {
      const recent = await workspaceService.listRecent();
      set({ recent });
    } catch (e) {
      set({ error: formatError(e) });
    }
  },

  openLocalFolder: async () => {
    const selected = await open({ directory: true, multiple: false });
    if (!selected || Array.isArray(selected)) return;
    await get().openLocalPath(selected);
  },

  /** 重新打开"最近工作区"里的本地目录时也要走这条路径——必须实际调用
   * `workspace_open_local` 在后端注册 WorkspaceHandle，只在前端 setState({ current })
   * 会导致 Explorer 后续所有 fs_* 调用都因为找不到 handle 而静默失败（目录树空白）。 */
  openLocalPath: async (path: string) => {
    set({ loading: true, error: null });
    try {
      const profile = await workspaceService.openLocal(path);
      set({ current: profile, loading: false });
      await get().loadRecent();
    } catch (e) {
      set({ loading: false, error: formatError(e) });
      throw e;
    }
  },

  openRemoteWorkspace: async (connectionId, remotePath) => {
    set({ loading: true, error: null });
    try {
      const profile = await workspaceService.openRemote(connectionId, remotePath);
      set({ current: profile, loading: false });
      await get().loadRecent();
    } catch (e) {
      set({ loading: false, error: formatError(e) });
      throw e;
    }
  },

  removeFromRecent: async (id: string) => {
    await workspaceService.removeRecent(id);
    await get().loadRecent();
  },

  backToPicker: () => {
    set({ current: null });
  },
}));
