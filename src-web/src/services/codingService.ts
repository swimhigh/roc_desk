import { invoke } from "@tauri-apps/api/core";
import type { ChatAttachment, CodingHistoryDetail, CodingHistorySummary, CodingMode, CodingSessionInfo, FileSyncInfo } from "../types/bindings";

/** IPC 边界（CODE_DESIGN.md §一分层原则）：AI 编程助手会话（DESIGN.md §3.8）。*/
export const codingService = {
  start(workspaceId: string, providerId: string): Promise<CodingSessionInfo> {
    return invoke("coding_start", { workspaceId, providerId });
  },
  newSession(workspaceId: string, providerId: string): Promise<CodingSessionInfo> {
    return invoke("coding_new_session", { workspaceId, providerId });
  },
  /** 释放一个工作区的常驻会话（有界保活的 LRU 淘汰时调用，见 codingStore.switchWorkspace）。*/
  closeSession(workspaceId: string): Promise<void> {
    return invoke("coding_close", { workspaceId });
  },
  setMode(workspaceId: string, mode: CodingMode): Promise<void> {
    return invoke("coding_set_mode", { workspaceId, mode });
  },
  setProvider(workspaceId: string, providerId: string): Promise<void> {
    return invoke("coding_set_provider", { workspaceId, providerId });
  },
  setAutoAllowReadonly(workspaceId: string, enabled: boolean): Promise<void> {
    return invoke("coding_set_auto_allow_readonly", { workspaceId, enabled });
  },
  setAutoGitCommit(workspaceId: string, enabled: boolean): Promise<void> {
    return invoke("coding_set_auto_git_commit", { workspaceId, enabled });
  },
  /** "完全授权模式"：开启后 AI 提出的文件改动直接落盘，且跳过命令确认。 */
  setFullAuto(workspaceId: string, enabled: boolean): Promise<void> {
    return invoke("coding_set_full_auto", { workspaceId, enabled });
  },
  /** 文件改动自动应用（默认开启，关掉退回"每条手动点应用"），不影响命令确认。 */
  setAutoApplyChanges(workspaceId: string, enabled: boolean): Promise<void> {
    return invoke("coding_set_auto_apply_changes", { workspaceId, enabled });
  },
  sendMessage(workspaceId: string, text: string, attachments?: ChatAttachment[]): Promise<string> {
    return invoke("coding_send_message", { workspaceId, text, attachments: attachments?.length ? attachments : null });
  },
  /** AI 正在处理上一条消息时用户又发了一条——不等当前这一轮工具循环结束，直接
   * 攒进后端一张独立的待注入队列，下一次模型请求前生效（不是打断正在跑的这
   * 次请求）。返回很快（不等 AI 给出新回复），前端负责乐观地把消息插进时间线。 */
  injectMessage(workspaceId: string, text: string, attachments?: ChatAttachment[]): Promise<void> {
    return invoke("coding_inject_message", { workspaceId, text, attachments: attachments?.length ? attachments : null });
  },
  /** "停止"按钮：中断当前正在跑的对话轮次（2026-09 用户反馈：中转过载时一轮
   * 对话能卡一两分钟，之前没有办法主动打断）。不依赖 `sendMessage` 是否已经
   * 返回——`coding_cancel_turn` 走独立的取消信号，不用等会话锁。 */
  cancelTurn(workspaceId: string): Promise<void> {
    return invoke("coding_cancel_turn", { workspaceId });
  },
  /** "优化输入"：用当前会话绑定的 Provider 把草稿改写一遍，不进入对话历史。 */
  optimizePrompt(workspaceId: string, text: string): Promise<string> {
    return invoke("coding_optimize_prompt", { workspaceId, text });
  },
  acceptChange(workspaceId: string, changeId: string): Promise<FileSyncInfo> {
    return invoke("coding_accept_change", { workspaceId, changeId });
  },
  rejectChange(workspaceId: string, changeId: string): Promise<void> {
    return invoke("coding_reject_change", { workspaceId, changeId });
  },
  undoChange(workspaceId: string, changeId: string): Promise<FileSyncInfo> {
    return invoke("coding_undo_change", { workspaceId, changeId });
  },
  redoChange(workspaceId: string): Promise<FileSyncInfo | null> {
    return invoke("coding_redo_change", { workspaceId });
  },
  /** 撤销某一轮对话里 AI 做出的全部已应用改动（参考 Cursor/Windsurf 的按轮次
   * 整体撤销，但不依赖 git）。 */
  revertTurn(workspaceId: string, turnId: string): Promise<FileSyncInfo[]> {
    return invoke("coding_revert_turn", { workspaceId, turnId });
  },
  confirmCommand(requestId: string, allow: boolean): Promise<void> {
    return invoke("coding_confirm_command", { requestId, allow });
  },
  answerQuestion(requestId: string, answer: string): Promise<void> {
    return invoke("coding_answer_question", { requestId, answer });
  },
  historyList(workspaceId: string): Promise<CodingHistorySummary[]> {
    return invoke("coding_history_list", { workspaceId });
  },
  historyGet(id: string): Promise<CodingHistoryDetail | null> {
    return invoke("coding_history_get", { id });
  },
  /** 打开一条历史记录并真正接续对话（不是只读回放）——用真实的 LLM 消息上下文
   * 重建这个工作区的活跃会话。 */
  historyResume(workspaceId: string, historyId: string): Promise<CodingSessionInfo> {
    return invoke("coding_history_resume", { workspaceId, historyId });
  },
  historySave(input: Record<string, unknown>): Promise<void> {
    return invoke("coding_history_save", { input });
  },
  historyRename(id: string, title: string): Promise<void> {
    return invoke("coding_history_rename", { id, title });
  },
  historyDelete(id: string): Promise<void> {
    return invoke("coding_history_delete", { id });
  },
};
