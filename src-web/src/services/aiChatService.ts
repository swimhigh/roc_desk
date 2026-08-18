import { invoke } from "@tauri-apps/api/core";
import type { AiProvider, AiProviderInput, ChatMessage } from "../types/bindings";

/** IPC 边界（CODE_DESIGN.md §一分层原则）：AI Provider 管理 + 流式对话发起。*/
export const aiChatService = {
  listProviders(): Promise<AiProvider[]> {
    return invoke("ai_provider_list");
  },
  createProvider(input: AiProviderInput): Promise<AiProvider> {
    return invoke("ai_provider_create", { input });
  },
  updateProvider(id: string, input: AiProviderInput): Promise<AiProvider> {
    return invoke("ai_provider_update", { id, input });
  },
  deleteProvider(id: string): Promise<void> {
    return invoke("ai_provider_delete", { id });
  },
  /** 返回 requestId；增量文本经 ai:chat-chunk/ai:chat-done/ai:chat-error 事件推送。*/
  send(providerId: string, messages: ChatMessage[], redactEnabled: boolean): Promise<string> {
    return invoke("ai_chat_send", { providerId, messages, redactEnabled });
  },
};
