import { create } from "zustand";
import { sshService } from "../services/sshService";
import { ptyService } from "../services/ptyService";

export interface TerminalTab {
  id: string; // channelId，同时用作 React key
  /** 这个 Tab 属于哪个工作区——有界保活（见下）要求同一时刻可能有多个工作区的
   * Tab 同时存在于 `allTabs` 里，必须能按工作区过滤/分组。可选是因为
   * `TerminalView`/`TerminalTab` 这个类型也被"远程工具模式"（`RemoteTool/HomeShell.tsx`，
   * 用的是完全独立的 `remoteSessionStore`，没有"工作区"概念）复用来标注 props 类型，
   * 那边的 Tab 从来不经过本 store，天然没有也用不到这个字段；本 store 自己产出的
   * Tab（`openTerminal` 等）总是会填上。 */
  workspaceId?: string;
  kind: "ssh" | "local";
  profileId?: string; // kind === 'ssh' 时必填
  title: string;
  /** Channel 已经断开（远端 shell 退出、连接中断等）——不是"用户主动关闭"，
   * 是被动检测到的，UI 上要和正常的 Tab 区分开并提供重连入口。*/
  disconnected?: boolean;
}

/** 有界保活的 LRU 上限——同时"活着"（后端 PTY 进程/SSH Channel 不被关闭，前端
 * xterm.js 实例不被销毁）的工作区数量。和 `codingStore.ts` 的同名常量各自独立
 * 维护，不共享同一份状态，因为终端 Channel 和 AI 会话是两类不同的后端资源。 */
const MAX_RESIDENT_WORKSPACES = 3;

interface TerminalState {
  /** 当前正在显示的工作区 id；null 表示还没有任何工作区切换过。 */
  currentWorkspaceId: string | null;
  /** 所有仍保活（未被 LRU 淘汰）的工作区的全部 Tab——不只是当前显示的工作区。
   * `TerminalPanel` 必须把这里的每一个 Tab 都渲染出来（只是非当前工作区/非
   * 激活 Tab 的用 CSS 隐藏，不能从渲染树里摘掉），否则对应的 `TerminalView`
   * 组件会被卸载，xterm.js 实例连同 scrollback 一起销毁——切回来看到的会是
   * 一个全新的空终端，这是"保活"名不副实的真实 bug（后端 Channel 虽然还活着，
   * 但前端渲染状态早就没了，后端也不会替你重放历史输出）。 */
  allTabs: TerminalTab[];
  /** 每个工作区各自的"当前激活 Tab"，工作区之间互不影响。 */
  activeIdByWorkspace: Record<string, string | null>;
  /** 当前工作区的 Tab 列表/激活 Tab——从 `allTabs`/`activeIdByWorkspace` 按
   * `currentWorkspaceId` 过滤出来，每次相关状态变化时同步更新，供大多数只关心
   * "当前工作区"的调用方（Explorer 右键运行脚本、Tab 栏渲染）直接使用，不用
   * 自己再 filter 一遍。真正需要跨工作区渲染保活内容的只有 `TerminalPanel`。 */
  tabs: TerminalTab[];
  activeId: string | null;
  panelOpen: boolean;
  panelHeight: number;
  /** 最近使用的工作区 id，最前面的最新；判断 LRU 淘汰顺序用。 */
  residentOrder: string[];

