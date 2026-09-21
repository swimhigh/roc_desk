import React, { useEffect, useLayoutEffect, useRef, useState } from "react";
import { Send, Bot, User, GitCommitHorizontal, Brain, ChevronRight, Sparkles, Settings, History, Plus, ShieldCheck, Plug, Blocks, BookOpen, CircleDot, CircleCheck, Circle, Paperclip, Wand2, X, Square } from "lucide-react";
import { useCodingStore, type TimelineEntry } from "../../stores/codingStore";
import { useExternalFileDrop } from "../../hooks/useExternalFileDrop";
import { formatTokenCount } from "../../utils/formatTokens";
import { useAiChatStore } from "../../stores/aiChatStore";
import { useEditorStore } from "../../stores/editorStore";
import { detectLanguage } from "../../utils/language";
import { SegmentedControl } from "../shared/SegmentedControl";
import { ToggleSwitch } from "../shared/ToggleSwitch";
import { TargetBadge } from "./TargetBadge";
import { RemoteCapabilityBadge } from "./RemoteCapabilityBadge";
import { ToolCallProgress, toolLabel } from "./ToolCallProgress";
import { FileChangeCard, type DiffLine as CardDiffLine } from "./FileChangeCard";
import { CommandConfirmDialog, BlockedCommandMessage } from "./CommandConfirmDialog";
import { QuestionDialog } from "./QuestionDialog";
import { PermissionRulesDialog } from "./PermissionRulesDialog";
import { McpServerManagerDialog } from "./McpServerManagerDialog";
import { SkillManagerDialog } from "./SkillManagerDialog";
import { AgentMarkdown } from "./AgentMarkdown";
import { ProviderManagerDialog, hasProviderDraft } from "../AiChat/ProviderManagerDialog";
import { CodingHistoryDialog } from "./CodingHistoryDialog";
import { useToastStore } from "../shared/Toast";
import { formatError } from "../../utils/error";
import type { CodingTarget, FileChange, TodoStatus } from "../../types/bindings";

/** 截断预览文字用——折叠行/表格视图里只需要看个大概，完整内容在弹框详情里。 */
function truncatePreview(text: string, max: number): string {
  const flat = text.replace(/\s+/g, " ").trim();
  return flat.length > max ? `${flat.slice(0, max)}…` : flat;
}

/** 一轮对话（从一条 "user" 条目到下一条 "user" 条目之前）的摘要——折叠行/表格
 * 视图用它渲染"用户问的什么、AI 最后说了什么、这一轮有多少工具调用/文件改动"，
 * 不需要展开整轮明细就能看出这一轮大致做了什么。 */
function summarizeRound(items: { entry: TimelineEntry }[]): {
  userPreview: string;
  answerPreview: string;
  toolCount: number;
  changeCount: number;
  totalTokens: number | null;
} {
  let userText = "";
  let lastAssistantText = "";
  let lastNoteText = "";
  let toolCount = 0;
  let changeCount = 0;
  let blockedCount = 0;
  let totalTokens: number | null = null;
  for (const { entry } of items) {
    if (entry.kind === "user") userText = entry.text;
    else if (entry.kind === "assistant") lastAssistantText = entry.text;
    else if (entry.kind === "note") lastNoteText = entry.text;
    else if (entry.kind === "tool") toolCount += 1;
    else if (entry.kind === "change") changeCount += 1;
    else if (entry.kind === "blocked") blockedCount += 1;
    // 折叠行只展示"这一轮总共花了多少"，不是每次请求各自的消耗——同一轮里
    // 工具循环可能打好几次请求，每次都会有一条非 isTurnTotal 的 usage 条目，
    // 只有 isTurnTotal 那一条才是整轮的合计（2026-09 用户需求：每一轮完成后
    // 显示这一轮消耗的 token 数）。
    else if (entry.kind === "usage" && entry.isTurnTotal) totalTokens = entry.totalTokens;
  }
  const answerSource = lastAssistantText || lastNoteText || (blockedCount > 0 ? "命令被拦截" : "");
  return {
    userPreview: userText ? truncatePreview(userText, 60) : "（无文字，仅附件）",
    answerPreview: answerSource ? truncatePreview(answerSource, 80) : "（进行中或没有文字回复）",
    toolCount,
    changeCount,
    totalTokens,
  };
}

/** 折叠行的批量操作条要展示的数据——按后端真实的 `turn_id` 分组（不是按前端
 * 的"轮次"，见 `roundGroups` 的文档：插话消息可能让一轮里出现多个 `turn_id`
 * 之外的情况反过来也存在），一个 `turn_id` 通常就对应这一行看到的这一轮，但
 * 遇到插话的边界情况时可能对应不止一组——挨个渲染，不假设只有一组。 */
function computeTurnBatches(
  items: { entry: TimelineEntry }[],
  changesById: Record<string, FileChange>,
  turnStats: Map<string, { changeIds: string[]; lastEntryId: string }>,
): Array<{ turnId: string; pendingInTurn: FileChange[]; appliedInTurn: FileChange[] }> {
  const seenTurnIds = new Set<string>();
  const batches: Array<{ turnId: string; pendingInTurn: FileChange[]; appliedInTurn: FileChange[] }> = [];
  for (const { entry } of items) {
    if (entry.kind !== "change") continue;
    const change = changesById[entry.changeId];
    if (!change || seenTurnIds.has(change.turn_id)) continue;
    seenTurnIds.add(change.turn_id);
    const stat = turnStats.get(change.turn_id);
    const turnChanges = (stat?.changeIds ?? []).map((id) => changesById[id]).filter((c): c is FileChange => Boolean(c));
    batches.push({
      turnId: change.turn_id,
      pendingInTurn: turnChanges.filter((c) => c.status === "pending"),
      appliedInTurn: turnChanges.filter((c) => c.status === "applied"),
    });
  }
  return batches;
}

function todoIcon(status: TodoStatus) {
  if (status === "completed") return <CircleCheck style={{ width: 13, height: 13, color: "var(--accent)" }} />;
  if (status === "in_progress") return <CircleDot style={{ width: 13, height: 13, color: "var(--warning)" }} />;
  return <Circle style={{ width: 13, height: 13, color: "var(--text-secondary)" }} />;
}

/** `run_command` 的"记住此模式"默认建议：取命令第一个词 + " *"（如 `npm install`
 * → `npm *`），比逐字精确匹配更实用——大多数场景是"这一类命令都想放行"，不是
 * "只放行一模一样的这一条"。用户仍然可以在弹窗里把它改成任意通配模式。 */
function suggestCommandPattern(command: string): string {
  const first = command.trim().split(/\s+/)[0];
  return first ? `${first} *` : command;
}

interface CodingAgentPanelProps {
  workspaceId: string;
  active: boolean;
  onOpenFile?: (path: string, line?: number) => void;
}

function targetLabel(target: CodingTarget): string {
  return target.kind === "Local" ? "本地" : target.host_label;
}

