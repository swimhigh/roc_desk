import { create } from "zustand";
import { listen } from "@tauri-apps/api/event";
import { connectionService } from "../services/connectionService";
import { sshService } from "../services/sshService";
import { agentService } from "../services/agentService";
import type { AgentCertPromptEvent, ConnectionProfile, HostKeyPromptEvent } from "../types/bindings";

interface ConnectionState {
  profiles: ConnectionProfile[];
  sessionStatus: Record<string, "connecting" | "connected" | "reconnecting" | "disconnected">;
  pendingHostKeyPrompt: HostKeyPromptEvent | null;
  pendingAgentCertPrompt: AgentCertPromptEvent | null;

  loadProfiles: () => Promise<void>;
  resolveHostKeyPrompt: (trust: boolean) => Promise<void>;
  resolveAgentCertPrompt: (trust: boolean) => Promise<void>;
}

export const useConnectionStore = create<ConnectionState>((set, get) => ({
  profiles: [],
  sessionStatus: {},
  pendingHostKeyPrompt: null,
  pendingAgentCertPrompt: null,

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

  resolveAgentCertPrompt: async (trust) => {
    const prompt = get().pendingAgentCertPrompt;
    if (!prompt) return;
    await agentService.confirmCert(prompt.requestId, trust);
    set({ pendingAgentCertPrompt: null });
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

/**
 * 和上面对称，只是监听的是 Agent TLS 证书指纹 TOFU 请求（AGENT_DESIGN.md §3.1），
 * 弹窗渲染见 components/ConnectionManager/AgentCertPromptHost.tsx。
 */
export function registerAgentCertPromptListener() {
  return listen<AgentCertPromptEvent>("agent:cert-prompt", (event) => {
    useConnectionStore.setState({ pendingAgentCertPrompt: event.payload });
  });
}
