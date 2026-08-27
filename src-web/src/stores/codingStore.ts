import { create } from "zustand";
import { listen } from "@tauri-apps/api/event";
import { codingService } from "../services/codingService";
import { permissionRuleService } from "../services/permissionRuleService";
import { mcpServerService } from "../services/mcpServerService";
import { formatError } from "../utils/error";
import { useAiChatStore } from "./aiChatStore";
import type {
  ChatAttachment,
  CodingAssistantNoteEvent,
  CodingCommandBlockedEvent,
  CodingCommandConfirmRequestEvent,
  CodingFileChangeEvent,
  CodingGitCommitResultEvent,
  CodingMode,
  CodingQuestionRequestEvent,
  CodingSessionInfo,
  CodingToolCallEvent,
  CodingTodoUpdateEvent,
  FileChange,
  CodingHistorySummary,
  McpServer,
  McpServerInput,
  PermissionRule,
  PermissionRuleInput,
} from "../types/bindings";

/** 附件在被真正发送之前，输入框上方"待发送"区域用的前端内部表示——图片额外带
 * `previewUrl`（完整 data URL，直接喂给 `<img>`）,发送时才拆成不带前缀的
 * `data_base64` 传给后端（见 `toChatAttachment`）。 */
export interface PendingAttachment {
  id: string;
  kind: "image" | "file";
  name: string;
  size: number;
  mime?: string;
  previewUrl?: string;
  base64?: string;
  content?: string;
}

/** 时间线里"用户消息"回显用的附件摘要——只保留渲染需要的字段，不是完整的
 * `ChatAttachment`（那份已经在发送时连同文本一起交给后端，不需要在前端状态里
 * 重复保留 base64/文件全文）。 */
export interface TimelineAttachment {
  kind: "image" | "file";
  name: string;
  previewUrl?: string;
}

export type TimelineEntry =
  | { kind: "user"; id: string; text: string; attachments?: TimelineAttachment[] }
  | { kind: "assistant"; id: string; text: string }
  /** 工具调用之外顺带写的说明文字，不是最终答案——单独一种样式，和 "assistant"
   * 区分开（2026-08-18 需求，见 CodingAssistantNoteEvent 的注释）。 */
  | { kind: "note"; id: string; text: string }
  | { kind: "progress"; id: string; text: string }
  | { kind: "tool"; id: string; tool: string; running: boolean; detail?: string | null }
  | { kind: "change"; id: string; changeId: string }
  | { kind: "blocked"; id: string; command: string }
  | { kind: "git"; id: string; path: string; output: string };

/** 有界保活的 LRU 上限——同时"活着"的工作区编程会话数（终端侧
 * `terminalStore.ts` 用同一个常量、同一套策略，两边各自独立维护，不共享一份
 * 状态，因为终端 Channel 和 AI 会话是两种不同的后端资源，没必要绑死成"必须
 * 同进同出"）。 */
const MAX_RESIDENT_WORKSPACES = 3;

/** 从当前显示切走时，被"晾"在一边但仍然保活的工作区快照——不落库，纯内存缓存，
 * 切回来时原样恢复，不重新走 `newSession`/历史恢复流程。 */
interface WorkspaceSnapshot {
  sessionInfo: CodingSessionInfo | null;
  timeline: TimelineEntry[];
  changesById: Record<string, FileChange>;
  viewingHistoryId: string | null;
}

