import { create } from "zustand";
import { listen } from "@tauri-apps/api/event";
import { aiChatService } from "../services/aiChatService";
import { formatError } from "../utils/error";
import type {
  AiChatChunkEvent,
  AiChatDoneEvent,
  AiChatErrorEvent,
  AiProvider,
  AiProviderInput,
  ChatMessage,
} from "../types/bindings";

export interface DisplayMessage extends ChatMessage {
  id: string;
  streaming?: boolean;
  error?: string;
}

interface AiChatState {
  providers: AiProvider[];
  activeProviderId: string | null;
  messages: DisplayMessage[];
  redactEnabled: boolean;
  webSearchEnabled: boolean;
  sending: boolean;
  error: string | null;
  pendingRequestId: string | null;

  loadProviders: () => Promise<void>;
  createProvider: (input: AiProviderInput) => Promise<void>;
  updateProvider: (id: string, input: AiProviderInput) => Promise<void>;
  deleteProvider: (id: string) => Promise<void>;
  setActiveProvider: (id: string) => void;
  setRedactEnabled: (v: boolean) => void;
  setWebSearchEnabled: (v: boolean) => void;
  sendMessage: (text: string) => Promise<void>;
  clearChat: () => void;
}

let listenersRegistered = false;

export const useAiChatStore = create<AiChatState>((set, get) => ({
  providers: [],
  activeProviderId: null,
  messages: [],
  redactEnabled: true,
  webSearchEnabled: true,
  sending: false,
  error: null,
  pendingRequestId: null,

  loadProviders: async () => {
    try {
      const providers = await aiChatService.listProviders();
      set((s) => ({
        providers,
        activeProviderId: s.activeProviderId && providers.some((p) => p.id === s.activeProviderId)
          ? s.activeProviderId
          : providers[0]?.id ?? null,
      }));
    } catch (e) {
      set({ error: formatError(e) });
    }
  },

  createProvider: async (input) => {
    const created = await aiChatService.createProvider(input);
    set((s) => ({ providers: [...s.providers, created], activeProviderId: s.activeProviderId ?? created.id }));
  },

  updateProvider: async (id, input) => {
    const updated = await aiChatService.updateProvider(id, input);
    set((s) => ({ providers: s.providers.map((p) => (p.id === id ? updated : p)) }));
  },

  deleteProvider: async (id) => {
    await aiChatService.deleteProvider(id);
    set((s) => ({
      providers: s.providers.filter((p) => p.id !== id),
      activeProviderId: s.activeProviderId === id ? null : s.activeProviderId,
    }));
  },

  setActiveProvider: (id) => set({ activeProviderId: id }),
  setRedactEnabled: (redactEnabled) => set({ redactEnabled }),
  setWebSearchEnabled: (webSearchEnabled) => set({ webSearchEnabled }),

  sendMessage: async (text) => {
    const { activeProviderId, messages, redactEnabled, webSearchEnabled } = get();
    if (!activeProviderId || !text.trim() || get().sending) return;

    const userMsg: DisplayMessage = { id: `local-${Date.now()}`, role: "user", content: text };
    const history: ChatMessage[] = [...messages, userMsg].map(({ role, content }) => ({ role, content }));

    set((s) => ({ messages: [...s.messages, userMsg], sending: true, error: null }));

    try {
      const requestId = await aiChatService.send(activeProviderId, history, redactEnabled, webSearchEnabled);
      const assistantMsg: DisplayMessage = { id: requestId, role: "assistant", content: "", streaming: true };
      set((s) => ({ messages: [...s.messages, assistantMsg], pendingRequestId: requestId }));
    } catch (e) {
      set({ sending: false, error: formatError(e) });
    }
  },

  clearChat: () => set({ messages: [], pendingRequestId: null, sending: false, error: null }),
}));

/** 全局注册一次流式事件监听（App.tsx 挂载时调用，和 registerHostKeyPromptListener 同款模式）。*/
export function registerAiChatListeners(): Promise<() => void> {
  if (listenersRegistered) return Promise.resolve(() => {});
  listenersRegistered = true;

  const unlistenPromises = [
    listen<AiChatChunkEvent>("ai:chat-chunk", (event) => {
      const { pendingRequestId } = useAiChatStore.getState();
      if (event.payload.requestId !== pendingRequestId) return;
      useAiChatStore.setState((s) => ({
        messages: s.messages.map((m) =>
          m.id === event.payload.requestId ? { ...m, content: m.content + event.payload.delta } : m,
        ),
      }));
    }),
    listen<AiChatDoneEvent>("ai:chat-done", (event) => {
      const { pendingRequestId } = useAiChatStore.getState();
      if (event.payload.requestId !== pendingRequestId) return;
      useAiChatStore.setState((s) => ({
        sending: false,
        pendingRequestId: null,
        messages: s.messages.map((m) => (m.id === event.payload.requestId ? { ...m, streaming: false } : m)),
      }));
    }),
    listen<AiChatErrorEvent>("ai:chat-error", (event) => {
      const { pendingRequestId } = useAiChatStore.getState();
      if (event.payload.requestId !== pendingRequestId) return;
      useAiChatStore.setState((s) => ({
        sending: false,
        pendingRequestId: null,
        error: event.payload.message,
        messages: s.messages.map((m) =>
          m.id === event.payload.requestId ? { ...m, streaming: false, error: event.payload.message } : m,
        ),
      }));
    }),
  ];

  return Promise.all(unlistenPromises).then((unlistens) => () => unlistens.forEach((u) => u()));
}
