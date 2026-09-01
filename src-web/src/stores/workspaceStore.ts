import { create } from "zustand";
import { open } from "@tauri-apps/plugin-dialog";
import { workspaceService } from "../services/workspaceService";
import { formatError } from "../utils/error";
import type { WorkspaceProfile } from "../types/bindings";

interface WorkspaceState {
  current: WorkspaceProfile | null;
  /** 是否展示工作区选择页——独立于 `current`（2026-09-01 用户反馈：点"返回首页"
   * 后终端会话状态没有保持住）。之前"返回首页"是把 `current` 直接置空，App.tsx
   * 顶层按 `!current` 整个换一棵渲染子树（工作区主界面 vs 选择页），这棵子树里
   * 挂着的 `TerminalPanel` 会连同它渲染的所有"有界保活"工作区的 xterm.js 实例
   * 一起被卸载——后端 SSH Channel/PTY 进程虽然还活着，前端渲染状态却已经没了，
   * 重新进工作区看到的是一个内容清空的新终端。现在"返回首页"只把 `showPicker`
   * 置 true，`current` 保持不变，App.tsx 两棵子树都常驻挂载只用 CSS 切换可见性
   * （和 RemoteTool/HomeShell.tsx 的 showPicker 是同一个模式），首页只是盖在上面
   * 的一层，工作区主界面（含 TerminalPanel）在它底下继续活着。 */
  showPicker: boolean;
  recent: WorkspaceProfile[];
  loading: boolean;
  error: string | null;

  loadRecent: () => Promise<void>;
  openLocalFolder: () => Promise<void>;
  openLocalPath: (path: string) => Promise<void>;
  openRemoteWorkspace: (connectionId: string, remotePath: string) => Promise<void>;
  removeFromRecent: (id: string) => Promise<void>;
  updatePath: (id: string, newPath: string) => Promise<WorkspaceProfile>;
  backToPicker: () => void;
  /** 首页点的正好是当前已经打开的那个工作区——不需要重新走一遍
   * `workspace_open_local`/`workspace_open_remote`，直接把首页收起来就行。 */
  returnToCurrentWorkspace: () => void;
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
  showPicker: true,
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
      set({ current: profile, loading: false, showPicker: false });
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
      set({ current: profile, loading: false, showPicker: false });
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

  backToPicker: () => {
    set({ showPicker: true });
  },

  returnToCurrentWorkspace: () => {
    set({ showPicker: false });
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