/**
 * AI 工具主面板（DESIGN.md §3.8.1）。普通问答与编程任务共用同一个会话入口；
 * Plan 模式适合问答、解释与方案分析，Build 模式在用户需要时再读写工作区。
 * DESIGN.md 草图是三栏布局（Explorer +
 * 对话 + 右栏 Monaco 实时编辑器），这里裁剪掉了独立的右栏编辑器——文件改动的 Diff
 * 已经用 FileChangeCard 内嵌展示，维护两份 Monaco 状态同步在 MVP 阶段收益不高，
 * 属于本轮有意识的范围裁剪（REQUIREMENTS.md §3.7 有记录）。左栏复用的是
 * App.tsx 里已经常驻的全局 Explorer，不在这个组件里重复渲染。
 */
export const ThinkingBlock: React.FC<{ text: string; active: boolean; onOpenFile?: (path: string, line?: number) => void }> = ({ text, active, onOpenFile }) => {
  const detailsRef = useRef<HTMLDetailsElement>(null);
  useEffect(() => {
    if (detailsRef.current) detailsRef.current.open = active;
  }, [active]);
  return (
    <details ref={detailsRef} className="agent-thinking">
      <summary><ChevronRight className="agent-thinking-chevron" /><Brain /> <span>{active ? "正在思考" : "思考过程"}</span></summary>
      <div className="agent-thinking-body"><AgentMarkdown content={text} onOpenFile={onOpenFile} /></div>
    </details>
  );
};

