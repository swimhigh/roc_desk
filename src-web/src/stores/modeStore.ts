import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";

/** 四种工作模块（`docs/HOME_MODES_DESIGN.md` §3.2）；`explorer` 尚未实现，
 * 首页只给占位卡片，不会真的作为 `--mode` 传给 `spawn_module_window`。 */
export type WorkMode = "ssh" | "workspace" | "editor" | "explorer";

interface LaunchContext {
  mode: string | null;
  open: string | null;
}

interface ModeState {
  /** 这个进程冷启动时的角色——`null` 表示首页/启动器，其它值表示某个模块窗口。
   * 在 `loadLaunchContext` 完成之前是 `undefined`，用来和"确定是首页"区分开，
   * 避免刚挂载的一瞬间先闪一下首页再跳到真正的模块内容。 */
  mode: WorkMode | null | undefined;
  open: string | null;
  loadLaunchContext: () => Promise<void>;
  /** 首页点开一个模块 / 模块窗口内"在新窗口打开"都调这个——见
   * `commands::launcher::spawn_module_window` 的文档注释，内部是另起一个真正
   * 独立的 `roc_desk.exe` 子进程，不在当前窗口里做任何渲染切换。 */
  spawnModule: (mode: WorkMode, open?: string) => Promise<void>;
  /** 模块窗口标题栏"返回首页"——不是切回本进程内的某棵子树，是唤起（或聚焦
   * 已经开着的）启动器进程；点了之后当前这个模块窗口本身保持不变，继续开着。 */
  goHome: () => Promise<void>;
}

export const useModeStore = create<ModeState>((set) => ({
  mode: undefined,
  open: null,

  loadLaunchContext: async () => {
    const ctx = await invoke<LaunchContext>("get_launch_context");
    set({ mode: (ctx.mode as WorkMode | null) ?? null, open: ctx.open ?? null });
  },

  spawnModule: async (mode, open) => {
    await invoke("spawn_module_window", { mode, open: open ?? null });
  },

  goHome: async () => {
    await invoke("spawn_module_window", { mode: null, open: null });
  },
}));