interface CodingState {
  workspaceId: string | null;
  sessionInfo: CodingSessionInfo | null;
  timeline: TimelineEntry[];
  changesById: Record<string, FileChange>;
  sending: boolean;
  error: string | null;
  /** composer 里"待发送"的附件——发出去之后清空（见 `sendMessage`）。 */
  attachments: PendingAttachment[];
  optimizing: boolean;
  confirmRequest: { requestId: string; command: string; host: string | null; kind: "command" | "mcp"; matchKey?: string } | null;
  questionRequest: { requestId: string; question: string; options: string[] } | null;
  permissionRules: PermissionRule[];
  mcpServers: McpServer[];
  histories: CodingHistorySummary[];
  viewingHistoryId: string | null;
  /** 最近使用的工作区 id，最前面的最新；只用来判断 LRU 淘汰顺序。 */
  residentOrder: string[];
  /** 当前没有显示、但仍保活（未被淘汰）的工作区快照。 */
  byWorkspace: Record<string, WorkspaceSnapshot>;
  loadHistories: (workspaceId?: string) => Promise<void>;
  saveCurrentHistory: () => Promise<void>;
  openHistory: (id: string) => Promise<void>;
  deleteHistory: (id: string) => Promise<void>;
  renameHistory: (id: string, title: string) => Promise<void>;
  newSession: (providerId: string) => Promise<void>;
  /** 切换到某个工作区的编程助手会话：已保活（在 `byWorkspace` 里）直接原地
   * 恢复快照，不碰后端；否则按原有逻辑走后端会话 + 12 小时内历史恢复。
   * 切走的上一个工作区会被记入保活集合，超过 `MAX_RESIDENT_WORKSPACES` 时
   * 淘汰最久未用的一个（前端丢弃快照 + 后端 `coding_close` 真正释放会话）。 */
  restoreOrStart: (workspaceId: string, providerId: string) => Promise<void>;
  /** 淘汰一个工作区的保活会话：前端丢快照，后端调 `coding_close` 真正释放
   * `CodingSession`（不这样做的话，之前"每次切换都 newSession"的实现其实从没
   * 清理过后端 `coding_sessions` 这张表，会随打开过的工作区数量无界增长）。 */
  evictWorkspace: (workspaceId: string) => Promise<void>;

