import { invoke } from "@tauri-apps/api/core";
import type { CodingMode, CodingSessionInfo } from "../types/bindings";

/** IPC 边界（CODE_DESIGN.md §一分层原则）：AI 编程助手会话（DESIGN.md §3.8）。*/
export const codingService = {
  start(workspaceId: string, providerId: string): Promise<CodingSessionInfo> {
    return invoke("coding_start", { workspaceId, providerId });
  },
  setMode(workspaceId: string, mode: CodingMode): Promise<void> {
    return invoke("coding_set_mode", { workspaceId, mode });
  },
  setAutoAllowReadonly(workspaceId: string, enabled: boolean): Promise<void> {
    return invoke("coding_set_auto_allow_readonly", { workspaceId, enabled });
  },
  setAutoGitCommit(workspaceId: string, enabled: boolean): Promise<void> {
    return invoke("coding_set_auto_git_commit", { workspaceId, enabled });
  },
  sendMessage(workspaceId: string, text: string): Promise<string> {
    return invoke("coding_send_message", { workspaceId, text });
  },
  acceptChange(workspaceId: string, changeId: string): Promise<void> {
    return invoke("coding_accept_change", { workspaceId, changeId });
  },
  rejectChange(workspaceId: string, changeId: string): Promise<void> {
    return invoke("coding_reject_change", { workspaceId, changeId });
  },
  undoChange(workspaceId: string, changeId: string): Promise<void> {
    return invoke("coding_undo_change", { workspaceId, changeId });
  },
  redoChange(workspaceId: string): Promise<string | null> {
    return invoke("coding_redo_change", { workspaceId });
  },
  confirmCommand(requestId: string, allow: boolean): Promise<void> {
    return invoke("coding_confirm_command", { requestId, allow });
  },
};
