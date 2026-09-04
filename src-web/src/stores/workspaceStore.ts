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
  /** 按"最近工作区"里的 id 直接打开——工作区模块窗口冷启动时 `--open=<workspace_id>`
   * 用这个（`docs/HOME_MODES_DESIGN.md` §3.5），和 App.tsx 顶部"切换工作区"下拉菜单
   * 是同一种查找逻辑，只是那边已经有一份内联实现，这里单独抽出来给启动流程复用。 */
  openById: (id: string) => Promise<void>;
  removeFromRecent: (id: string) => Promise<void>;
  updatePath: (id: string, newPath: string) => Promise<WorkspaceProfile>;
  /** SFTP/Agent 双栏浏览器每次导航都调一次（用户需求："下次启动工作区中的SFTP
   * 或文件传输时，直接定位到最后记住的目录"）——只写后端，不更新 `current`：
   * 这次打开期间 `current.last_sftp_*` 保持打开时的旧值也没关系，没有谁会在
   * 中途重新读它，下次真正重新打开这个工作区时 `openLocalPath`/`openRemoteWorkspace`
   * 会取到最新值。如果这里也 `set({ current: {...} })`，会把新值喂回
   * `SftpBrowser`/`AgentBrowser` 的 `initialLocalPath`/`initialRemotePath` prop，
   * 触发它们的挂载 effect 重新导航一遍，变成自己写的值又把自己重新导航一次的
   * 无意义循环。 */
  updateLastSftpPaths: (localPath: string, remotePath: string) => void;
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

  openById: async (id: string) => {
    let list = get().recent;
    if (list.length === 0) {
      await get().loadRecent();
      list = get().recent;
    }
    const profile = list.find((w) => w.id === id);
    if (!profile) throw new Error(`未找到工作区: ${id}`);
    if (profile.kind === "local") {
      await get().openLocalPath(profile.root_path);
    } else if (profile.connection_id) {
      await get().openRemoteWorkspace(profile.connection_id, profile.root_path);
    }
  },

  removeFromRecent: async (id: string) => {
    await workspaceService.removeRecent(id);
    await get().loadRecent();
  },

  /** 目录配错了不用删了重加——直接改这条"最近工作区"记录的 root_path。如果改的
   * 正好是当前已打开的工作区，也同步一下 `current`，虽然实际上目前只有还没打开的
   * 记录（工作区选择页）才会走到编辑入口。 */
  updatePath: async (id: string, newPath: string) => {
    const profile = await workspaceService.updatePath(id, newPath);
    set((state) => ({
      current: state.current?.id === id ? profile : state.current,
    }));
    await get().loadRecent();
    return profile;
  },

  updateLastSftpPaths: (localPath, remotePath) => {
    const id = get().current?.id;
    if (!id) return;
    // 纯记忆便利性写入，失败不影响用户正在做的浏览/传输操作，不弹 toast 打扰。
    workspaceService.updateLastSftpPaths(id, localPath, remotePath).catch((e) => {
      console.error("failed to persist last sftp paths", e);
    });
  },
}));