  start: (workspaceId: string, providerId: string) => Promise<void>;
  setMode: (mode: CodingMode) => Promise<void>;
  setProvider: (providerId: string) => Promise<void>;
  setAutoAllowReadonly: (enabled: boolean) => Promise<void>;
  setAutoGitCommit: (enabled: boolean) => Promise<void>;
  sendMessage: (text: string) => Promise<void>;
  addAttachments: (files: File[]) => Promise<void>;
  removeAttachment: (id: string) => void;
  clearAttachments: () => void;
  /** 调用当前 Provider 把 `text` 改写成更清晰的提示词并返回改写结果，不修改
   * 任何会话状态——组件拿到返回值后自己决定是否替换输入框内容。 */
  optimizePrompt: (text: string) => Promise<string>;
  acceptChange: (changeId: string) => Promise<void>;
  rejectChange: (changeId: string) => Promise<void>;
  undoChange: (changeId: string) => Promise<void>;
  redoChange: () => Promise<void>;
  resolveConfirm: (allow: boolean) => Promise<void>;
  /** "允许并记住"：先按建议模式落一条 allow 规则，再照常放行这一次。 */
  resolveConfirmAndRemember: (pattern: string) => Promise<void>;
  answerQuestion: (answer: string) => Promise<void>;
  loadPermissionRules: () => Promise<void>;
  createPermissionRule: (input: PermissionRuleInput) => Promise<void>;
  deletePermissionRule: (id: string) => Promise<void>;
  loadMcpServers: () => Promise<void>;
  createMcpServer: (input: McpServerInput) => Promise<void>;
  updateMcpServer: (id: string, input: McpServerInput) => Promise<void>;
  deleteMcpServer: (id: string) => Promise<void>;
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
  attachments: [],
  optimizing: false,
  confirmRequest: null,
  questionRequest: null,
  permissionRules: [],
  mcpServers: [],
  histories: [],
  viewingHistoryId: null,
  residentOrder: [],
  byWorkspace: {},

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
    const { workspaceId, sending, viewingHistoryId, attachments } = get();
    if (!workspaceId || (!text.trim() && attachments.length === 0) || sending || viewingHistoryId) return;
    const outgoing = attachments.map(toChatAttachment);
    const timelineAttachments: TimelineAttachment[] | undefined = attachments.length
      ? attachments.map((a) => ({ kind: a.kind, name: a.name, previewUrl: a.previewUrl }))
      : undefined;
    set((s) => ({
      timeline: [...s.timeline, { kind: "user", id: nextId(), text, attachments: timelineAttachments }],
      sending: true,
      error: null,
      attachments: [],
    }));
    try {
      const reply = await codingService.sendMessage(workspaceId, text, outgoing);
      set((s) => ({ timeline: [...s.timeline, { kind: "assistant", id: nextId(), text: reply }], sending: false }));
      await get().saveCurrentHistory();
    } catch (e) {
      set({ sending: false, error: formatError(e) });
    }
  },

  addAttachments: async (files) => {
    const read = await Promise.all(files.map(readAttachment));
    const valid = read.filter((a): a is PendingAttachment => a !== null);
    if (valid.length === 0) return;
    set((s) => ({ attachments: [...s.attachments, ...valid].slice(0, MAX_ATTACHMENTS) }));
  },

  removeAttachment: (id) => {
    set((s) => ({ attachments: s.attachments.filter((a) => a.id !== id) }));
  },

  clearAttachments: () => set({ attachments: [] }),

  optimizePrompt: async (text) => {
    const { workspaceId, optimizing, viewingHistoryId } = get();
    if (!workspaceId || !text.trim() || optimizing || viewingHistoryId) return text;
    set({ optimizing: true, error: null });
    try {
      const optimized = await codingService.optimizePrompt(workspaceId, text);
      set({ optimizing: false });
      return optimized;
    } catch (e) {
      set({ optimizing: false, error: formatError(e) });
      return text;
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

  resolveConfirmAndRemember: async (pattern) => {
    const { confirmRequest } = get();
    if (!confirmRequest) return;
    set({ confirmRequest: null });
    try {
      await get().createPermissionRule({
        tool: confirmRequest.kind === "mcp" ? "mcp" : "run_command",
        pattern,
        decision: "allow",
      });
    } finally {
      await codingService.confirmCommand(confirmRequest.requestId, true);
    }
  },

  answerQuestion: async (answer) => {
    const { questionRequest } = get();
    if (!questionRequest) return;
    set({ questionRequest: null });
    await codingService.answerQuestion(questionRequest.requestId, answer);
  },

  loadPermissionRules: async () => {
    try { set({ permissionRules: await permissionRuleService.list() }); } catch { /* best effort */ }
  },

  createPermissionRule: async (input) => {
    const rule = await permissionRuleService.create(input);
    set((s) => ({ permissionRules: [...s.permissionRules, rule] }));
  },

  deletePermissionRule: async (id) => {
    await permissionRuleService.delete(id);
    set((s) => ({ permissionRules: s.permissionRules.filter((r) => r.id !== id) }));
  },

  loadMcpServers: async () => {
    try { set({ mcpServers: await mcpServerService.list() }); } catch { /* best effort */ }
  },

  createMcpServer: async (input) => {
    const server = await mcpServerService.create(input);
    set((s) => ({ mcpServers: [...s.mcpServers, server] }));
  },

  updateMcpServer: async (id, input) => {
    const server = await mcpServerService.update(id, input);
    set((s) => ({ mcpServers: s.mcpServers.map((m) => (m.id === id ? server : m)) }));
  },

  deleteMcpServer: async (id) => {
    await mcpServerService.delete(id);
    set((s) => ({ mcpServers: s.mcpServers.filter((m) => m.id !== id) }));
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
    set({ workspaceId: detail.workspace_id, viewingHistoryId: detail.id, sessionInfo: { id: detail.id, provider_id: detail.provider_id, mode: detail.mode as CodingMode, target: get().sessionInfo?.target ?? { kind: "Local" }, auto_allow_readonly: false, git_repo: false, auto_git_commit: false, changes: detail.changes as FileChange[], todos: [], project_memory_loaded: [] }, timeline: detail.timeline as TimelineEntry[], changesById: Object.fromEntries((detail.changes as FileChange[]).map((c) => [c.id, c])), error: null });
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
    const prev = get();
    if (prev.workspaceId === workspaceId) return;

    // 把切走的上一个工作区的当前显示状态存进保活缓存（不落库，纯内存），
    // 后端会话原样留着不动——这就是"保活"的核心：不调用任何关闭/新建接口。
    const nextByWorkspace = prev.workspaceId
      ? {
          ...prev.byWorkspace,
          [prev.workspaceId]: {
            sessionInfo: prev.sessionInfo,
            timeline: prev.timeline,
            changesById: prev.changesById,
            viewingHistoryId: prev.viewingHistoryId,
          },
        }
      : prev.byWorkspace;

    // LRU：把目标工作区提到最前，超出上限的从尾部淘汰。
    const order = [workspaceId, ...prev.residentOrder.filter((id) => id !== workspaceId)];
    const toEvict: string[] = [];
    while (order.length > MAX_RESIDENT_WORKSPACES) {
      const victim = order.pop();
      if (victim) toEvict.push(victim);
    }
    set({ residentOrder: order, byWorkspace: nextByWorkspace });
    for (const id of toEvict) {
      await get().evictWorkspace(id);
    }

    const cached = get().byWorkspace[workspaceId];
    if (cached) {
      // 已保活：原地恢复快照，不碰后端（既不 newSession 也不重新拉历史）。
      const { [workspaceId]: _discard, ...rest } = get().byWorkspace;
      set({
        workspaceId,
        sessionInfo: cached.sessionInfo,
        timeline: cached.timeline,
        changesById: cached.changesById,
        viewingHistoryId: cached.viewingHistoryId,
        byWorkspace: rest,
        error: null,
      });
      return;
    }

    // 未保活（首次打开，或此前已被淘汰）：走原有逻辑——后端起一个新会话，
    // 12 小时内有历史就把历史内容覆盖回显。
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

  evictWorkspace: async (workspaceId) => {
    set((s) => {
      const { [workspaceId]: _discard, ...rest } = s.byWorkspace;
      return { byWorkspace: rest };
    });
    try {
      await codingService.closeSession(workspaceId);
    } catch {
      /* 尽力而为——释放不掉也不应该阻塞正在进行的工作区切换 */
    }
  },

  reset: () =>
    set({
      workspaceId: null,
      sessionInfo: null,
      timeline: [],
      changesById: {},
      sending: false,
      error: null,
      attachments: [],
      optimizing: false,
      confirmRequest: null,
      questionRequest: null,
      residentOrder: [],
      byWorkspace: {},
    }),
}));

