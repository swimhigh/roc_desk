import { create } from "zustand";
import { listen } from "@tauri-apps/api/event";
import { connectionService } from "../services/connectionService";
import { sshService } from "../services/sshService";
import type { ConnectionProfile, HostKeyPromptEvent } from "../types/bindings";

interface ConnectionState {
  profiles: ConnectionProfile[];
  sessionStatus: Record<string, "connecting" | "connected" | "reconnecting" | "disconnected">;
  pendingHostKeyPrompt: HostKeyPromptEvent | null;

  loadProfiles: () => Promise<void>;
  resolveHostKeyPrompt: (trust: boolean) => Promise<void>;
}

export const useConnectionStore = create<ConnectionState>((set, get) => ({
  profiles: [],
  sessionStatus: {},
  pendingHostKeyPrompt: null,

  loadProfiles: async () => {
    const profiles = await connectionService.list();
    set({ profiles });
  },

  resolveHostKeyPrompt: async (trust) => {
    const prompt = get().pendingHostKeyPrompt;
    if (!prompt) return;
    await sshService.confirmHostKey(prompt.requestId, trust);
    set({ pendingHostKeyPrompt: null });
  },
}));

/**
 * 全局监听主机指纹确认请求（DESIGN.md §3.2.1），只需在应用启动时注册一次
 * （main.tsx 里调用），弹窗渲染见 components/ConnectionManager/HostKeyPromptHost.tsx。
 */
export function registerHostKeyPromptListener() {
  return listen<HostKeyPromptEvent>("ssh:host-key-prompt", (event) => {
    useConnectionStore.setState({ pendingHostKeyPrompt: event.payload });
  });
}
