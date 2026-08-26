import { create } from "zustand";
import { listen } from "@tauri-apps/api/event";
import { codingService } from "../services/codingService";
import { formatError } from "../utils/error";
import { useAiChatStore } from "./aiChatStore";
import type {
  CodingAssistantNoteEvent,
  CodingCommandBlockedEvent,
  CodingCommandConfirmRequestEvent,
  CodingFileChangeEvent,
  CodingGitCommitResultEvent,
  CodingMode,
  CodingSessionInfo,
  CodingToolCallEvent,
  FileChange,
  CodingHistorySummary,
} from "../types/bindings";

export type TimelineEntry =
  | { kind: "user"; id: string; text: string }
  | { kind: "assistant"; id: string; text: string }
  /** 工具调用之外顺带写的说明文字，不是最终答案——单独一种样式，和 "assistant"
   * 区分开（2026-08-18 需求，见 CodingAssistantNoteEvent 的注释）。 */
  | { kind: "note"; id: string; text: string }
  | { kind: "progress"; id: string; text: string }
  | { kind: "tool"; id: string; tool: string; running: boolean; detail?: string | null }
  | { kind: "change"; id: string; changeId: string }
  | { kind: "blocked"; id: string; command: string }
  | { kind: "git"; id: string; path: string; output: string };

interface CodingState {
  workspaceId: string | null;
  sessionInfo: CodingSessionInfo | null;
  timeline: TimelineEntry[];
  changesById: Record<string, FileChange>;
  sending: boolean;
  error: string | null;
  confirmRequest: { requestId: string; command: string; host: string | null } | null;
  histories: CodingHistorySummary[];
  viewingHistoryId: string | null;
  loadHistories: (workspaceId?: string) => Promise<void>;
  saveCurrentHistory: () => Promise<void>;
  openHistory: (id: string) => Promise<void>;
  deleteHistory: (id: string) => Promise<void>;
  renameHistory: (id: string, title: string) => Promise<void>;
  newSession: (providerId: string) => Promise<void>;
  restoreOrStart: (workspaceId: string, providerId: string) => Promise<void>;

  start: (workspaceId: string, providerId: string) => Promise<void>;
  setMode: (mode: CodingMode) => Promise<void>;
  setProvider: (providerId: string) => Promise<void>;
  setAutoAllowReadonly: (enabled: boolean) => Promise<void>;
  setAutoGitCommit: (enabled: boolean) => Promise<void>;
  sendMessage: (text: string) => Promise<void>;
  acceptChange: (changeId: string) => Promise<void>;
  rejectChange: (changeId: string) => Promise<void>;
  undoChange: (changeId: string) => Promise<void>;
  redoChange: () => Promise<void>;
  resolveConfirm: (allow: boolean) => Promise<void>;
  reset: () => void;
}

let seq = 0;
const nextId = () => `t-${++seq}`;

