import { create } from "zustand";
import { sshService } from "../services/sshService";
import type { ConnectionProfile } from "../types/bindings";

export type RemoteSessionTab =
  | { id: string; kind: "ssh-terminal"; profileId: string; title: string; disconnected?: boolean }
  | { id: string; kind: "sftp"; profileId: string; title: string }
  | { id: string; kind: "rdp"; profileId: string; title: string };

interface RemoteSessionState {
  tabs: RemoteSessionTab[];
  activeId: string | null;
  /** 多路执行模式（参考 MobaXterm 的 MultiExec）：开着的时候，任意一个未被排除的
   * SSH 终端里敲的字符会广播给其它所有未被排除的 SSH 终端。只影响输入广播，
   * 不影响标签是否显示——排除的终端照样能看、能单独操作，只是不接收广播。*/
  multiExecEnabled: boolean;
  multiExecExcluded: Set<string>;

  /** 打开一个终端/SFTP/RDP 标签。总是新开一个（和现有 terminalStore 的"+"习惯一致，
   * 不做"同一个连接已经开过就复用"的去重——用户点几次就要几个独立标签，SSH 场景下
   * "多开一个终端到同一台机器"是常见操作，不是误操作）。*/
  openSshTerminal: (profile: ConnectionProfile) => Promise<string>;
  openSftp: (profile: ConnectionProfile) => string;
  openRdp: (profile: ConnectionProfile) => string;
  closeTab: (id: string) => Promise<void>;
  setActive: (id: string) => void;
  markDisconnected: (id: string) => void;
  /** 断线重连：和 terminalStore.reconnectTerminal 同款逻辑——原 Channel 死了没法复活，
   * 开一个新 Channel 顶替原位置，Tab 的 id 会变但用户看来还是同一个 Tab。*/
  reconnectSshTerminal: (id: string) => Promise<void>;
  toggleMultiExec: () => void;
  toggleExcludedFromMultiExec: (id: string) => void;
  /** `sourceTabId` 自己已经在 TerminalView 里正常写过一次了（用户直接敲的那个终端），
   * 这里只转发给其它未排除的终端，不重复发给自己，否则命令会在源终端里被执行两次。*/
  broadcastInput: (sourceTabId: string, data: Uint8Array) => void;
  reset: () => void;
}

let seq = 0;
function nextId(prefix: string): string {
  seq += 1;
  return `${prefix}-${Date.now()}-${seq}`;
}

export const useRemoteSessionStore = create<RemoteSessionState>((set, get) => ({
  tabs: [],
  activeId: null,
  multiExecEnabled: false,
  multiExecExcluded: new Set(),

  openSshTerminal: async (profile) => {
    await sshService.connect(profile.id);
    const count = get().tabs.filter((t) => t.kind === "ssh-terminal" && t.profileId === profile.id).length;
    const channelId = await sshService.openShell(profile.id, 24, 80);
    const tab: RemoteSessionTab = {
      id: channelId,
      kind: "ssh-terminal",
      profileId: profile.id,
      title: count > 0 ? `${profile.name} (${count + 1})` : profile.name,
    };
    set((s) => ({ tabs: [...s.tabs, tab], activeId: channelId }));
    return channelId;
  },

  openSftp: (profile) => {
    const id = nextId("sftp");
    const tab: RemoteSessionTab = { id, kind: "sftp", profileId: profile.id, title: `${profile.name} · SFTP` };
    set((s) => ({ tabs: [...s.tabs, tab], activeId: id }));
    return id;
  },

  openRdp: (profile) => {
    const id = nextId("rdp");
    const tab: RemoteSessionTab = { id, kind: "rdp", profileId: profile.id, title: `${profile.name} · RDP` };
    set((s) => ({ tabs: [...s.tabs, tab], activeId: id }));
    return id;
  },

  closeTab: async (id) => {
    const tab = get().tabs.find((t) => t.id === id);
    if (!tab) return;
    set((s) => {
      const tabs = s.tabs.filter((t) => t.id !== id);
      const activeId = s.activeId === id ? (tabs[tabs.length - 1]?.id ?? null) : s.activeId;
      return { tabs, activeId };
    });
    if (tab.kind === "ssh-terminal") {
      try {
        await sshService.closeChannel(tab.profileId, id);
      } catch (e) {
        console.error("关闭终端失败", e);
      }
    }
  },

  setActive: (id) => set({ activeId: id }),

  markDisconnected: (id) =>
    set((s) => ({ tabs: s.tabs.map((t) => (t.id === id && t.kind === "ssh-terminal" ? { ...t, disconnected: true } : t)) })),

  reconnectSshTerminal: async (id) => {
    const tab = get().tabs.find((t) => t.id === id);
    if (!tab || tab.kind !== "ssh-terminal") return;
    const newChannelId = await sshService.openShell(tab.profileId, 24, 80);
    set((s) => ({
      tabs: s.tabs.map((t) => (t.id === id ? { ...t, id: newChannelId, disconnected: false } : t)),
      activeId: s.activeId === id ? newChannelId : s.activeId,
    }));
  },

  toggleMultiExec: () => set((s) => ({ multiExecEnabled: !s.multiExecEnabled })),

  toggleExcludedFromMultiExec: (id) =>
    set((s) => {
      const excluded = new Set(s.multiExecExcluded);
      if (excluded.has(id)) excluded.delete(id);
      else excluded.add(id);
      return { multiExecExcluded: excluded };
    }),

  broadcastInput: (sourceTabId, data) => {
    const { tabs, multiExecEnabled, multiExecExcluded } = get();
    if (!multiExecEnabled) return;
    for (const tab of tabs) {
      if (tab.kind !== "ssh-terminal") continue;
      if (tab.id === sourceTabId || tab.disconnected) continue;
      if (multiExecExcluded.has(tab.id)) continue;
      void sshService.write(tab.profileId, tab.id, data);
    }
  },

  reset: () => set({ tabs: [], activeId: null, multiExecEnabled: false, multiExecExcluded: new Set() }),
}));