/** composer 里同时挂着的附件数量上限——纯 UX 约束（避免误拖一整个文件夹进来），
 * 不是协议限制。 */
const MAX_ATTACHMENTS = 6;
/** 文本类附件按字符数截断，避免一个不小心拖进来的大文件把整条消息撑爆、吃光
 * 上下文预算；截断比拒绝更友好，模型仍能看到文件的大部分内容。 */
const MAX_TEXT_ATTACHMENT_CHARS = 200_000;
/** 图片按原始文件大小限制——base64 编码后体积还会再涨约 1/3，云端 Provider 的
 * 请求体通常也有上限，这里留足余量。 */
const MAX_IMAGE_BYTES = 8 * 1024 * 1024;

function readAttachment(file: File): Promise<PendingAttachment | null> {
  return new Promise((resolve) => {
    const isImage = file.type.startsWith("image/");
    if (isImage && file.size > MAX_IMAGE_BYTES) {
      resolve(null);
      return;
    }
    const reader = new FileReader();
    reader.onerror = () => resolve(null);
    if (isImage) {
      reader.onload = () => {
        const dataUrl = typeof reader.result === "string" ? reader.result : "";
        const base64 = dataUrl.slice(dataUrl.indexOf(",") + 1);
        if (!base64) {
          resolve(null);
          return;
        }
        resolve({
          id: nextId(),
          kind: "image",
          name: file.name,
          size: file.size,
          mime: file.type || "image/png",
          previewUrl: dataUrl,
          base64,
        });
      };
      reader.readAsDataURL(file);
    } else {
      reader.onload = () => {
        const text = typeof reader.result === "string" ? reader.result : "";
        const truncated = text.length > MAX_TEXT_ATTACHMENT_CHARS;
        resolve({
          id: nextId(),
          kind: "file",
          name: file.name,
          size: file.size,
          content: truncated ? `${text.slice(0, MAX_TEXT_ATTACHMENT_CHARS)}\n...[内容过长，已截断]` : text,
        });
      };
      reader.readAsText(file);
    }
  });
}

function toChatAttachment(attachment: PendingAttachment): ChatAttachment {
  if (attachment.kind === "image") {
    return { kind: "image", name: attachment.name, mime: attachment.mime ?? "image/png", data_base64: attachment.base64 ?? "" };
  }
  return { kind: "file", name: attachment.name, content: attachment.content ?? "" };
}

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
        confirmRequest: {
          requestId: event.payload.requestId,
          command: event.payload.command,
          host: event.payload.host,
          kind: event.payload.kind ?? "command",
          matchKey: event.payload.matchKey,
        },
      });
    }),
    listen<CodingTodoUpdateEvent>("coding:todo-update", (event) => {
      if (event.payload.sessionId !== currentSessionId()) return;
      useCodingStore.setState((s) => (s.sessionInfo ? { sessionInfo: { ...s.sessionInfo, todos: event.payload.todos } } : s));
    }),
    listen<CodingQuestionRequestEvent>("coding:question-request", (event) => {
      if (event.payload.sessionId !== currentSessionId()) return;
      useCodingStore.setState({
        questionRequest: { requestId: event.payload.requestId, question: event.payload.question, options: event.payload.options },
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
