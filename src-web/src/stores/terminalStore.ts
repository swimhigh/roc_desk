import { create } from "zustand";
import { sshService } from "../services/sshService";
import { ptyService } from "../services/ptyService";

export interface TerminalTab {
  id: string; // channelId，同时用作 React key
  kind: "ssh" | "local";
  profileId?: string; // kind === 'ssh' 时必填
  title: string;
  /** Channel 已经断开（远端 shell 退出、连接中断等）——不是"用户主动关闭"，
   * 是被动检测到的，UI 上要和正常的 Tab 区分开并提供重连入口。*/
  disconnected?: boolean;
}

interface TerminalState {
  tabs: TerminalTab[];
  activeId: string | null;
  panelOpen: boolean;
  panelHeight: number;

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
  reset: () => void;
}

export const useTerminalStore = create<TerminalState>((set, get) => ({
  tabs: [],
  activeId: null,
  panelOpen: false,
  panelHeight: 260,

  openTerminal: async (target) => {
    const channelId =
      target.kind === "ssh"
        ? await sshService.openShell(target.profileId, 24, 80, target.cwd)
        : await ptyService.open(target.cwd ?? "", 24, 80);
    const seq = get().tabs.length + 1;
    const tab: TerminalTab =
      target.kind === "ssh"
        ? { id: channelId, kind: "ssh", profileId: target.profileId, title: `终端 ${seq}` }
        : { id: channelId, kind: "local", title: `终端 ${seq}` };
    set((s) => ({ tabs: [...s.tabs, tab], activeId: channelId, panelOpen: true }));
    return channelId;
  },

  closeTerminal: async (id) => {
    const tab = get().tabs.find((t) => t.id === id);
    if (!tab) return;
    set((s) => {
      const tabs = s.tabs.filter((t) => t.id !== id);
      const activeId = s.activeId === id ? (tabs[tabs.length - 1]?.id ?? null) : s.activeId;
      return { tabs, activeId, panelOpen: tabs.length === 0 ? false : s.panelOpen };
    });
    try {
      if (tab.kind === "ssh") {
        await sshService.closeChannel(tab.profileId!, id);
      } else {
        await ptyService.close(id);
      }
    } catch (e) {
      console.error("关闭终端失败", e);
    }
  },

  writeToTerminal: async (id, text) => {
    const tab = get().tabs.find((t) => t.id === id);
    if (!tab) return;
    const bytes = new TextEncoder().encode(text);
    if (tab.kind === "ssh") {
      await sshService.write(tab.profileId!, id, bytes);
    } else {
      await ptyService.write(id, bytes);
    }
  },

  markDisconnected: (id) =>
    set((s) => ({ tabs: s.tabs.map((t) => (t.id === id ? { ...t, disconnected: true } : t)) })),

  reconnectTerminal: async (id, cwd) => {
    const tab = get().tabs.find((t) => t.id === id);
    if (!tab) return;
    const newChannelId =
      tab.kind === "ssh" ? await sshService.openShell(tab.profileId!, 24, 80, cwd) : await ptyService.open(cwd ?? "", 24, 80);
    set((s) => ({
      tabs: s.tabs.map((t) => (t.id === id ? { ...t, id: newChannelId, disconnected: false } : t)),
      activeId: s.activeId === id ? newChannelId : s.activeId,
    }));
  },

  setActive: (id) => set({ activeId: id, panelOpen: true }),
  setPanelOpen: (open) => set({ panelOpen: open }),
  togglePanel: () => set((s) => ({ panelOpen: !s.panelOpen })),
  setPanelHeight: (h) => set({ panelHeight: Math.max(140, Math.min(h, 640)) }),
  reset: () => set({ tabs: [], activeId: null, panelOpen: false }),
}));