  /** 新开一个终端 Tab（VS Code 风格：底部面板 + 多终端），返回新 Tab 的 id。
   * `cwd` 默认就是当前工作区根目录——参考 VS Code 打开项目后集成终端自动进到项目目录。*/
  openTerminal: (target: { kind: "ssh"; profileId: string; cwd?: string } | { kind: "local"; cwd?: string }) => Promise<string>;
  closeTerminal: (id: string) => Promise<void>;
  /** 把一段文本写进某个终端（供 Explorer 右键"运行脚本"等场景复用，不需要终端已经
   * focus 或可见——数据经后端 emit 回来，已挂载的 TerminalView 自然会显示）。*/
  writeToTerminal: (id: string, text: string) => Promise<void>;
  markDisconnected: (id: string) => void;
  /** 断线后重连：原 Channel 已经死了没法复活，实际是"开一个新 Channel 顶替原位置"——
   * Tab 的 id 会变（重新用它当 React key，旧的 xterm 实例连同断线消息一起被替换），
   * 但在用户看来还是同一个 Tab 位置、同一个标题。*/
  reconnectTerminal: (id: string, cwd?: string) => Promise<void>;
  setActive: (id: string) => void;
  setPanelOpen: (open: boolean) => void;
  togglePanel: () => void;
  setPanelHeight: (h: number) => void;
  /** 切换到某个工作区的终端面板：只是把"当前显示哪个工作区"这根指针挪过去，
   * 不销毁任何东西——目标工作区名下的 Tab 本来就一直在 `allTabs` 里、对应的
   * `TerminalView` 也一直挂载着（只是之前被 CSS 隐藏）。超过
   * `MAX_RESIDENT_WORKSPACES` 时淘汰最久未用的一个——真正关闭它名下所有
   * PTY/SSH Channel 并从 `allTabs` 摘除（这时对应 `TerminalView` 才会真正卸载）。 */
  switchWorkspace: (workspaceId: string) => Promise<void>;
  /** 淘汰一个工作区的保活终端：逐个关闭它名下的 PTY/SSH Channel 释放后端资源，
   * 并把它的 Tab 从 `allTabs` 里摘除（对应 `TerminalView` 这时才卸载）。 */
  evictWorkspace: (workspaceId: string) => Promise<void>;
  reset: () => void;
}

async function closeBackendChannel(tab: TerminalTab): Promise<void> {
  try {
    if (tab.kind === "ssh") {
      await sshService.closeChannel(tab.profileId!, tab.id);
    } else {
      await ptyService.close(tab.id);
    }
  } catch (e) {
    console.error("释放终端失败", e);
  }
}

