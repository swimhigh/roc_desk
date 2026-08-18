import { create } from "zustand";
import { listen } from "@tauri-apps/api/event";
import { codingService } from "../services/codingService";
import { formatError } from "../utils/error";
import type {
  CodingCommandBlockedEvent,
  CodingCommandConfirmRequestEvent,
  CodingFileChangeEvent,
  CodingGitCommitResultEvent,
  CodingMode,
  CodingSessionInfo,
  CodingToolCallEvent,
  FileChange,
} from "../types/bindings";

export type TimelineEntry =
  | { kind: "user"; id: string; text: string }
  | { kind: "assistant"; id: string; text: string }
  | { kind: "tool"; id: string; tool: string; running: boolean }
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

  start: (workspaceId: string, providerId: string) => Promise<void>;
  setMode: (mode: CodingMode) => Promise<void>;
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
    } catch (e) {
      set({ error: formatError(e) });
    }
  },

  setMode: async (mode) => {
    const { workspaceId, sessionInfo } = get();
    if (!workspaceId || !sessionInfo) return;
    await codingService.setMode(workspaceId, mode);
    set({ sessionInfo: { ...sessionInfo, mode } });
  },

  setAutoAllowReadonly: async (enabled) => {
    const { workspaceId, sessionInfo } = get();
    if (!workspaceId || !sessionInfo) return;
    await codingService.setAutoAllowReadonly(workspaceId, enabled);
    set({ sessionInfo: { ...sessionInfo, auto_allow_readonly: enabled } });
  },

  setAutoGitCommit: async (enabled) => {
    const { workspaceId, sessionInfo } = get();
    if (!workspaceId || !sessionInfo) return;
    await codingService.setAutoGitCommit(workspaceId, enabled);
    set({ sessionInfo: { ...sessionInfo, auto_git_commit: enabled } });
  },

  sendMessage: async (text) => {
    const { workspaceId, sending } = get();
    if (!workspaceId || !text.trim() || sending) return;
    set((s) => ({ timeline: [...s.timeline, { kind: "user", id: nextId(), text }], sending: true, error: null }));
    try {
      const reply = await codingService.sendMessage(workspaceId, text);
      set((s) => ({ timeline: [...s.timeline, { kind: "assistant", id: nextId(), text: reply }], sending: false }));
    } catch (e) {
      set({ sending: false, error: formatError(e) });
    }
  },

  acceptChange: async (changeId) => {
    const { workspaceId } = get();
    if (!workspaceId) return;
    await codingService.acceptChange(workspaceId, changeId);
    set((s) => ({
      changesById: { ...s.changesById, [changeId]: { ...s.changesById[changeId], status: "applied" } },
    }));
  },

  rejectChange: async (changeId) => {
    const { workspaceId } = get();
    if (!workspaceId) return;
    await codingService.rejectChange(workspaceId, changeId);
    set((s) => ({
      changesById: { ...s.changesById, [changeId]: { ...s.changesById[changeId], status: "rejected" } },
    }));
  },

  undoChange: async (changeId) => {
    const { workspaceId } = get();
    if (!workspaceId) return;
    await codingService.undoChange(workspaceId, changeId);
    set((s) => ({
      changesById: { ...s.changesById, [changeId]: { ...s.changesById[changeId], status: "undone" } },
    }));
  },

  redoChange: async () => {
    const { workspaceId } = get();
    if (!workspaceId) return;
    const changeId = await codingService.redoChange(workspaceId);
    if (!changeId) return;
    set((s) => ({
      changesById: { ...s.changesById, [changeId]: { ...s.changesById[changeId], status: "applied" } },
    }));
  },

  resolveConfirm: async (allow) => {
    const { confirmRequest } = get();
    if (!confirmRequest) return;
    set({ confirmRequest: null });
    await codingService.confirmCommand(confirmRequest.requestId, allow);
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
        timeline: [...s.timeline, { kind: "tool", id: nextId(), tool: event.payload.tool, running: true }],
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
