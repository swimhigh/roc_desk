import { create } from "zustand";
import { sftpService } from "../services/sftpService";
import { formatError } from "../utils/error";
import type { FileEntry } from "../types/bindings";

interface SftpState {
  cwd: string;
  entries: FileEntry[];
  loading: boolean;
  selectedPath: string | null;
  error: string | null;

  navigate: (profileId: string, path: string) => Promise<void>;
  select: (path: string | null) => void;
}

/** SFTP 自由浏览快捷工具的状态（DESIGN.md §3.3），与 explorerStore 分开——
 * 那是工作区树的懒加载展开状态，这里是单一"当前目录"的平铺列表导航。 */
export const useSftpStore = create<SftpState>((set) => ({
  cwd: "/",
  entries: [],
  loading: false,
  selectedPath: null,
  error: null,

  navigate: async (profileId, path) => {
    set({ loading: true, error: null });
    try {
      const entries = await sftpService.listDir(profileId, path);
      set({ cwd: path, entries, loading: false, selectedPath: null });
    } catch (e) {
      set({ loading: false, error: formatError(e) });
    }
  },

  select: (path) => set({ selectedPath: path }),
}));
