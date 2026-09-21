import { invoke } from "@tauri-apps/api/core";
import type { ChatAttachment, SqlAgentHistoryDetail, SqlAgentHistorySummary, SqlAgentSessionInfo } from "../types/bindings";

/** IPC 边界：SQL Agent 会话（`sql::agent`）——和 `codingService.ts` 一一对应，
 * 只是把 `workspaceId` 换成 `dataSourceId`。 */
export const sqlAgentService = {
  start(dataSourceId: string, providerId: string): Promise<SqlAgentSessionInfo> {
    return invoke("sql_agent_start", { dataSourceId, providerId });
  },
  newSession(dataSourceId: string, providerId: string): Promise<SqlAgentSessionInfo> {
    return invoke("sql_agent_new_session", { dataSourceId, providerId });
  },
  closeSession(dataSourceId: string): Promise<void> {
    return invoke("sql_agent_close", { dataSourceId });
  },
  setProvider(dataSourceId: string, providerId: string): Promise<void> {
    return invoke("sql_agent_set_provider", { dataSourceId, providerId });
  },
  sendMessage(dataSourceId: string, text: string, attachments: ChatAttachment[] = []): Promise<string> {
    return invoke("sql_agent_send_message", { dataSourceId, text, attachments });
  },
  cancelTurn(dataSourceId: string): Promise<void> {
    return invoke("sql_agent_cancel_turn", { dataSourceId });
  },
  resolveConfirm(requestId: string, allow: boolean): Promise<void> {
    return invoke("sql_agent_resolve_confirm", { requestId, allow });
  },
  answerQuestion(requestId: string, answer: string): Promise<void> {
    return invoke("sql_agent_answer_question", { requestId, answer });
  },
  historyList(dataSourceId: string): Promise<SqlAgentHistorySummary[]> {
    return invoke("sql_agent_history_list", { dataSourceId });
  },
  historyGet(id: string): Promise<SqlAgentHistoryDetail | null> {
    return invoke("sql_agent_history_get", { id });
  },
  historyResume(dataSourceId: string, historyId: string): Promise<SqlAgentSessionInfo> {
    return invoke("sql_agent_history_resume", { dataSourceId, historyId });
  },
  historySave(input: Record<string, unknown>): Promise<void> {
    return invoke("sql_agent_history_save", { input });
  },
  historyRename(id: string, title: string): Promise<void> {
    return invoke("sql_agent_history_rename", { id, title });
  },
  historyDelete(id: string): Promise<void> {
    return invoke("sql_agent_history_delete", { id });
  },
};