export const useTerminalStore = create<TerminalState>((set, get) => ({
  currentWorkspaceId: null,
  allTabs: [],
  activeIdByWorkspace: {},
  tabs: [],
  activeId: null,
  panelOpen: false,
  panelHeight: 260,
  residentOrder: [],

  openTerminal: async (target) => {
    const workspaceId = get().currentWorkspaceId;
    if (!workspaceId) throw new Error("没有当前工作区，无法新建终端");
    const channelId =
      target.kind === "ssh"
        ? await sshService.openShell(target.profileId, 24, 80, target.cwd)
        : await ptyService.open(target.cwd ?? "", 24, 80);
    const seq = get().allTabs.filter((t) => t.workspaceId === workspaceId).length + 1;
    const tab: TerminalTab =
      target.kind === "ssh"
        ? { id: channelId, workspaceId, kind: "ssh", profileId: target.profileId, title: `终端 ${seq}` }
        : { id: channelId, workspaceId, kind: "local", title: `终端 ${seq}` };
    set((s) => {
      const allTabs = [...s.allTabs, tab];
      return {
        allTabs,
        activeIdByWorkspace: { ...s.activeIdByWorkspace, [workspaceId]: channelId },
        tabs: allTabs.filter((t) => t.workspaceId === s.currentWorkspaceId),
        activeId: channelId,
        panelOpen: true,
      };
    });
    return channelId;
  },

  closeTerminal: async (id) => {
    const tab = get().allTabs.find((t) => t.id === id);
    if (!tab) return;
    set((s) => {
      const allTabs = s.allTabs.filter((t) => t.id !== id);
      const siblingTabs = allTabs.filter((t) => t.workspaceId === tab.workspaceId);
      const activeIdByWorkspace = { ...s.activeIdByWorkspace };
      if (activeIdByWorkspace[tab.workspaceId!] === id) {
        activeIdByWorkspace[tab.workspaceId!] = siblingTabs[siblingTabs.length - 1]?.id ?? null;
      }
      return {
        allTabs,
        activeIdByWorkspace,
        tabs: allTabs.filter((t) => t.workspaceId === s.currentWorkspaceId),
        activeId: activeIdByWorkspace[s.currentWorkspaceId ?? ""] ?? null,
        panelOpen: s.currentWorkspaceId === tab.workspaceId && siblingTabs.length === 0 ? false : s.panelOpen,
      };
    });
    await closeBackendChannel(tab);
  },

  writeToTerminal: async (id, text) => {
    const tab = get().allTabs.find((t) => t.id === id);
    if (!tab) return;
    const bytes = new TextEncoder().encode(text);
    if (tab.kind === "ssh") {
      await sshService.write(tab.profileId!, id, bytes);
    } else {
      await ptyService.write(id, bytes);
    }
  },

  markDisconnected: (id) =>
    set((s) => {
      const allTabs = s.allTabs.map((t) => (t.id === id ? { ...t, disconnected: true } : t));
      return { allTabs, tabs: allTabs.filter((t) => t.workspaceId === s.currentWorkspaceId) };
    }),

  reconnectTerminal: async (id, cwd) => {
    const tab = get().allTabs.find((t) => t.id === id);
    if (!tab) return;
    const newChannelId =
      tab.kind === "ssh" ? await sshService.openShell(tab.profileId!, 24, 80, cwd) : await ptyService.open(cwd ?? "", 24, 80);
    set((s) => {
      const allTabs = s.allTabs.map((t) => (t.id === id ? { ...t, id: newChannelId, disconnected: false } : t));
      const activeIdByWorkspace = { ...s.activeIdByWorkspace };
      if (activeIdByWorkspace[tab.workspaceId!] === id) activeIdByWorkspace[tab.workspaceId!] = newChannelId;
      return {
        allTabs,
        activeIdByWorkspace,
        tabs: allTabs.filter((t) => t.workspaceId === s.currentWorkspaceId),
        activeId: activeIdByWorkspace[s.currentWorkspaceId ?? ""] ?? null,
      };
    });
  },

  setActive: (id) =>
    set((s) => {
      if (!s.currentWorkspaceId) return {};
      return { activeIdByWorkspace: { ...s.activeIdByWorkspace, [s.currentWorkspaceId]: id }, activeId: id, panelOpen: true };
    }),
  setPanelOpen: (open) => set({ panelOpen: open }),
  togglePanel: () => set((s) => ({ panelOpen: !s.panelOpen })),
  setPanelHeight: (h) => set({ panelHeight: Math.max(140, Math.min(h, 640)) }),

  switchWorkspace: async (workspaceId) => {
    const prev = get();
    if (prev.currentWorkspaceId === workspaceId) return;

    // LRU：目标工作区提到最前，超出上限的从尾部淘汰——注意这里不做任何"保存/
    // 恢复快照"的动作，目标工作区的 Tab 本来就一直在 `allTabs` 里，对应的
    // TerminalView 也一直挂载着，切换只是换一下"当前显示哪个工作区"这根指针。
    const order = [workspaceId, ...prev.residentOrder.filter((id) => id !== workspaceId)];
    const toEvict: string[] = [];
    while (order.length > MAX_RESIDENT_WORKSPACES) {
      const victim = order.pop();
      if (victim) toEvict.push(victim);
    }
    set({ residentOrder: order });
    for (const id of toEvict) {
      await get().evictWorkspace(id);
    }

    set((s) => {
      const tabs = s.allTabs.filter((t) => t.workspaceId === workspaceId);
      const activeId = s.activeIdByWorkspace[workspaceId] ?? null;
      return { currentWorkspaceId: workspaceId, tabs, activeId, panelOpen: tabs.length > 0 };
    });
  },

  evictWorkspace: async (workspaceId) => {
    const victimTabs = get().allTabs.filter((t) => t.workspaceId === workspaceId);
    set((s) => {
      const allTabs = s.allTabs.filter((t) => t.workspaceId !== workspaceId);
      const activeIdByWorkspace = { ...s.activeIdByWorkspace };
      delete activeIdByWorkspace[workspaceId];
      const isCurrent = s.currentWorkspaceId === workspaceId;
      return {
        allTabs,
        activeIdByWorkspace,
        tabs: isCurrent ? [] : s.tabs,
        activeId: isCurrent ? null : s.activeId,
        residentOrder: s.residentOrder.filter((id) => id !== workspaceId),
      };
    });
    for (const tab of victimTabs) {
      await closeBackendChannel(tab);
    }
  },

  reset: () =>
    set({
      currentWorkspaceId: null,
      allTabs: [],
      activeIdByWorkspace: {},
      tabs: [],
      activeId: null,
      panelOpen: false,
      residentOrder: [],
    }),
}));
