import { create } from "zustand";
import { agentService } from "../services/agentService";
import { formatError } from "../utils/error";
import { AGENT_ROOT, isAgentRoot } from "../utils/windowsPath";
import type { FileEntry } from "../types/bindings";

interface AgentBrowseState {
  cwd: string;
  entries: FileEntry[];
  loading: boolean;
  selectedPath: string | null;
  error: string | null;

  navigate: (profileId: string, path: string) => Promise<void>;
  select: (path: string | null) => void;
}

/** Agent 自由浏览快捷工具的状态（AGENT_DESIGN.md §四.3），和 `sftpStore.ts` 是
 * 同一种模式，唯一的区别是 `navigate("")` 要走盘符列表（`listRoots`）而不是
 * `listDir`——Agent 目标没有单一根目录 "/" 的概念，见 `utils/windowsPath.ts`。 */
export const useAgentBrowseStore = create<AgentBrowseState>((set) => ({
  cwd: AGENT_ROOT,
  entries: [],
  loading: false,
  selectedPath: null,
  error: null,

  navigate: async (profileId, path) => {
    set({ loading: true, error: null });
    try {
      const entries = isAgentRoot(path)
        ? (await agentService.listRoots(profileId)).map((root) => ({ name: root, path: root, is_dir: true, size: null, modified: null }))
        : await agentService.listDir(profileId, path);
      set({ cwd: path, entries, loading: false, selectedPath: null });
    } catch (e) {
      set({ loading: false, error: formatError(e) });
    }
  },

  select: (path) => set({ selectedPath: path }),
}));