export const CodingAgentPanel: React.FC<CodingAgentPanelProps> = ({ workspaceId, active, onOpenFile }) => {
  const {
    sessionInfo,
    timeline,
    changesById,
    sending,
    liveTokenUsage,
    error,
    confirmRequest,
    questionRequest,
    start,
    setMode,
    setProvider,
    setAutoAllowReadonly,
    setAutoGitCommit,
    setFullAuto,
    setAutoApplyChanges,
    sendMessage,
    cancelTurn,
    attachments,
    addAttachments,
    addAttachmentsFromPaths,
    removeAttachment,
    optimizing,
    optimizePrompt,
    acceptChange,
    rejectChange,
    undoChange,
    toggleToolOutput,
    revertTurn,
    resolveConfirm,
    resolveConfirmAndRemember,
    answerQuestion,
    histories,
    viewingHistoryId,
    loadHistories,
    openHistory,
    deleteHistory,
    renameHistory,
    newSession,
  } = useCodingStore();
  const providers = useAiChatStore((s) => s.providers);
  const loadProviders = useAiChatStore((s) => s.loadProviders);
  const modelsByProvider = useAiChatStore((s) => s.modelsByProvider);
  const fetchModels = useAiChatStore((s) => s.fetchModels);
  const updateProvider = useAiChatStore((s) => s.updateProvider);
  const push = useToastStore((s) => s.push);
  // Keep keystrokes out of the large timeline render tree.  The composer is a
  // native uncontrolled textarea; React only receives a debounced snapshot for
  // button state/optimization, while send always reads the latest ref value.
  const [input, setInput] = useState("");
  const inputValueRef = useRef("");
  const inputElementRef = useRef<HTMLTextAreaElement>(null);
  const inputSyncTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const [selectedProviderId, setSelectedProviderId] = useState<string>("");
  const [showProviders, setShowProviders] = useState(false);
  const [providerDraftPending, setProviderDraftPending] = useState(() => hasProviderDraft());
  const [showHistory, setShowHistory] = useState(false);
  const [showPermissionRules, setShowPermissionRules] = useState(false);
  const [showMcpServers, setShowMcpServers] = useState(false);
  const [showSkills, setShowSkills] = useState(false);
  // 2026-09 用户反馈："多轮后拉得太长了"/"折叠了也没法再收起来"/"每一轮的问答
  // 结果也应该默认折叠，不止是工具调用细节"——默认只有"当前/最新这一轮"完整
  // 展开（用户问题 + 过程 + AI 最终答复），更早的轮次统一折叠成一行摘要（表格
  // 式的一行：轮次号 + 用户问题预览 + AI 答复预览 + 工具/改动计数），点这一行
  // 用弹框看完整明细，不再有"点开之后没法收回去"的中间态需要管理。
  const [detailRound, setDetailRound] = useState<number | null>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const shouldFollowBottomRef = useRef(true);
  const stableScrollTopRef = useRef(0);
  const suppressScrollEventRef = useRef(false);
  const scrollingRef = useRef(false);
  const scrollEndTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const [showJumpBottom, setShowJumpBottom] = useState(false);
  const showJumpBottomRef = useRef(false);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const composerRef = useRef<HTMLDivElement>(null);
  const { isDragOver: draggingAttachments } = useExternalFileDrop(composerRef, (paths) => {
    void addAttachmentsFromPaths(paths);
  });

  // 用户反馈"看不出来 AI 还有没有在运行"——工具调用之间（模型正在琢磨下一步、
  // 还没吐出新的事件）时间线上什么新条目都不会出现，界面看起来和"已经彻底
  // 结束"一模一样。这里只是个每 500ms 触发一次重渲染的计时器（`sending` 为
  // true 时才跑），让下面固定在消息列表底部、composer 上方的状态条能持续
  // 更新——不依赖任何新的后端事件，纯前端"心跳"。
  const [liveTick, setLiveTick] = useState(0);
  useEffect(() => {
    if (!sending) return;
    const timer = setInterval(() => setLiveTick((t) => t + 1), 500);
    return () => clearInterval(timer);
  }, [sending]);

  const restoredWorkspaceRef = useRef<string | null>(null);

  useEffect(() => {
    loadProviders();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (!active) {
      restoredWorkspaceRef.current = null;
      return;
    }
    if (providers.length === 0) {
      restoredWorkspaceRef.current = null;
      return;
    }
    if (restoredWorkspaceRef.current === workspaceId) return;
    restoredWorkspaceRef.current = workspaceId;
    const providerId = selectedProviderId || providers[0].id;
    useCodingStore.getState().restoreOrStart(workspaceId, providerId);
  }, [active, workspaceId, providers, selectedProviderId]);

  useEffect(() => {
    if (!active || !sessionInfo || providers.length === 0) return;
    if (providers.some((provider) => provider.id === sessionInfo.provider_id)) return;
    const fallback = providers.find((provider) => provider.id === selectedProviderId) ?? providers[0];
    // A deleted/recreated provider gets a new UUID. Rebind an already-open coding
    // session immediately instead of leaving it pointed at the removed UUID.
    void useCodingStore.getState().setProvider(fallback.id);
  }, [active, providers, selectedProviderId, sessionInfo]);

  // 2026-09 需求：每次 AI 会话启动、或者切换到另一个 Provider 时，重新拉取一遍
  // 这个 Provider 的模型列表——不长期缓存陈旧列表，Provider 侧新上线的模型下次
  // 打开就能选到。`fetchModels` 内部还负责"当前默认模型不在新拉到的列表里就自动
  // 选第一个"（比如第一次配置好一个 Provider、还没手动选过模型的场景）。
  useEffect(() => {
    if (!active || !sessionInfo?.provider_id) return;
    void fetchModels(sessionInfo.provider_id);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, sessionInfo?.provider_id]);

  useEffect(() => {
    if (providers.length > 0 && !providers.some((provider) => provider.id === selectedProviderId)) {
      setSelectedProviderId(providers[0].id);
    }
  }, [providers, selectedProviderId]);

  useLayoutEffect(() => {
    const list = listRef.current;
    if (!list) return;
    const maxScrollTop = Math.max(0, list.scrollHeight - list.clientHeight);
    if (shouldFollowBottomRef.current) {
      suppressScrollEventRef.current = true;
      list.scrollTop = maxScrollTop;
      stableScrollTopRef.current = list.scrollTop;
      requestAnimationFrame(() => { suppressScrollEventRef.current = false; });
    } else if (list.scrollTop > maxScrollTop) {
      // The timeline may shrink when a session is restored or old steps collapse.
      // Clamp stale scrollTop instead of leaving a blank viewport past the content.
      list.scrollTop = maxScrollTop;
      stableScrollTopRef.current = maxScrollTop;
    }
  }, [timeline]);

  // Markdown/图片布局可能在 React 提交后继续改变高度。只有用户明确停在底部时
  // 才跟随这些变化；阅读旧消息时绝不重设 scrollTop，避免新内容把阅读位置抢走。
  useEffect(() => {
    const list = listRef.current;
    if (!list) return;
    let frame = 0;
    const target = list.querySelector<HTMLElement>(".agent-timeline-content") ?? list;
    const observer = new ResizeObserver(() => {
      if (!shouldFollowBottomRef.current || scrollingRef.current) return;
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        list.scrollTop = Math.max(0, list.scrollHeight - list.clientHeight);
      });
    });
    observer.observe(target);
    return () => { cancelAnimationFrame(frame); observer.disconnect(); };
  }, []);

  const handleTimelineScroll = () => {
    const list = listRef.current;
    if (!list) return;
    if (suppressScrollEventRef.current) return;
    scrollingRef.current = true;
    if (scrollEndTimerRef.current) clearTimeout(scrollEndTimerRef.current);
    scrollEndTimerRef.current = setTimeout(() => {
      scrollingRef.current = false;
      if (shouldFollowBottomRef.current && listRef.current) {
        listRef.current.scrollTop = Math.max(0, listRef.current.scrollHeight - listRef.current.clientHeight);
      }
    }, 120);
    const distance = list.scrollHeight - list.scrollTop - list.clientHeight;
    // 只有已经在底部附近才跟随流式输出。用户向上滚动超过阈值后，
    // 新的工具事件不会再改变当前阅读位置。
    const follow = distance <= 48;
    shouldFollowBottomRef.current = follow;
    stableScrollTopRef.current = list.scrollTop;
    const nextShowJump = !follow;
    if (showJumpBottomRef.current !== nextShowJump) {
      showJumpBottomRef.current = nextShowJump;
      setShowJumpBottom(nextShowJump);
    }
  };

  const scrollTimelineToBottom = () => {
    const list = listRef.current;
    if (!list) return;
    shouldFollowBottomRef.current = true;
    showJumpBottomRef.current = false;
    setShowJumpBottom(false);
    // `behavior: "smooth"` 的滚动动画本身会连续触发一串中间态 scroll 事件；
    // `handleTimelineScroll` 如果照常处理这些事件，会在动画还没真正到底部时
    // 就按"距离底部还有 48px 以上"判定成"用户自己划走了"，把刚设的
    // `shouldFollowBottomRef`/`showJumpBottom` 都翻回去——这正是"点了按钮却
    // 没有下压到最新"的根因：这次滚动是按钮自己触发的，不该被它自己触发的
    // 滚动事件反过来判定成"用户手动离开了底部"。用 `suppressScrollEventRef`
    // 盖住整段动画期间的 scroll 事件，动画结束（`scrollend`，不支持的浏览器
    // 退回一个覆盖动画时长的定时器）后再放开。
    suppressScrollEventRef.current = true;
    list.scrollTo({ top: Math.max(0, list.scrollHeight - list.clientHeight), behavior: "smooth" });
    const clearSuppress = () => { suppressScrollEventRef.current = false; };
    if ("onscrollend" in list) {
      list.addEventListener("scrollend", clearSuppress, { once: true });
    } else {
      setTimeout(clearSuppress, 500);
    }
  };

  // 按 `turn_id`（同一条用户消息触发的一轮工具调用）给 FileChangeCard 分组——
  // 记下每一轮最后一张卡片在时间线里的 id，只在那张卡片后面挂一次"全部应用/
  // 全部拒绝/撤销本轮"的批量操作条，而不是每张卡片都重复一份（参考 Cursor/
  // Windsurf 按对话轮次做批量操作，但这里不依赖 git，见项目内部设计讨论）。
  // 每个时间线条目属于第几轮对话——以 "user" 条目为轮次边界，和后端
  // `current_turn_id`/`turn_id` 是同一个"一条用户消息 = 一轮"的概念，只是这里
  // 不需要真的按 turn_id 关联，纯按顺序数第几个 "user" 条目就够用。
  const roundOfIndex = React.useMemo(() => {
    const rounds: number[] = [];
    let round = -1;
    for (const entry of timeline) {
      if (entry.kind === "user") round += 1;
      rounds.push(round);
    }
    return rounds;
  }, [timeline]);
  const currentRound = roundOfIndex.length > 0 ? roundOfIndex[roundOfIndex.length - 1] : -1;

  // 按 `roundOfIndex` 把时间线切成一段一段——只用来决定"这一轮要不要默认
  // 展开"，不是按后端 `turn_id` 分组（那是 `turnStats` 的职责，两者概念不同：
  // 插话消息插进正在跑的轮次时，前端会多出一条新的 "user" 气泡、多算一轮，
  // 但后端可能仍然算同一个 `turn_id`——批量操作按钮因此继续按 `turnStats`
  // 计算，不能直接假设"一轮"就对应"一个 turn_id"）。
  const roundGroups = React.useMemo(() => {
    const groups: { round: number; items: { entry: TimelineEntry; index: number }[] }[] = [];
    timeline.forEach((entry, index) => {
      const round = roundOfIndex[index];
      const last = groups[groups.length - 1];
      if (last && last.round === round) {
        last.items.push({ entry, index });
      } else {
        groups.push({ round, items: [{ entry, index }] });
      }
    });
    return groups;
  }, [timeline, roundOfIndex]);

  const turnStats = React.useMemo(() => {
    const stats = new Map<string, { changeIds: string[]; lastEntryId: string }>();
    for (const entry of timeline) {
      if (entry.kind !== "change") continue;
      const change = changesById[entry.changeId];
      if (!change) continue;
      const existing = stats.get(change.turn_id);
      if (existing) {
        existing.changeIds.push(change.id);
        existing.lastEntryId = entry.id;
      } else {
        stats.set(change.turn_id, { changeIds: [change.id], lastEntryId: entry.id });
      }
    }
    return stats;
  }, [timeline, changesById]);

  // 切工作区/切历史会话时关掉还开着的轮次详情弹框——round 编号在新的时间线里
  // 完全是另一回事，留着旧的 `detailRound` 可能凑巧命中一个无关的轮次。
  useEffect(() => {
    setDetailRound(null);
  }, [workspaceId, viewingHistoryId]);

  useEffect(() => {
    if (detailRound === null) return;
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") setDetailRound(null);
    };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [detailRound]);

  const handleStart = () => {
    if (!selectedProviderId) return;
    start(workspaceId, selectedProviderId);
  };

  const handleSend = () => {
    const value = inputValueRef.current;
    if (!value.trim() && attachments.length === 0) return;
    sendMessage(value);
    inputValueRef.current = "";
    if (inputElementRef.current) inputElementRef.current.value = "";
    setInput("");
  };

  const handleOptimize = async () => {
    const value = inputValueRef.current;
    if (!value.trim() || optimizing) return;
    const optimized = await optimizePrompt(value);
    inputValueRef.current = optimized;
    if (inputElementRef.current) inputElementRef.current.value = optimized;
    setInput(optimized);
  };

  const handleFilesSelected = (files: FileList | null) => {
    if (!files || files.length === 0) return;
    addAttachments(Array.from(files));
    if (fileInputRef.current) fileInputRef.current.value = "";
  };

  /** 输入框直接 Ctrl+V 粘贴图片（截图工具/浏览器"复制图片"都是这个路径）走和
   * 点回形针选文件同一套 `addAttachments`。只在剪贴板里确实带文件时才
   * `preventDefault()`——普通文本粘贴不会有 `kind === "file"` 的条目，不拦截，
   * 交给浏览器正常处理，两种粘贴不冲突。 */
  const handlePaste = (e: React.ClipboardEvent<HTMLTextAreaElement>) => {
    const items = e.clipboardData?.items;
    if (!items || items.length === 0) return;
    const files = Array.from(items)
      .filter((item) => item.kind === "file")
      .map((item) => item.getAsFile())
      .filter((file): file is File => file !== null);
    if (files.length === 0) return;
    e.preventDefault();
    addAttachments(files);
  };

  if (!sessionInfo) {
    return (
      <div style={{ display: "flex", flexDirection: "column", alignItems: "center", justifyContent: "center", height: "100%", gap: 12 }}>
        {providers.length === 0 ? (
          <>
            <div style={{ fontSize: 13, color: "var(--text-secondary)" }}>需要先配置一个 AI Provider</div>
            <button className="btn primary sm" onClick={() => setShowProviders(true)}>
              <Settings style={{ width: 14, height: 14 }} /> {providerDraftPending ? "继续配置" : "配置模型"}
            </button>
            {showProviders && <ProviderManagerDialog onClose={(hasDraft) => { setShowProviders(false); setProviderDraftPending(hasDraft); }} />}
          </>
        ) : (
          <>
            <select className="form-select" value={selectedProviderId} onChange={(e) => setSelectedProviderId(e.target.value)}>
              {providers.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.name}
                </option>
              ))}
            </select>
            <button className="btn primary sm" onClick={handleStart}>
              开始 AI 会话
            </button>
            {/* `restoreOrStart`（打开工作区时自动触发）失败时只会把 `error` 塞进
                store，不会抛异常给调用方——之前这个分支完全不渲染 `error`，
                自动启动失败时界面表现和"从来没启动过、等你手动点"一模一样，
                用户完全看不出后端到底出没出错、出的什么错（2026-09 用户反馈：
                远程工作区打开后一直停在这个界面，实际是自动启动超时失败了，
                但没有任何提示）。 */}
            {error && <div style={{ padding: "0 12px", fontSize: 12, color: "var(--danger)", textAlign: "center", maxWidth: 320 }}>{error}</div>}
          </>
        )}
      </div>
    );
  }

  // "全部应用/全部拒绝"批量操作——2026-09 用户反馈"改了 3 个文件，点全部应用要
  // 点 3 次，每点一次数量减少一个"：根因是原来的 `for...of` 循环里
  // `await acceptChange(c.id)` 没有任何错误处理，其中一个改动应用失败（比如
  // 两个改动碰巧动了同一个文件、后一个的 diff 基于的旧内容已经因为前一个改动
  // 生效而过期）就会让整个循环直接中断在那一项，剩下的改动完全没被尝试，
  // 也没有任何错误提示——用户只看到"数量少了一个"，只能又点一次重试剩下的。
  // 现在改成每一项单独 try/catch，一个失败不影响继续处理其它项，处理完之后
  // 用 toast 汇总报告失败的数量和原因，不会再静默吞掉错误。
  const runBatch = async (items: FileChange[], action: (id: string) => Promise<void>, verb: string) => {
    let failed = 0;
    let lastError: unknown = null;
    for (const item of items) {
      try {
        await action(item.id);
      } catch (e) {
        failed += 1;
        lastError = e;
      }
    }
    if (failed > 0) {
      push("error", `${items.length} 个改动中有 ${failed} 个${verb}失败：${formatError(lastError)}`);
    }
  };

  const isRemote = sessionInfo.target.kind === "Remote";
  const activeProvider = providers.find((provider) => provider.id === sessionInfo.provider_id);
  const activeThinkingId = [...timeline].reverse().find((entry) => entry.kind === "note")?.id;
  const hasDraft = providerDraftPending || hasProviderDraft();

  // 固定在消息列表底部、composer 上方（不随滚动消失）的"AI 是否还在运行"状态条——
  // 和时间线里内嵌的 ToolCallProgress/ThinkingBlock 不一样，这条不会被滚动
  // 滚出视野，也覆盖"两个事件之间的空档期"（模型正在生成下一段回复但还没有
  // 任何新事件推过来，这时候时间线看起来和"已经彻底结束"完全一样）。
  const lastEntry = timeline[timeline.length - 1];
  const liveStatusText = !sending
    ? null
    : lastEntry?.kind === "tool" && lastEntry.running
    ? `正在执行 ${toolLabel(lastEntry.tool)}${lastEntry.detail ? ` · ${lastEntry.detail}` : ""}`
    : lastEntry?.kind === "note"
    ? "AI 正在思考…"
    : "AI 正在处理…";
  // 本轮进行中的 token 用量——原地跟着这条状态一起展示，不再各自插一条时间线
  // 消息（见 `liveTokenUsage` 字段文档，2026-09 用户反馈）。
  const liveTokenText = sending && liveTokenUsage ? `已用 ${formatTokenCount(liveTokenUsage.totalTokens)} tokens` : null;

  // 单条时间线条目的渲染——当前这一轮内联展开、和轮次详情弹框共用同一份逻辑
  // （不重复维护两套 JSX）。"change" 分支的批量操作条仍然按 `turnStats`（真实
  // 的后端 turn_id）判断，只在这个 turn_id 最后一张卡片后面出现一次，逻辑和
  // 改动前完全一样，只是从"整条时间线的 .map"里搬出来成了一个独立函数。
  const renderEntry = (entry: TimelineEntry): React.ReactNode => {
    if (entry.kind === "user" || entry.kind === "assistant") {
      return (
        <div key={entry.id} style={{ display: "flex", gap: 8, alignItems: "flex-start" }}>
          <div
            style={{
              width: 22, height: 22, borderRadius: "50%", flexShrink: 0,
              display: "flex", alignItems: "center", justifyContent: "center",
              background: entry.kind === "user" ? "var(--bg-hover)" : "var(--accent-dim)",
              color: entry.kind === "user" ? "var(--text-secondary)" : "var(--accent)",
            }}
          >
            {entry.kind === "user" ? <User style={{ width: 13, height: 13 }} /> : <Bot style={{ width: 13, height: 13 }} />}
          </div>
          <div className={`agent-message ${entry.kind}`}>
            {entry.kind === "assistant" ? (
              <AgentMarkdown content={entry.text} onOpenFile={onOpenFile} />
            ) : (
              <>
                {entry.text && <div className="agent-user-text">{entry.text}</div>}
                {entry.attachments && entry.attachments.length > 0 && (
                  <div className="agent-attachment-list">
                    {entry.attachments.map((att, idx) =>
                      att.kind === "image" && att.previewUrl ? (
                        <img key={idx} src={att.previewUrl} alt={att.name} className="agent-attachment-thumb" title={att.name} />
                      ) : (
                        <span key={idx} className="agent-attachment-chip" title={att.name}>
                          <Paperclip style={{ width: 11, height: 11 }} /> {att.name}
                        </span>
                      )
                    )}
                  </div>
                )}
              </>
            )}
          </div>
        </div>
      );
    }
    if (entry.kind === "tool") {
      const hasFileTarget = ["read_file", "write_file", "edit_file", "list_directory", "multi_edit"].includes(entry.tool);
      return (
        <ToolCallProgress
          key={entry.id}
          tool={entry.tool}
          elapsedMs={entry.running && entry.startedAt ? Date.now() - entry.startedAt : 0}
          done={!entry.running}
          detail={entry.detail}
          onOpenFile={hasFileTarget && entry.detail && onOpenFile ? () => onOpenFile(entry.detail!) : undefined}
          output={entry.output}
          expanded={entry.expanded}
          onToggleOutput={() => toggleToolOutput(entry.id)}
        />
      );
    }
    if (entry.kind === "note") {
      // 工具调用之间模型顺带写的说明文字，不是最终答案——样式上比正式回复
      // 弱化（更小字号、次要文字色、无头像），提示"这是过程中的想法"。
      return <ThinkingBlock key={entry.id} text={entry.text} active={sending && entry.id === activeThinkingId} onOpenFile={onOpenFile} />;
    }
    if (entry.kind === "progress") {
      return <div key={entry.id} className="agent-progress-note"><Sparkles /><span>{entry.text}</span></div>;
    }
    if (entry.kind === "blocked") {
      return <BlockedCommandMessage key={entry.id} command={entry.command} />;
    }
    if (entry.kind === "git") {
      return (
        <div key={entry.id} style={{ fontSize: 12, color: "var(--text-secondary)", display: "flex", gap: 6, alignItems: "flex-start" }}>
          <GitCommitHorizontal style={{ width: 14, height: 14, flexShrink: 0, marginTop: 2 }} />
          <div>
            <div>Git 提交 {entry.path}</div>
            <pre style={{ margin: 0, fontFamily: "var(--font-mono)", whiteSpace: "pre-wrap", fontSize: 11 }}>{entry.output}</pre>
          </div>
        </div>
      );
    }
    if (entry.kind === "usage") {
      return entry.isTurnTotal ? (
        <div key={entry.id} style={{ fontSize: 12, color: "var(--text-secondary)", fontWeight: 600, padding: "2px 0" }}>
          本轮对话共消耗 tokens：输入 {formatTokenCount(entry.promptTokens)} · 输出 {formatTokenCount(entry.completionTokens)} · 合计{" "}
          {formatTokenCount(entry.totalTokens)}
        </div>
      ) : (
        <div key={entry.id} style={{ fontSize: 11, color: "var(--text-secondary)", opacity: 0.65 }}>
          本次请求消耗 tokens：输入 {formatTokenCount(entry.promptTokens)} · 输出 {formatTokenCount(entry.completionTokens)} · 合计{" "}
          {formatTokenCount(entry.totalTokens)}
        </div>
      );
    }
    const change = changesById[entry.changeId];
    if (!change) return null;
    const diff: CardDiffLine[] = change.diff.map((l) => ({ sign: l.sign, content: l.content }));
    const status = change.status === "undone" ? "rejected" : change.status;
    const stat = turnStats.get(change.turn_id);
    const turnChanges = (stat?.changeIds ?? []).map((id) => changesById[id]).filter((c): c is FileChange => Boolean(c));
    const pendingInTurn = turnChanges.filter((c) => c.status === "pending");
    const appliedInTurn = turnChanges.filter((c) => c.status === "applied");
    // 单文件改动同样是一轮完整的 AI 操作，必须提供"撤销本轮"入口；之前
    // 误加了 `turnChanges.length > 1`，导致最常见的单文件修改只能看到卡片
    // 级操作，无法按轮回退，也让端到端"改功能后回退"流程无法完成。
    const showBatchActions = !viewingHistoryId && stat?.lastEntryId === entry.id
      && (pendingInTurn.length > 0 || appliedInTurn.length > 0);
    return (
      <React.Fragment key={entry.id}>
        <FileChangeCard
          path={change.path}
          status={status}
          diff={diff}
          onViewDiff={() =>
            useEditorStore.getState().openDiffContent(
              `${change.path}（改动前）`,
              change.old_content,
              `${change.path}（改动后）`,
              change.new_content,
              detectLanguage(change.path),
            )
          }
          onAccept={viewingHistoryId ? undefined : () => acceptChange(change.id)}
          onReject={viewingHistoryId ? undefined : () => rejectChange(change.id)}
          onUndo={viewingHistoryId ? undefined : () => undoChange(change.id)}
        />
        {showBatchActions && (
          <div style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>
            {pendingInTurn.length > 0 && (
              <>
                <button
                  className="btn ghost sm"
                  onClick={() => void runBatch(pendingInTurn, acceptChange, "应用")}
                >
                  全部应用（{pendingInTurn.length}）
                </button>
                <button
                  className="btn ghost sm"
                  onClick={() => void runBatch(pendingInTurn, rejectChange, "拒绝")}
                >
                  全部拒绝（{pendingInTurn.length}）
                </button>
              </>
            )}
            {appliedInTurn.length > 0 && (
              <button className="btn ghost sm" onClick={() => void revertTurn(change.turn_id)}>
                撤销本轮全部改动（{appliedInTurn.length}）
              </button>
            )}
          </div>
        )}
      </React.Fragment>
    );
  };

  const detailGroup = detailRound !== null ? roundGroups.find((g) => g.round === detailRound) ?? null : null;

  return (
    <div style={{ display: "flex", flexDirection: "column", flex: 1, minHeight: 0 }}>
      {/* 根节点不能用 height:"100%"——实测（headless Chrome 量出精确像素）发现它在
          flex 列（.ai-tools-dock: header 35px + 本节点）里不会按"剩余空间"收缩：
          height:100% 只是把 .ai-tools-dock 的整高当基准，不会减去旁边 header 的
          35px，于是整个面板往下多出 35px，正好把最下面 .agent-composer 的发送
          按钮挤出 .ai-tools-dock 的可视区（overflow:hidden 直接切掉，够不着）——
          这才是截图反馈"右下角出了屏幕"的真正原因，前两轮改 CSS 换行/收缩逻辑
          都只是治标。flex:1 + minHeight:0 才是 flex 列里"占满剩余空间、允许被
          兄弟元素挤压收缩"的正确写法。

          顶部工具栏按钮多、面板窄时会换行到 2~3 行；工具栏和消息列表包在下面同一个
          flex:1 + overflow-y:auto 的滚动区里，换行再多也只是这个区域自己滚动，
          不会挤占 .agent-composer 的空间，输入框固定在底部、永远完整可见。 */}
      <div
        ref={listRef}
        onScroll={handleTimelineScroll}
        // `overflow-anchor: none`（`.agent-timeline-scroll`，和 SqlAgentPanel 的
        // 同款滚动容器一致）——浏览器默认的"滚动锚定"会在视口上方内容发生布局
        // 变化时自作聪明地帮你调整 scrollTop，想保持"锚点元素看起来没动"，但这
        // 和这里自己维护的 `shouldFollowBottomRef`/`handleTimelineScroll` 手动
        // 跟随逻辑直接打架：2026-09 用户反馈"AI 没在跑的时候，把滚动条手动拖到
        // 最下面，它会自动弹上去"——正是浏览器自己的滚动锚定在跟用户的手动
        // 拖拽较劲，不是这里哪段 JS 逻辑写错了。这个类原本只加在 SqlAgentPanel
        // 那份拷贝上，这边漏掉了。
        className="agent-timeline-scroll"
        style={{ flex: 1, minHeight: 0, overflowY: "auto", display: "flex", flexDirection: "column", position: "relative" }}
      >
      <div className="editor-toolbar" style={{ gap: 12, flexWrap: "wrap", height: "auto", minHeight: 32 }}>
        <SegmentedControl
          value={sessionInfo.mode}
          onChange={(m) => { if (!viewingHistoryId) setMode(m); }}
          options={[
            { value: "plan", label: "Plan" },
            { value: "build", label: "Build" },
          ]}
        />
        {/* 2026-09 需求：Provider（连接配置）和模型拆成两级——这里只选 Provider，
            具体用哪个模型挪到下面 `.agent-composer-footer` 里选（跟输入框放
            一起，对齐 ChatGPT/Claude 桌面版 composer 的惯例），切了 Provider 之后
            那边会自动拉取这个 Provider 的模型列表（见下面 `fetchModels` 的 effect）。 */}
        <select
          className="form-select agent-model-select"
          value={sessionInfo.provider_id}
          onChange={(event) => setProvider(event.target.value)}
          disabled={sending || Boolean(viewingHistoryId)}
          title="切换后续消息使用的 Provider"
        >
          {providers.map((provider) => (
            <option key={provider.id} value={provider.id}>
              {provider.name}
            </option>
          ))}
        </select>
        <TargetBadge targetLabel={targetLabel(sessionInfo.target)} isRemote={isRemote} />
        {isRemote && <RemoteCapabilityBadge />}
        {sessionInfo.project_memory_loaded.length > 0 && (
          <span
            style={{ display: "inline-flex", alignItems: "center", gap: 4, fontSize: 11, color: "var(--text-secondary)" }}
            title={`系统提示词已注入：${sessionInfo.project_memory_loaded.join(", ")}`}
          >
            <BookOpen style={{ width: 12, height: 12 }} /> {sessionInfo.project_memory_loaded.join(" / ")}
          </span>
        )}
        <button className="btn ghost sm" onClick={() => { setShowHistory(true); loadHistories(workspaceId); }} title="历史会话">
          <History style={{ width: 13, height: 13 }} /> 历史{histories.length ? ` (${histories.length})` : ""}
        </button>
        <button className="btn ghost sm" onClick={() => newSession(sessionInfo.provider_id)} disabled={sending} title="新建会话">
          <Plus style={{ width: 13, height: 13 }} /> 新会话
        </button>
        <button className="btn ghost sm" onClick={() => setShowPermissionRules(true)} title="权限规则管理">
          <ShieldCheck style={{ width: 13, height: 13 }} /> 权限规则
        </button>
        <button className="btn ghost sm" onClick={() => setShowMcpServers(true)} title="MCP 服务器管理">
          <Plug style={{ width: 13, height: 13 }} /> MCP
        </button>
        <button className="btn ghost sm" onClick={() => setShowSkills(true)} title="项目 Skills 查看/导入">
          <Blocks style={{ width: 13, height: 13 }} /> Skills
        </button>
        <button className={`btn ghost sm ${hasDraft ? "active" : ""}`} onClick={() => setShowProviders(true)}>
          <Settings style={{ width: 13, height: 13 }} /> {hasDraft ? "继续配置" : "模型管理"}
        </button>
        {sessionInfo.mode === "build" && !viewingHistoryId && (
          <>
            <label style={{ display: "flex", alignItems: "center", gap: 6, fontSize: 12, color: "var(--text-secondary)" }}>
              <ToggleSwitch checked={sessionInfo.auto_allow_readonly} onChange={setAutoAllowReadonly} label="自动放行只读命令" />
              自动放行只读命令
            </label>
            <label
              style={{ display: "flex", alignItems: "center", gap: 6, fontSize: 12, color: "var(--text-secondary)" }}
              title={"默认开启：AI 的文件改动直接写入磁盘，界面上只需要点\"撤销\"；关闭后退回每条改动手动点\"应用\"。不影响命令确认。"}
            >
              <ToggleSwitch checked={sessionInfo.auto_apply_changes} onChange={setAutoApplyChanges} label="自动应用文件改动" />
              自动应用文件改动
            </label>
            <label
              style={{ display: "flex", alignItems: "center", gap: 6, fontSize: 12, color: "var(--text-secondary)" }}
              title={sessionInfo.git_repo ? "每次点击\"应用\"就自动 git add + commit 这个文件" : "工作区根目录不是 Git 仓库，无法使用"}
            >
              <ToggleSwitch
                checked={sessionInfo.auto_git_commit}
                onChange={setAutoGitCommit}
                disabled={!sessionInfo.git_repo}
                label="自动 Git 提交"
              />
              自动 Git 提交
            </label>
            <label
              style={{ display: "flex", alignItems: "center", gap: 6, fontSize: 12, color: "var(--text-secondary)" }}
              title="开启后 AI 的文件改动直接写入磁盘，命令与 MCP 工具不再逐项确认；已弹出的当前会话命令确认也会立即放行。高危命令和显式拒绝的权限规则仍会生效。"
            >
              <ToggleSwitch checked={sessionInfo.full_auto} onChange={setFullAuto} label="完全授权模式" />
              完全授权模式
            </label>
          </>
        )}
      </div>

      {showJumpBottom && (
        <button className="agent-jump-bottom" onClick={scrollTimelineToBottom} title="跳到最新消息">
          ↓ 最新消息
        </button>
      )}

      {sessionInfo.todos.length > 0 && (
        <div style={{ padding: "6px 12px", borderBottom: "1px solid var(--border-subtle)", display: "flex", flexDirection: "column", gap: 3 }}>
          {sessionInfo.todos.map((todo) => (
            <div
              key={todo.id}
              style={{
                display: "flex", alignItems: "center", gap: 6, fontSize: 12,
                color: todo.status === "completed" ? "var(--text-secondary)" : "var(--text-primary)",
                textDecoration: todo.status === "completed" ? "line-through" : "none",
              }}
            >
              {todoIcon(todo.status)}
              <span>{todo.content}</span>
            </div>
          ))}
        </div>
      )}

      {viewingHistoryId && <div className="coding-history-banner">
        <span>正在查看历史会话（只读）</span>
        <button className="btn primary sm" onClick={() => newSession(activeProvider?.id ?? selectedProviderId ?? sessionInfo.provider_id)}>返回新会话</button>
      </div>}

      <div className="agent-timeline-content" style={{ padding: "8px 12px", display: "flex", flexDirection: "column", gap: 10 }}>
        {timeline.length === 0 ? (
          <div style={{ textAlign: "center", color: "var(--text-secondary)", fontSize: 13, marginTop: 24 }}>
            {sessionInfo.mode === "plan" ? "直接提问或描述任务；Plan 模式不会修改文件" : "提问，或描述你想做的改动"}
          </div>
        ) : (
          roundGroups.map((group) => {
            // 只有最新这一轮内联完整展开（用户问题 + 过程 + AI 最终答复）；
            // 更早的轮次统一收成一行摘要（网格式布局：轮次号 + 用户问题预览 +
            // AI 答复预览 + 计数），点这一行用弹框看完整明细——不再有"展开了
            // 却没法收回去"的中间态需要维护（2026-09 用户反馈）。
            if (group.round === currentRound) {
              return (
                <div key={`round-${group.round}`} className="agent-round-current">
                  <div className="agent-round-current-label">
                    第 {group.round + 1} 轮{sending ? " · 进行中" : ""}
                  </div>
                  {group.items.map(({ entry }) => renderEntry(entry))}
                </div>
              );
            }
            const summary = summarizeRound(group.items);
            const batches = computeTurnBatches(group.items, changesById, turnStats);
            const hasBatchActions = !viewingHistoryId
              && batches.some((b) => b.pendingInTurn.length > 0 || b.appliedInTurn.length > 0);
            return (
              <div
                key={`round-${group.round}`}
                className="agent-round-row"
                role="button"
                tabIndex={0}
                onClick={() => setDetailRound(group.round)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" || e.key === " ") {
                    e.preventDefault();
                    setDetailRound(group.round);
                  }
                }}
                title="点击查看这一轮的完整过程"
              >
                <span className="agent-round-row-index">#{group.round + 1}</span>
                <div className="agent-round-row-texts">
                  <div className="agent-round-row-user"><User style={{ width: 11, height: 11 }} /> {summary.userPreview}</div>
                  <div className="agent-round-row-answer"><Bot style={{ width: 11, height: 11 }} /> {summary.answerPreview}</div>
                </div>
                <div className="agent-round-row-meta">
                  {summary.toolCount > 0 && <span className="agent-round-row-badge">{summary.toolCount} 个工具</span>}
                  {summary.changeCount > 0 && <span className="agent-round-row-badge">{summary.changeCount} 处改动</span>}
                  {summary.totalTokens !== null && (
                    <span className="agent-round-row-badge">{formatTokenCount(summary.totalTokens)} tokens</span>
                  )}
                </div>
                {hasBatchActions && (
                  <div className="agent-round-row-actions" onClick={(e) => e.stopPropagation()}>
                    {batches.map((b) => (
                      <React.Fragment key={b.turnId}>
                        {b.pendingInTurn.length > 0 && (
                          <>
                            <button className="btn ghost sm" onClick={() => void runBatch(b.pendingInTurn, acceptChange, "应用")}>
                              应用（{b.pendingInTurn.length}）
                            </button>
                            <button className="btn ghost sm" onClick={() => void runBatch(b.pendingInTurn, rejectChange, "拒绝")}>
                              拒绝（{b.pendingInTurn.length}）
                            </button>
                          </>
                        )}
                        {b.appliedInTurn.length > 0 && (
                          <button className="btn ghost sm" onClick={() => void revertTurn(b.turnId)}>
                            撤销（{b.appliedInTurn.length}）
                          </button>
                        )}
                      </React.Fragment>
                    ))}
                  </div>
                )}
                <ChevronRight style={{ width: 14, height: 14, color: "var(--text-secondary)", flexShrink: 0 }} />
              </div>
            );
          })
        )}
      </div>
      </div>

      {detailGroup && (
        <div className="agent-round-detail-overlay" onClick={() => setDetailRound(null)}>
          <div className="agent-round-detail-dialog" onClick={(e) => e.stopPropagation()} role="dialog" aria-modal="true">
            <div className="agent-round-detail-header">
              <span>第 {detailGroup.round + 1} 轮详情</span>
              <button className="agent-round-detail-close" onClick={() => setDetailRound(null)} title="关闭（Esc）">
                <X style={{ width: 14, height: 14 }} />
              </button>
            </div>
            <div className="agent-round-detail-body">
              {detailGroup.items.map(({ entry }) => renderEntry(entry))}
            </div>
          </div>
        </div>
      )}

      {liveStatusText && (
        <div className="agent-live-status" key={liveTick}>
          <span className="agent-live-dot" />
          {liveStatusText}
          {liveTokenText && <span className="agent-live-tokens">· {liveTokenText}</span>}
        </div>
      )}

      {error && <div style={{ padding: "4px 12px", fontSize: 12, color: "var(--danger)" }}>{error}</div>}

      <div
        ref={composerRef}
        className={`agent-composer ${draggingAttachments ? "agent-composer-dragging" : ""}`}
      >
        {attachments.length > 0 && (
          <div className="agent-pending-attachments">
            {attachments.map((att) => (
              <div key={att.id} className="agent-pending-attachment">
                {att.kind === "image" && att.previewUrl ? (
                  <img src={att.previewUrl} alt={att.name} className="agent-attachment-thumb" title={att.name} />
                ) : (
                  <span className="agent-attachment-chip" title={att.name}>
                    <Paperclip style={{ width: 11, height: 11 }} /> {att.name}
                  </span>
                )}
                <button className="agent-pending-attachment-remove" onClick={() => removeAttachment(att.id)} title="移除附件">
                  <X style={{ width: 10, height: 10 }} />
                </button>
              </div>
            ))}
          </div>
        )}
        <textarea
          className="agent-composer-input"
          ref={inputElementRef}
          rows={3}
          disabled={Boolean(viewingHistoryId)}
          placeholder={
            sending
              ? "AI 正在处理，这时候按 Enter 会直接插进当前对话，不用等它说完"
              : "提问或描述任务，Enter 发送，Shift+Enter 换行，可直接粘贴图片"
          }
          defaultValue=""
          onChange={(e) => {
            inputValueRef.current = e.target.value;
            if (inputSyncTimerRef.current) clearTimeout(inputSyncTimerRef.current);
            inputSyncTimerRef.current = setTimeout(() => {
              setInput(inputValueRef.current);
              inputSyncTimerRef.current = null;
            }, 120);
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              handleSend();
            }
          }}
          onPaste={handlePaste}
        />
        <div className="agent-composer-footer">
          <div className="agent-model-meta" title={activeProvider?.api_base}>
            <Sparkles />
            {/* 这里选的是"当前 Provider 用哪个模型"，不是切 Provider（那个在上面
                工具栏里）——选项来自 `fetchModels` 拉到的列表，拉取失败/还没拉到
                时退回只显示 Provider 当前配置的那一个，保证选择器不会是空的。
                切换直接持久化写回这个 Provider 的 `model` 字段（复用现成的
                `updateProvider`，`api_key: null` 表示不改已保存的密钥）。 */}
            <select
              className="agent-model-select-inline"
              value={activeProvider?.model ?? ""}
              onChange={(event) => {
                if (!activeProvider) return;
                void updateProvider(activeProvider.id, {
                  name: activeProvider.name,
                  api_base: activeProvider.api_base,
                  api_key: null,
                  model: event.target.value,
                  is_local: activeProvider.is_local,
                  wire_api: activeProvider.wire_api,
                  reasoning_effort: activeProvider.reasoning_effort,
                  context_window_tokens: activeProvider.context_window_tokens,
                });
              }}
              disabled={sending || Boolean(viewingHistoryId) || !activeProvider}
              title="切换这个 Provider 使用的模型"
            >
              {(modelsByProvider[sessionInfo.provider_id]?.length
                ? modelsByProvider[sessionInfo.provider_id]
                : activeProvider
                ? [activeProvider.model]
                : []
              ).map((m) => (
                <option key={m} value={m}>
                  {m}
                </option>
              ))}
            </select>
            {activeProvider && <span>{activeProvider.name} · {activeProvider.is_local ? "本地" : "云端"}</span>}
            <span>· {sessionInfo.mode === "plan" ? "Plan" : "Build"}</span>
          </div>
          <span className="agent-input-hint">{sending ? "Enter 插话 · Shift+Enter 换行" : "Enter 发送 · Shift+Enter 换行"}</span>
          <input
            ref={fileInputRef}
            type="file"
            multiple
            accept="image/*,.pdf,application/pdf,.txt,.md,.json,.ts,.tsx,.js,.jsx,.py,.rs,.go,.java,.c,.cpp,.h,.hpp,.css,.html,.yaml,.yml,.toml,.csv,.log,.sh"
            style={{ display: "none" }}
            onChange={(e) => handleFilesSelected(e.target.files)}
          />
          <button
            className="agent-composer-icon-btn"
            onClick={() => fileInputRef.current?.click()}
            disabled={Boolean(viewingHistoryId)}
            title="添加图片/文件附件"
          >
            <Paperclip />
          </button>
          <button
            className="agent-composer-icon-btn"
            onClick={handleOptimize}
            disabled={optimizing || sending || Boolean(viewingHistoryId) || !input.trim()}
            title="优化输入：用当前模型把草稿改写成更清晰的提示词"
          >
            <Wand2 className={optimizing ? "agent-icon-spin" : undefined} />
          </button>
          {sending ? (
            <button
              className="agent-send-btn agent-stop-btn"
              onClick={cancelTurn}
              title="停止：中断当前正在进行的对话轮次"
            >
              <Square fill="currentColor" />
            </button>
          ) : (
            <button
              className="agent-send-btn"
              onClick={handleSend}
              disabled={Boolean(viewingHistoryId) || (!input.trim() && attachments.length === 0)}
              title="发送"
            >
              <Send />
            </button>
          )}
        </div>
      </div>

      {confirmRequest && (
        <CommandConfirmDialog
          open
          host={confirmRequest.host ?? undefined}
          command={confirmRequest.command}
          kind={confirmRequest.kind}
          suggestedPattern={confirmRequest.kind === "mcp" ? (confirmRequest.matchKey ?? confirmRequest.command) : suggestCommandPattern(confirmRequest.command)}
          onReject={() => resolveConfirm(false)}
          onAllowOnce={() => resolveConfirm(true)}
          onAllowAndRemember={(pattern) => resolveConfirmAndRemember(pattern)}
        />
      )}
      {questionRequest && (
        <QuestionDialog
          open
          question={questionRequest.question}
          options={questionRequest.options}
          onAnswer={(answer) => answerQuestion(answer)}
        />
      )}
      {showProviders && <ProviderManagerDialog onClose={(hasDraft) => { setShowProviders(false); setProviderDraftPending(hasDraft); }} />}
      {showHistory && <CodingHistoryDialog title="编程会话历史" emptyText="还没有已保存的编程会话" histories={histories} onOpen={(id) => { openHistory(id); setShowHistory(false); }} onDelete={deleteHistory} onRename={renameHistory} onClose={() => setShowHistory(false)} />}
      {showPermissionRules && <PermissionRulesDialog onClose={() => setShowPermissionRules(false)} />}
      {showMcpServers && <McpServerManagerDialog onClose={() => setShowMcpServers(false)} />}
      {showSkills && <SkillManagerDialog workspaceId={workspaceId} onClose={() => setShowSkills(false)} />}
    </div>
  );
};
