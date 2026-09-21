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
  /** 拉取某个 Provider 实际支持的模型列表（OpenAI 兼容的 `GET /models`）。不缓存，
   * 每次调用都是一次真实请求——调用方（CodingAgentPanel）负责按"每次 AI 会话
   * 启动/切换 Provider 时重新拉取一遍"的策略来调它，不是无限期复用上次结果。 */
  listModels(id: string): Promise<string[]> {
    return invoke("ai_provider_list_models", { id });
  },
  /** 返回 requestId；增量文本经 ai:chat-chunk/ai:chat-done/ai:chat-error 事件推送。*/
  send(providerId: string, messages: ChatMessage[], redactEnabled: boolean, webSearchEnabled: boolean): Promise<string> {
    return invoke("ai_chat_send", { providerId, messages, redactEnabled, webSearchEnabled });
  },
  cancel(requestId: string): Promise<boolean> {
    return invoke("ai_chat_cancel", { requestId });
  },
};