export const useCodingStore = create<CodingState>((set, get) => ({
  workspaceId: null,
  sessionInfo: null,
  timeline: [],
  changesById: {},
  sending: false,
  error: null,
  confirmRequest: null,
  histories: [],
  viewingHistoryId: null,

  start: async (workspaceId, providerId) => {
    set({ error: null });
    try {
      const info = await codingService.start(workspaceId, providerId);
      const changesById: Record<string, FileChange> = {};
      const timeline: TimelineEntry[] = [];
      for (const change of info.changes) {
        changesById[change.id] = change;
        timeline.push({ kind: "change", id: nextId(), changeId: change.id });
      }
      set({ workspaceId, sessionInfo: info, changesById, timeline });
      await get().loadHistories(workspaceId);
    } catch (e) {
      set({ error: formatError(e) });
    }
  },

  setMode: async (mode) => {
    const { workspaceId, sessionInfo, viewingHistoryId } = get();
    if (viewingHistoryId) return;
    if (!workspaceId || !sessionInfo) return;
    await codingService.setMode(workspaceId, mode);
    set({ sessionInfo: { ...sessionInfo, mode } });
    await get().saveCurrentHistory();
  },

  setProvider: async (providerId) => {
    const { workspaceId, sessionInfo, viewingHistoryId } = get();
    if (viewingHistoryId) return;
    if (!workspaceId || !sessionInfo) return;
    await codingService.setProvider(workspaceId, providerId);
    set({ sessionInfo: { ...sessionInfo, provider_id: providerId } });
    await get().saveCurrentHistory();
  },

  setAutoAllowReadonly: async (enabled) => {
    const { workspaceId, sessionInfo, viewingHistoryId } = get();
    if (viewingHistoryId) return;
    if (!workspaceId || !sessionInfo) return;
    await codingService.setAutoAllowReadonly(workspaceId, enabled);
    set({ sessionInfo: { ...sessionInfo, auto_allow_readonly: enabled } });
  },

  setAutoGitCommit: async (enabled) => {
    const { workspaceId, sessionInfo, viewingHistoryId } = get();
    if (viewingHistoryId) return;
    if (!workspaceId || !sessionInfo) return;
    await codingService.setAutoGitCommit(workspaceId, enabled);
    set({ sessionInfo: { ...sessionInfo, auto_git_commit: enabled } });
  },

  sendMessage: async (text) => {
    const { workspaceId, sending, viewingHistoryId } = get();
    if (!workspaceId || !text.trim() || sending || viewingHistoryId) return;
    set((s) => ({ timeline: [...s.timeline, { kind: "user", id: nextId(), text }], sending: true, error: null }));
    try {
      const reply = await codingService.sendMessage(workspaceId, text);
      set((s) => ({ timeline: [...s.timeline, { kind: "assistant", id: nextId(), text: reply }], sending: false }));
      await get().saveCurrentHistory();
    } catch (e) {
      set({ sending: false, error: formatError(e) });
    }
  },

  acceptChange: async (changeId) => {
    const { workspaceId, viewingHistoryId } = get();
    if (!workspaceId || viewingHistoryId) return;
    await codingService.acceptChange(workspaceId, changeId);
    set((s) => ({
      changesById: { ...s.changesById, [changeId]: { ...s.changesById[changeId], status: "applied" } },
    }));
    await get().saveCurrentHistory();
  },

  rejectChange: async (changeId) => {
    const { workspaceId, viewingHistoryId } = get();
    if (!workspaceId || viewingHistoryId) return;
    await codingService.rejectChange(workspaceId, changeId);
    set((s) => ({
      changesById: { ...s.changesById, [changeId]: { ...s.changesById[changeId], status: "rejected" } },
    }));
    await get().saveCurrentHistory();
  },

  undoChange: async (changeId) => {
    const { workspaceId, viewingHistoryId } = get();
    if (!workspaceId || viewingHistoryId) return;
    await codingService.undoChange(workspaceId, changeId);
    set((s) => ({
      changesById: { ...s.changesById, [changeId]: { ...s.changesById[changeId], status: "undone" } },
    }));
    await get().saveCurrentHistory();
  },

  redoChange: async () => {
    const { workspaceId, viewingHistoryId } = get();
    if (!workspaceId || viewingHistoryId) return;
    const changeId = await codingService.redoChange(workspaceId);
    if (!changeId) return;
    set((s) => ({
      changesById: { ...s.changesById, [changeId]: { ...s.changesById[changeId], status: "applied" } },
    }));
    await get().saveCurrentHistory();
  },

  resolveConfirm: async (allow) => {
    const { confirmRequest } = get();
    if (!confirmRequest) return;
    set({ confirmRequest: null });
    await codingService.confirmCommand(confirmRequest.requestId, allow);
  },

  loadHistories: async (workspaceId) => {
    const id = workspaceId ?? get().workspaceId;
    if (!id) return;
    try { set({ histories: await codingService.historyList(id) }); } catch { /* history must not block chat */ }
  },

  saveCurrentHistory: async () => {
    const { workspaceId, sessionInfo, timeline, changesById, viewingHistoryId } = get();
    if (!workspaceId || !sessionInfo || timeline.length === 0 || viewingHistoryId) return;
    const titleEntry = timeline.find((entry) => entry.kind === "user");
    const title = titleEntry && titleEntry.kind === "user" ? titleEntry.text.slice(0, 80) : "编程会话";
    const provider = useAiChatStore.getState().providers.find((item) => item.id === sessionInfo.provider_id);
    try {
      await codingService.historySave({ id: sessionInfo.id, workspace_id: workspaceId, title, provider_id: sessionInfo.provider_id, provider_label: provider?.name ?? "AI Provider", model: provider?.model ?? "", mode: sessionInfo.mode, timeline, changes: Object.values(changesById) });
      await get().loadHistories(workspaceId);
    } catch { /* history is best effort */ }
  },

  openHistory: async (id) => {
    await get().saveCurrentHistory();
    const detail = await codingService.historyGet(id);
    if (!detail) return;
    set({ workspaceId: detail.workspace_id, viewingHistoryId: detail.id, sessionInfo: { id: detail.id, provider_id: detail.provider_id, mode: detail.mode as CodingMode, target: get().sessionInfo?.target ?? { kind: "Local" }, auto_allow_readonly: false, git_repo: false, auto_git_commit: false, changes: detail.changes as FileChange[] }, timeline: detail.timeline as TimelineEntry[], changesById: Object.fromEntries((detail.changes as FileChange[]).map((c) => [c.id, c])), error: null });
  },

  deleteHistory: async (id) => {
    const { viewingHistoryId, sessionInfo } = get();
    await codingService.historyDelete(id);
    set((s) => ({ histories: s.histories.filter((item) => item.id !== id), viewingHistoryId: s.viewingHistoryId === id ? null : s.viewingHistoryId }));
    if (viewingHistoryId === id && sessionInfo) await get().newSession(sessionInfo.provider_id);
  },

  renameHistory: async (id, title) => {
    const trimmed = title.trim();
    if (!trimmed) return;
    await codingService.historyRename(id, trimmed);
    await get().loadHistories();
  },

  newSession: async (providerId) => {
    await get().saveCurrentHistory();
    const workspaceId = get().workspaceId;
    if (!workspaceId) return;
    const info = await codingService.newSession(workspaceId, providerId);
    set({ sessionInfo: info, timeline: [], changesById: {}, viewingHistoryId: null, error: null });
  },

  restoreOrStart: async (workspaceId, providerId) => {
    // 切换工作区时先清空旧工作区的内存快照，避免异步恢复期间串显示。
    set({ workspaceId, sessionInfo: null, timeline: [], changesById: {}, viewingHistoryId: null, error: null, histories: [] });
    try {
      const histories = await codingService.historyList(workspaceId);
      const latest = histories[0];
      const latestTime = latest ? Date.parse(latest.updated_at) : NaN;
      const recent = latest && Number.isFinite(latestTime) && (Date.now() - latestTime) <= 12 * 60 * 60 * 1000;
      const detail = recent ? await codingService.historyGet(latest.id) : null;
      const availableProviders = useAiChatStore.getState().providers;
      const resumeProviderId = detail && availableProviders.some((item) => item.id === detail.provider_id) ? detail.provider_id : providerId;
      // 始终通过 new_session 重新绑定当前工作区；不能复用其他工作区/旧进程中的 target。
      const info = await codingService.newSession(workspaceId, detail ? resumeProviderId : providerId);
      if (get().workspaceId !== workspaceId) return;
      if (!recent) {
        set({ workspaceId, sessionInfo: info, timeline: [], changesById: {}, viewingHistoryId: null, histories, error: null });
        return;
      }
      if (!detail) {
        set({ workspaceId, sessionInfo: info, timeline: [], changesById: {}, viewingHistoryId: null, histories, error: null });
        return;
      }
      const changes = detail.changes as FileChange[];
      set({ workspaceId, sessionInfo: { ...info, mode: detail.mode as CodingMode, changes }, timeline: detail.timeline as TimelineEntry[], changesById: Object.fromEntries(changes.map((change) => [change.id, change])), viewingHistoryId: null, histories, error: null });
    } catch (e) {
      if (get().workspaceId === workspaceId) set({ error: formatError(e) });
    }
  },

  reset: () => set({ workspaceId: null, sessionInfo: null, timeline: [], changesById: {}, sending: false, error: null, confirmRequest: null }),
}));

