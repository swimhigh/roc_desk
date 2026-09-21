import { create } from "zustand";
import { listen } from "@tauri-apps/api/event";
import { codingService } from "../services/codingService";
import { permissionRuleService } from "../services/permissionRuleService";
import { mcpServerService } from "../services/mcpServerService";
import { skillService } from "../services/skillService";
import { formatError } from "../utils/error";
import { useAiChatStore } from "./aiChatStore";
import { useEditorStore } from "./editorStore";
import type {
  ChatAttachment,
  CodingAssistantNoteEvent,
  CodingAutoContinueDoneEvent,
  CodingAutoContinueStartEvent,
  CodingCommandBlockedEvent,
  CodingCommandConfirmRequestEvent,
  CodingFileChangeEvent,
  CodingGitCommitResultEvent,
  CodingMode,
  CodingQuestionRequestEvent,
  CodingSessionInfo,
  CodingToolCallEvent,
  CodingTodoUpdateEvent,
  CodingTokenUsageEvent,
  FileChange,
  CodingHistorySummary,
  McpServer,
  McpServerInput,
  PermissionRule,
  PermissionRuleInput,
  SkillMeta,
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
export interface CommandConfirmRequest {
  requestId: string;
  command: string;
  host: string | null;
  kind: "command" | "mcp";
  matchKey?: string;
}

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
  | { kind: "tool"; id: string; tool: string; running: boolean; detail?: string | null; startedAt?: number; output?: string | null; expanded?: boolean }
  | { kind: "change"; id: string; changeId: string }
  | { kind: "blocked"; id: string; command: string }
  | { kind: "git"; id: string; path: string; output: string }
  | { kind: "usage"; id: string; promptTokens: number; completionTokens: number; totalTokens: number; isTurnTotal: boolean };

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
  /** 当前这一轮还在进行中的 token 用量——2026-09 用户反馈"屏幕上不停在刷
   * token，也看不到关键进度"：根因是之前每一次模型往返（一轮用户消息里可能
   * 有几十次）都各自往时间线里插一条"本次请求消耗 tokens"消息，把真正有信息量
   * 的"已完成 XXX/正在思考"这些进度提示淹没在一长串数字里。改成参考
   * Claude Code 自己的做法：单次请求的用量只做"实时覆盖式"展示（这个字段），
   * 跟"AI 正在处理…"状态行放一起、原地更新，不占用一条独立的历史消息；只有
   * 一整轮真正结束时的汇总（`coding:token-usage-summary`）才留一条时间线记录，
   * 见下面两个事件监听器。 */
  liveTokenUsage: { promptTokens: number; completionTokens: number; totalTokens: number } | null;
  error: string | null;
  /** composer 里"待发送"的附件——发出去之后清空（见 `sendMessage`）。 */
  attachments: PendingAttachment[];
  optimizing: boolean;
  confirmRequest: CommandConfirmRequest | null;
  /** 排在 `confirmRequest` 后面、还没展示出来的待确认请求——2026-09 用户实测
   * 复现：AI 引擎会并发发起多个工具调用（比如远程 SSH 目标下一连串
   * `run_command`），每个都各自触发一次 `coding:command-confirm-request`。
   * `confirmRequest` 之前是单个可空字段，事件监听器直接整体覆盖，第二个
   * 确认请求一来就把还没被用户处理的第一个悄悄顶掉——被顶掉那个在后端
   * `CommandConfirmRegistry` 里永远等不到 `resolve()`，对应的
   * `run_command_gated_shared_with_status_in_context` 里的 `rx.await` 会
   * 卡死，界面上完全看不出还有请求在等，表现就是"AI 工作卡住不回应"。
   * 现在改成队列：新请求如果当前已经在展示一个，就排到这里；当前那个被
   * 处理完（`resolveConfirm`/`resolveConfirmAndRemember`）之后自动把队首
   * 换上来展示，不会再丢失。 */
  confirmQueue: CommandConfirmRequest[];
  questionRequest: { requestId: string; question: string; options: string[] } | null;
  permissionRules: PermissionRule[];
  mcpServers: McpServer[];
  skills: SkillMeta[];
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
  /** "完全授权模式"：开启后 AI 提出的文件改动直接落盘，不再逐个 Accept
   * （用户反馈"一次改 20 多个文件还要逐个确认太繁琐"）。会话级开关。 */
  setFullAuto: (enabled: boolean) => Promise<void>;
  /** 文件改动自动应用（默认开启），关掉退回"每条手动点应用"，不影响命令确认。 */
  setAutoApplyChanges: (enabled: boolean) => Promise<void>;
  sendMessage: (text: string) => Promise<void>;
  /** "停止"按钮：只在 `sending` 为 true 时有意义——`sendMessage` 本身的
   * catch 分支已经会处理后端返回的"已取消"错误，这里不需要额外更新
   * `sending`/`error` 状态。 */
  cancelTurn: () => Promise<void>;
  addAttachments: (files: File[]) => Promise<void>;
  removeAttachment: (id: string) => void;
  clearAttachments: () => void;
  /** 调用当前 Provider 把 `text` 改写成更清晰的提示词并返回改写结果，不修改
   * 任何会话状态——组件拿到返回值后自己决定是否替换输入框内容。 */
  optimizePrompt: (text: string) => Promise<string>;
  acceptChange: (changeId: string) => Promise<void>;
  rejectChange: (changeId: string) => Promise<void>;
  /** 点开/收起时间线里某条已完成工具调用的执行结果——纯前端展示状态，不涉及
   * 任何后端往返。 */
  toggleToolOutput: (id: string) => void;
  undoChange: (changeId: string) => Promise<void>;
  redoChange: () => Promise<void>;
  /** 撤销某一轮对话里 AI 做出的全部已应用改动（参考 Cursor/Windsurf 的按轮次
   * 整体撤销，不依赖 git）。 */
  revertTurn: (turnId: string) => Promise<void>;
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
  loadSkills: (workspaceId: string) => Promise<void>;
  importSkill: (workspaceId: string, localPath: string) => Promise<SkillMeta>;
  deleteSkill: (workspaceId: string, name: string) => Promise<void>;
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
  liveTokenUsage: null,
  error: null,
  attachments: [],
  optimizing: false,
  confirmRequest: null,
  confirmQueue: [],
  questionRequest: null,
  permissionRules: [],
  mcpServers: [],
  skills: [],
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

  setFullAuto: async (enabled) => {
    const { workspaceId, sessionInfo, viewingHistoryId } = get();
    if (viewingHistoryId) return;
    if (!workspaceId || !sessionInfo) return;
    await codingService.setFullAuto(workspaceId, enabled);
    set((s) => ({
      sessionInfo: { ...sessionInfo, full_auto: enabled },
      confirmRequest: enabled ? null : s.confirmRequest,
    }));
  },

  setAutoApplyChanges: async (enabled) => {
    const { workspaceId, sessionInfo, viewingHistoryId } = get();
    if (viewingHistoryId) return;
    if (!workspaceId || !sessionInfo) return;
    await codingService.setAutoApplyChanges(workspaceId, enabled);
    set({ sessionInfo: { ...sessionInfo, auto_apply_changes: enabled } });
  },

  sendMessage: async (text) => {
    const { workspaceId, sending, viewingHistoryId, attachments } = get();
    if (!workspaceId || (!text.trim() && attachments.length === 0) || viewingHistoryId) return;
    const outgoing = attachments.map(toChatAttachment);
    const timelineAttachments: TimelineAttachment[] | undefined = attachments.length
      ? attachments.map((a) => ({ kind: a.kind, name: a.name, previewUrl: a.previewUrl }))
      : undefined;

    // AI 还在处理上一条消息——之前这里直接 no-op 返回，但调用方 `handleSend`
    // 无条件清空了输入框，导致这条新消息看起来像是"发出去了"，实际上既没有
    // 进时间线也没有真正发给后端（2026-09 用户反馈：处理期间按 Enter，输入框
    // 内容消失、消息没有插入到对话里）。现在改成走 `injectMessage`：不等这一
    // 轮结束，后端会在下一次工具调用/模型请求前把它塞进对话上下文（见
    // `codingService.injectMessage` 文档），这里先把气泡乐观加进时间线、清空
    // 输入框，不改 `sending`（已经是 true，这次调用不会有对应的 assistant 回复）。
    if (sending) {
      set((s) => ({
        timeline: [...s.timeline, { kind: "user", id: nextId(), text, attachments: timelineAttachments }],
        attachments: [],
      }));
      try {
        await codingService.injectMessage(workspaceId, text, outgoing);
      } catch (e) {
        set({ error: formatError(e) });
      }
      return;
    }

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
      const message = formatError(e);
      // 用户主动点了"停止"，不算真正的错误——用红色错误条展示会显得像哪里
      // 出了故障，改成走时间线里一条普通的状态提示（跟 `assistant-note`
      // 那类顺带说明文字的展示方式一致，不是最终答案）。
      if (message.includes("已停止：用户取消了当前对话轮次")) {
        // 被取消的这一轮里，已经派发但还没收到 `coding:tool-call-end` 的
        // 工具调用（`kind: "tool", running: true`）永远等不到那个事件了——
        // 轮次本身已经中止，不会再有后续事件补上。不清掉的话这些条目会
        // 一直转圈，看起来像"点了停止也没用"（2026-09 用户实测反馈）。
        set((s) => ({
          sending: false,
          timeline: [
            ...s.timeline.map((t) => (t.kind === "tool" && t.running ? { ...t, running: false } : t)),
            { kind: "note", id: nextId(), text: "已停止" },
          ],
        }));
      } else {
        set({ sending: false, error: message });
      }
    }
  },

  cancelTurn: async () => {
    const { workspaceId, sending } = get();
    if (!workspaceId || !sending) return;
    await codingService.cancelTurn(workspaceId);
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

  toggleToolOutput: (id) => {
    set((s) => ({
      timeline: s.timeline.map((entry) =>
        entry.kind === "tool" && entry.id === id ? { ...entry, expanded: !entry.expanded } : entry,
      ),
    }));
  },

  acceptChange: async (changeId) => {
    const { workspaceId, viewingHistoryId } = get();
    if (!workspaceId || viewingHistoryId) return;
    const sync = await codingService.acceptChange(workspaceId, changeId);
    set((s) => ({
      changesById: { ...s.changesById, [changeId]: { ...s.changesById[changeId], status: "applied" } },
    }));
    useEditorStore.getState().syncExternalWrite(sync.path, sync.content, sync.mtime);
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
    const sync = await codingService.undoChange(workspaceId, changeId);
    set((s) => ({
      changesById: { ...s.changesById, [changeId]: { ...s.changesById[changeId], status: "undone" } },
    }));
    useEditorStore.getState().syncExternalWrite(sync.path, sync.content, sync.mtime);
    await get().saveCurrentHistory();
  },

  redoChange: async () => {
    const { workspaceId, viewingHistoryId } = get();
    if (!workspaceId || viewingHistoryId) return;
    const sync = await codingService.redoChange(workspaceId);
    if (!sync) return;
    set((s) => ({
      changesById: { ...s.changesById, [sync.change_id]: { ...s.changesById[sync.change_id], status: "applied" } },
    }));
    useEditorStore.getState().syncExternalWrite(sync.path, sync.content, sync.mtime);
    await get().saveCurrentHistory();
  },

  revertTurn: async (turnId) => {
    const { workspaceId, viewingHistoryId } = get();
    if (!workspaceId || viewingHistoryId) return;
    try {
      const reverted = await codingService.revertTurn(workspaceId, turnId);
      if (reverted.length === 0) {
        set({ error: "当前轮次没有可撤销的已应用改动" });
        return;
      }
      set((s) => {
        const changesById = { ...s.changesById };
        for (const sync of reverted) {
          if (changesById[sync.change_id]) changesById[sync.change_id] = { ...changesById[sync.change_id], status: "undone" };
        }
        return { changesById, error: null };
      });
      for (const sync of reverted) {
        useEditorStore.getState().syncExternalWrite(sync.path, sync.content, sync.mtime);
      }
      await get().saveCurrentHistory();
    } catch (e) {
      set({ error: formatError(e) });
    }
  },

  resolveConfirm: async (allow) => {
    const { confirmRequest } = get();
    if (!confirmRequest) return;
    // 处理完当前这个之后，把排队里的下一个换上来——不这样做的话，队列里
    // 积压的请求永远没有机会展示，对应的后端等待会一直卡着（见
    // `confirmQueue` 的文档注释）。
    set((s) => {
      const [next, ...rest] = s.confirmQueue;
      return { confirmRequest: next ?? null, confirmQueue: rest };
    });
    await codingService.confirmCommand(confirmRequest.requestId, allow);
  },

  resolveConfirmAndRemember: async (pattern) => {
    const { confirmRequest } = get();
    if (!confirmRequest) return;
    set((s) => {
      const [next, ...rest] = s.confirmQueue;
      return { confirmRequest: next ?? null, confirmQueue: rest };
    });
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

  loadSkills: async (workspaceId) => {
    try { set({ skills: await skillService.list(workspaceId) }); } catch { /* best effort */ }
  },

  importSkill: async (workspaceId, localPath) => {
    const skill = await skillService.import(workspaceId, localPath);
    set((s) => ({ skills: [...s.skills.filter((sk) => sk.name !== skill.name), skill] }));
    return skill;
  },

  deleteSkill: async (workspaceId, name) => {
    await skillService.delete(workspaceId, name);
    set((s) => ({ skills: s.skills.filter((sk) => sk.name !== name) }));
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

  /** 打开一条历史记录——真正接续对话（不是只读回放），用户 2026-09 反馈"历史
   * 会话只能只读不能继续修改"。`coding_history_resume` 在后端用持久化的真实
   * LLM 消息上下文重建这个工作区的活跃会话，`timeline`/`changesById` 仍然从
   * `historyGet` 拿（纯展示用，后端的 `CodingSessionInfo` 不携带时间线）。
   * `viewingHistoryId` 保持 `null`——这个会话现在是"活的"，输入框/操作按钮
   * 不应该再被当成只读禁用。 */
  openHistory: async (id) => {
    await get().saveCurrentHistory();
    const workspaceId = get().workspaceId;
    if (!workspaceId) return;
    const detail = await codingService.historyGet(id);
    if (!detail) return;
    try {
      const info = await codingService.historyResume(workspaceId, id);
      const changes = detail.changes as FileChange[];
      set({
        workspaceId,
        viewingHistoryId: null,
        sessionInfo: info,
        timeline: detail.timeline as TimelineEntry[],
        changesById: Object.fromEntries(changes.map((c) => [c.id, c])),
        error: null,
      });
    } catch (e) {
      set({ error: formatError(e) });
    }
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
    // `coding_new_session` 失败（比如 provider 被删了）之前没有 catch，button 的
    // onClick 也是 fire-and-forget——异常直接变成一条不可见的 unhandled rejection，
    // 界面上就是"点了没反应，还停在老会话"，用户完全不知道发生了什么
    // （2026-09 用户真实反馈）。这里补上 catch，把错误显示出来。
    try {
      const info = await codingService.newSession(workspaceId, providerId);
      set({ sessionInfo: info, timeline: [], changesById: {}, viewingHistoryId: null, error: null });
    } catch (e) {
      set({ error: formatError(e) });
    }
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
      // 首屏先建可用的空会话；历史同步可能需要从远端读取很多快照，绝不能让它
      // 挡住面板从"开始 AI 会话"切到对话界面。列表和最近会话回显随后异步补上。
      const historiesPromise = codingService.historyList(workspaceId);
      const info = await codingService.newSession(workspaceId, providerId);
      if (get().workspaceId !== workspaceId) return;
      set({ workspaceId, sessionInfo: info, timeline: [], changesById: {}, viewingHistoryId: null, histories: [], error: null });

      void historiesPromise.then(async (histories) => {
        if (get().workspaceId !== workspaceId || get().sessionInfo?.id !== info.id) return;
        set({ histories });
        const latest = histories[0];
        const latestTime = latest ? Date.parse(latest.updated_at) : NaN;
        const recent = latest && Number.isFinite(latestTime) && (Date.now() - latestTime) <= 12 * 60 * 60 * 1000;
        if (!recent) return;
        // 用户还没碰过这个刚建好的空会话才去替换——已经开始在里面说话了就不要
        // 再把它换掉，避免刚打的字/已经发出去的消息被"补历史"这步悄悄顶掉。
        if (get().timeline.length > 0) return;
        // 2026-09 真实复现："进程被杀掉重开后，历史会话里的待确认文件改动点
        // 应用没反应"——根因是这里原来只用 `historyGet` 把 timeline/changes
        // 摆回前端做"看起来接上了"的展示，从来没调 `historyResume` 真正在
        // 后端重建 `CodingSession.messages`/`ChangeStore`。用户点的"应用"发给
        // 的是后端一个全新、空的 `ChangeStore`（`newSession` 建的那个），找不到
        // 时间线上显示的那个 change_id，自然什么反应都没有。改成和 `openHistory`
        // 同一条正确路径：调 `historyResume` 真正续上后端会话，而不是只做前端
        // 展示层面的"贴图"。
        try {
          const resumedInfo = await codingService.historyResume(workspaceId, latest.id);
          if (get().workspaceId !== workspaceId || get().sessionInfo?.id !== info.id) return;
          const detail = await codingService.historyGet(latest.id);
          if (!detail || get().workspaceId !== workspaceId || get().sessionInfo?.id !== info.id) return;
          const changes = detail.changes as FileChange[];
          set({
            sessionInfo: resumedInfo,
            timeline: detail.timeline as TimelineEntry[],
            changesById: Object.fromEntries(changes.map((change) => [change.id, change])),
            error: null,
          });
        } catch {
          // 恢复失败（比如这条历史关联的 Provider 已被删除）不影响已经建好的
          // 新会话，用户可以继续在这个空会话里正常工作，不阻塞整个面板。
        }
      }).catch(() => undefined);
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
      confirmQueue: [],
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
// 图片会以 base64 同时进入前端时间线、后端会话上下文和历史快照；原来的 8MB
// 上限会在编码后变成约 10.7MB，并且每轮工具调用都重复序列化一次，足以把 AI
// 子进程/渲染进程推到 OOM。512KB 对截图提问仍足够，超过时请先压缩图片。
const MAX_IMAGE_BYTES = 512 * 1024;

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

/** 长任务中途做增量存档的最小间隔——`saveCurrentHistory()` 原来只在 `sendMessage()`
 * 整个 `send_message` 后端调用（可能是几十轮工具调用）成功返回之后才触发一次。
 * 如果进程在这中途崩了/被系统杀了（2026-09 用户报告"AI 程序运行着运行着进程自动
 * 退出了"——见 `coding/session.rs` 的 `MAX_TOOL_RESULT_CHARS` 注释，根因是超大
 * 工具结果反复重发导致内存失控被系统直接终止，不会走任何清理/保存逻辑），这一整
 * 轮的进度（含中途已经 stage/accept 的文件改动）完全没有落盘，重启后自然找不到。
 * 这里在"确实产生了新进度"的事件（工具调用完成、文件改动、以及每一轮结束时的
 * `coding:token-usage-summary`——覆盖"这一轮完全没调用工具，纯文本回复"的情况，
 * 见该监听器里的文档）上顺带触发一次存档，用时间节流而不是每个事件都存，避免
 * 快速连续的工具调用把 SQLite/磁盘 I/O 打爆。 */
const HISTORY_CHECKPOINT_THROTTLE_MS = 5000;
let lastHistoryCheckpointAt = 0;
function checkpointHistory() {
  const now = Date.now();
  if (now - lastHistoryCheckpointAt < HISTORY_CHECKPOINT_THROTTLE_MS) return;
  lastHistoryCheckpointAt = now;
  void useCodingStore.getState().saveCurrentHistory();
}

/** 全局注册一次编程助手事件监听（App.tsx 挂载时调用，和 registerAiChatListeners 同款模式）。
 *
 * `listenersRegistered` 只是防止同一次挂载里重复注册，不代表"永远只注册一次"——
 * 之前清理函数（React effect 卸载时调用的那个 unlisten）没有把它重置回
 * `false`，如果这个 effect 曾经卸载又重新挂载过一次（比如 React 18 StrictMode
 * 在开发模式下对每个 effect 故意做的 mount→unmount→mount），第二次注册会被
 * 这个陈旧的 `true` 拦下来直接变成空操作，之后所有 `coding:*` 事件都收不到了。
 * 这是独立于"点确认没反应"（真正根因是 `CodingSession`/`ChangeStore` 锁竞争，
 * 见 `coding/changes.rs`）的另一个潜在缺陷，顺手一起修掉。*/
export function registerCodingListeners(): Promise<() => void> {
  if (listenersRegistered) return Promise.resolve(() => {});
  listenersRegistered = true;

  const currentSessionId = () => useCodingStore.getState().sessionInfo?.id;

  const unlistenPromises = [
    listen<CodingToolCallEvent>("coding:tool-call-start", (event) => {
      if (event.payload.sessionId !== currentSessionId()) return;
      useCodingStore.setState((s) => ({
        timeline: [...s.timeline, { kind: "tool", id: nextId(), tool: event.payload.tool, running: true, detail: event.payload.detail, startedAt: Date.now() }],
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
        if (entry.kind === "tool") timeline[realIdx] = { ...entry, running: false, output: event.payload.output };
        return { timeline };
      });
      checkpointHistory();
    }),
    listen<CodingAssistantNoteEvent>("coding:assistant-note", (event) => {
      if (event.payload.sessionId !== currentSessionId()) return;
      useCodingStore.setState((s) => ({
        timeline: [...s.timeline, event.payload.kind === "status"
          ? { kind: "progress", id: nextId(), text: event.payload.text }
          : { kind: "note", id: nextId(), text: event.payload.text }],
      }));
    }),
    // 单次模型往返的用量——只做"实时覆盖"，不再各自插一条时间线消息（见
    // `liveTokenUsage` 字段文档：之前这里 append 进 timeline，一轮几十次工具
    // 调用就是几十条"本次请求消耗 tokens"，把真正有信息量的进度提示淹没了）。
    listen<CodingTokenUsageEvent>("coding:token-usage", (event) => {
      if (event.payload.sessionId !== currentSessionId()) return;
      useCodingStore.setState({
        liveTokenUsage: {
          promptTokens: event.payload.promptTokens,
          completionTokens: event.payload.completionTokens,
          totalTokens: event.payload.totalTokens,
        },
      });
    }),
    // 一整轮对话（一条用户消息到最终给出结论，中间可能跑了好几次工具调用/API
    // 请求）结束时的汇总，和上面单次请求的 `coding:token-usage` 是两个不同粒度
    // 的事件——单次请求那条只做实时进度参考（不进历史），这条是"这一轮总共花了
    // 多少"，作为唯一进时间线的 usage 记录留存；同时清空 `liveTokenUsage`，
    // 避免这一轮的数字残留到下一轮开始之前的空档期里。
    listen<CodingTokenUsageEvent>("coding:token-usage-summary", (event) => {
      if (event.payload.sessionId !== currentSessionId()) return;
      useCodingStore.setState((s) => ({
        liveTokenUsage: null,
        timeline: [
          ...s.timeline,
          {
            kind: "usage",
            id: nextId(),
            promptTokens: event.payload.promptTokens,
            completionTokens: event.payload.completionTokens,
            totalTokens: event.payload.totalTokens,
            isTurnTotal: true,
          },
        ],
      }));
      // 这一轮对话结束了（不管中间有没有调用工具）——之前只在"工具调用完成/
      // 文件改动"上存档，遗漏了"模型直接给一段纯文本回复、这一轮完全没调工具"
      // 这种情况（比如只是回答/解释方案，不产生任何 tool-call-end/file-change
      // 事件）。2026-09 真实复现：这类纯文本回复因为从没被存档，进程一旦被杀掉
      // 重启（哪怕是几分钟后另一次不相关的操作杀的），resume 恢复到的还是这条
      // 回复之前的旧存档——用户接着说"按刚才说的方案改"，AI 上下文里根本没有
      // "刚才"那段内容，只能一脸茫然地要用户重新贴一遍。`coding:token-usage-summary`
      // 每轮不管有没有工具调用都必然触发一次，是"这一轮真的结束了"最可靠的信号。
      checkpointHistory();
    }),
    listen<CodingFileChangeEvent>("coding:file-change", (event) => {
      if (event.payload.sessionId !== currentSessionId()) return;
      const change = event.payload.change;
      useCodingStore.setState((s) => ({
        changesById: { ...s.changesById, [change.id]: change },
        timeline: [...s.timeline, { kind: "change", id: nextId(), changeId: change.id }],
      }));
      // "完全授权模式"下这条改动已经直接落盘（change.status 一进来就是
      // "applied"）——同步刷新这个路径可能已经打开的编辑器 buffer，否则磁盘
      // 内容变了、编辑器里显示的还是旧内容（和 acceptChange 里的处理一致）。
      if (event.payload.sync) {
        const sync = event.payload.sync;
        useEditorStore.getState().syncExternalWrite(sync.path, sync.content, sync.mtime);
      }
      checkpointHistory();
    }),
    listen<CodingCommandBlockedEvent>("coding:command-blocked", (event) => {
      if (event.payload.sessionId !== currentSessionId()) return;
      useCodingStore.setState((s) => ({
        timeline: [...s.timeline, { kind: "blocked", id: nextId(), command: event.payload.command }],
      }));
    }),
    listen<CodingCommandConfirmRequestEvent>("coding:command-confirm-request", (event) => {
      if (event.payload.sessionId !== currentSessionId()) return;
      const incoming: CommandConfirmRequest = {
        requestId: event.payload.requestId,
        command: event.payload.command,
        host: event.payload.host,
        kind: event.payload.kind ?? "command",
        matchKey: event.payload.matchKey,
      };
      // AI 引擎可能并发发起多个工具调用，每个都各自触发一次这个事件——
      // 已经有一个在展示的话排到队列末尾，不能直接覆盖 `confirmRequest`，
      // 否则被覆盖那个在后端永远等不到回应（见 `confirmQueue` 的文档注释，
      // 2026-09 用户实测复现的"AI 工作卡住不回应"）。
      useCodingStore.setState((s) =>
        s.confirmRequest
          ? { confirmQueue: [...s.confirmQueue, incoming] }
          : { confirmRequest: incoming }
      );
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
    // 用户点"应用/拒绝"把这一轮提议的改动都处理完之后，后端自动发起的续跑轮次——
    // 和手动 `sendMessage` 表现得一致：置 `sending`（禁用输入框/显示"停止"按钮），
    // 时间线里插一条说明，不是静默在后台跑（2026-09 用户反馈：点了应用后 AI 像
    // 没反应一样，必须再发一条消息才会继续）。
    listen<CodingAutoContinueStartEvent>("coding:auto-continue-start", (event) => {
      if (event.payload.sessionId !== currentSessionId()) return;
      useCodingStore.setState((s) => ({
        sending: true,
        timeline: [...s.timeline, { kind: "note", id: nextId(), text: event.payload.note }],
      }));
    }),
    listen<CodingAutoContinueDoneEvent>("coding:auto-continue-done", (event) => {
      if (event.payload.sessionId !== currentSessionId()) return;
      // 用户在自动续跑进行中点了"停止"——和手动发送里的取消处理走同一套展示
      // （不算真正的错误，不弹红条；清掉还转圈的工具条目，否则永远等不到
      // `coding:tool-call-end` 补上）。
      if (event.payload.error?.includes("已停止：用户取消了当前对话轮次")) {
        useCodingStore.setState((s) => ({
          sending: false,
          timeline: [
            ...s.timeline.map((t) => (t.kind === "tool" && t.running ? { ...t, running: false } : t)),
            { kind: "note", id: nextId(), text: "已停止" },
          ],
        }));
        return;
      }
      useCodingStore.setState((s) => ({
        sending: false,
        timeline: event.payload.reply
          ? [...s.timeline, { kind: "assistant", id: nextId(), text: event.payload.reply }]
          : event.payload.error
            ? [...s.timeline, { kind: "note", id: nextId(), text: `自动继续失败：${event.payload.error}` }]
            : s.timeline,
      }));
      void useCodingStore.getState().saveCurrentHistory();
    }),
  ];

  return Promise.all(unlistenPromises).then((unlistens) => () => {
    listenersRegistered = false;
    unlistens.forEach((u) => u());
  });
}