let listenersRegistered = false;

/** 全局注册一次编程助手事件监听（App.tsx 挂载时调用，和 registerAiChatListeners 同款模式）。*/
export function registerCodingListeners(): Promise<() => void> {
  if (listenersRegistered) return Promise.resolve(() => {});
  listenersRegistered = true;

  const currentSessionId = () => useCodingStore.getState().sessionInfo?.id;

  const unlistenPromises = [
    listen<CodingToolCallEvent>("coding:tool-call-start", (event) => {
      if (event.payload.sessionId !== currentSessionId()) return;
      useCodingStore.setState((s) => ({
        timeline: [...s.timeline, { kind: "tool", id: nextId(), tool: event.payload.tool, running: true, detail: event.payload.detail }],
      }));
    }),
    listen<CodingToolCallEvent>("coding:tool-call-end", (event) => {
      if (event.payload.sessionId !== currentSessionId()) return;
      useCodingStore.setState((s) => {
        const idx = [...s.timeline].reverse().findIndex((t) => t.kind === "tool" && t.tool === event.payload.tool && t.running);
        if (idx === -1) return s;
        const realIdx = s.timeline.length - 1 - idx;
        const timeline = [...s.timeline];
        const entry = timeline[realIdx];
        if (entry.kind === "tool") timeline[realIdx] = { ...entry, running: false };
        return { timeline };
      });
    }),
    listen<CodingAssistantNoteEvent>("coding:assistant-note", (event) => {
      if (event.payload.sessionId !== currentSessionId()) return;
      useCodingStore.setState((s) => ({
        timeline: [...s.timeline, event.payload.kind === "status"
          ? { kind: "progress", id: nextId(), text: event.payload.text }
          : { kind: "note", id: nextId(), text: event.payload.text }],
      }));
    }),
    listen<CodingFileChangeEvent>("coding:file-change", (event) => {
      if (event.payload.sessionId !== currentSessionId()) return;
      const change = event.payload.change;
      useCodingStore.setState((s) => ({
        changesById: { ...s.changesById, [change.id]: change },
        timeline: [...s.timeline, { kind: "change", id: nextId(), changeId: change.id }],
      }));
    }),
    listen<CodingCommandBlockedEvent>("coding:command-blocked", (event) => {
      if (event.payload.sessionId !== currentSessionId()) return;
      useCodingStore.setState((s) => ({
        timeline: [...s.timeline, { kind: "blocked", id: nextId(), command: event.payload.command }],
      }));
    }),
    listen<CodingCommandConfirmRequestEvent>("coding:command-confirm-request", (event) => {
      if (event.payload.sessionId !== currentSessionId()) return;
      useCodingStore.setState({
        confirmRequest: { requestId: event.payload.requestId, command: event.payload.command, host: event.payload.host },
      });
    }),
    listen<CodingGitCommitResultEvent>("coding:git-commit-result", (event) => {
      if (event.payload.sessionId !== currentSessionId()) return;
      useCodingStore.setState((s) => ({
        timeline: [...s.timeline, { kind: "git", id: nextId(), path: event.payload.path, output: event.payload.output }],
      }));
    }),
  ];

  return Promise.all(unlistenPromises).then((unlistens) => () => unlistens.forEach((u) => u()));
}
