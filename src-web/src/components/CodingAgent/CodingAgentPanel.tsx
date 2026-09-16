import React, { useEffect, useRef, useState } from "react";
import { Send, Bot, User, GitCommitHorizontal, Brain, ChevronRight, Sparkles, Settings, History, Plus, ShieldCheck, Plug, Blocks, BookOpen, CircleDot, CircleCheck, Circle, Paperclip, Wand2, X, Square } from "lucide-react";
import { useCodingStore } from "../../stores/codingStore";
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
import type { CodingTarget, FileChange, TodoStatus } from "../../types/bindings";

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
const ThinkingBlock: React.FC<{ text: string; active: boolean; onOpenFile?: (path: string, line?: number) => void }> = ({ text, active, onOpenFile }) => {
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
    error,
    confirmRequest,
    questionRequest,
    start,
    setMode,
    setProvider,
    setAutoAllowReadonly,
    setAutoGitCommit,
    setFullAuto,
    sendMessage,
    cancelTurn,
    attachments,
    addAttachments,
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
  const [input, setInput] = useState("");
  const [selectedProviderId, setSelectedProviderId] = useState<string>("");
  const [showProviders, setShowProviders] = useState(false);
  const [providerDraftPending, setProviderDraftPending] = useState(() => hasProviderDraft());
  const [showHistory, setShowHistory] = useState(false);
  const [showPermissionRules, setShowPermissionRules] = useState(false);
  const [showMcpServers, setShowMcpServers] = useState(false);
  const [showSkills, setShowSkills] = useState(false);
  const listRef = useRef<HTMLDivElement>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);

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

  useEffect(() => {
    if (providers.length > 0 && !providers.some((provider) => provider.id === selectedProviderId)) {
      setSelectedProviderId(providers[0].id);
    }
  }, [providers, selectedProviderId]);

  useEffect(() => {
    listRef.current?.scrollTo({ top: listRef.current.scrollHeight });
  }, [timeline]);

  // 按 `turn_id`（同一条用户消息触发的一轮工具调用）给 FileChangeCard 分组——
  // 记下每一轮最后一张卡片在时间线里的 id，只在那张卡片后面挂一次"全部应用/
  // 全部拒绝/撤销本轮"的批量操作条，而不是每张卡片都重复一份（参考 Cursor/
  // Windsurf 按对话轮次做批量操作，但这里不依赖 git，见项目内部设计讨论）。
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

  const handleStart = () => {
    if (!selectedProviderId) return;
    start(workspaceId, selectedProviderId);
  };

  const handleSend = () => {
    if (!input.trim() && attachments.length === 0) return;
    sendMessage(input);
    setInput("");
  };

  const handleOptimize = async () => {
    if (!input.trim() || optimizing) return;
    const optimized = await optimizePrompt(input);
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
      <div style={{ flex: 1, minHeight: 0, overflowY: "auto", display: "flex", flexDirection: "column" }}>
      <div className="editor-toolbar" style={{ gap: 12, flexWrap: "wrap", height: "auto", minHeight: 32 }}>
        <SegmentedControl
          value={sessionInfo.mode}
          onChange={(m) => { if (!viewingHistoryId) setMode(m); }}
          options={[
            { value: "plan", label: "Plan" },
            { value: "build", label: "Build" },
          ]}
        />
        <select
          className="form-select agent-model-select"
          value={sessionInfo.provider_id}
          onChange={(event) => setProvider(event.target.value)}
          disabled={sending || Boolean(viewingHistoryId)}
          title="切换后续消息使用的 Provider / 模型"
        >
          {providers.map((provider) => (
            <option key={provider.id} value={provider.id}>
              {provider.name} · {provider.model}
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

      <div ref={listRef} style={{ padding: "8px 12px", display: "flex", flexDirection: "column", gap: 10 }}>
        {timeline.length === 0 ? (
          <div style={{ textAlign: "center", color: "var(--text-secondary)", fontSize: 13, marginTop: 24 }}>
            {sessionInfo.mode === "plan" ? "直接提问或描述任务；Plan 模式不会修改文件" : "提问，或描述你想做的改动"}
          </div>
        ) : (
          timeline.map((entry) => {
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
              const hasFileTarget = ["read_file", "write_file", "edit_file", "list_directory"].includes(entry.tool);
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
                  本轮对话共消耗 tokens：输入 {entry.promptTokens} · 输出 {entry.completionTokens} · 合计 {entry.totalTokens}
                </div>
              ) : (
                <div key={entry.id} style={{ fontSize: 11, color: "var(--text-secondary)", opacity: 0.65 }}>
                  本次请求消耗 tokens：输入 {entry.promptTokens} · 输出 {entry.completionTokens} · 合计 {entry.totalTokens}
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
            // 单文件改动同样是一轮完整的 AI 操作，必须提供“撤销本轮”入口；之前
            // 误加了 `turnChanges.length > 1`，导致最常见的单文件修改只能看到卡片
            // 级操作，无法按轮回退，也让端到端“改功能后回退”流程无法完成。
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
                          onClick={() => { void (async () => { for (const c of pendingInTurn) await acceptChange(c.id); })(); }}
                        >
                          全部应用（{pendingInTurn.length}）
                        </button>
                        <button
                          className="btn ghost sm"
                          onClick={() => { void (async () => { for (const c of pendingInTurn) await rejectChange(c.id); })(); }}
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
          })
        )}
      </div>
      </div>

      {liveStatusText && (
        <div className="agent-live-status" key={liveTick}>
          <span className="agent-live-dot" />
          {liveStatusText}
        </div>
      )}

      {error && <div style={{ padding: "4px 12px", fontSize: 12, color: "var(--danger)" }}>{error}</div>}

      <div className="agent-composer">
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
          rows={3}
          disabled={Boolean(viewingHistoryId)}
          placeholder="提问或描述任务，Enter 发送，Shift+Enter 换行，可直接粘贴图片"
          value={input}
          onChange={(e) => setInput(e.target.value)}
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
            <strong>{activeProvider?.model ?? "未选择模型"}</strong>
            {activeProvider && <span>{activeProvider.name} · {activeProvider.is_local ? "本地" : "云端"}</span>}
            <span>· {sessionInfo.mode === "plan" ? "Plan" : "Build"}</span>
          </div>
          <span className="agent-input-hint">Enter 发送 · Shift+Enter 换行</span>
          <input
            ref={fileInputRef}
            type="file"
            multiple
            accept="image/*,.txt,.md,.json,.ts,.tsx,.js,.jsx,.py,.rs,.go,.java,.c,.cpp,.h,.hpp,.css,.html,.yaml,.yml,.toml,.csv,.log,.sh"
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
      {showHistory && <CodingHistoryDialog histories={histories} onOpen={(id) => { openHistory(id); setShowHistory(false); }} onDelete={deleteHistory} onRename={renameHistory} onClose={() => setShowHistory(false)} />}
      {showPermissionRules && <PermissionRulesDialog onClose={() => setShowPermissionRules(false)} />}
      {showMcpServers && <McpServerManagerDialog onClose={() => setShowMcpServers(false)} />}
      {showSkills && <SkillManagerDialog workspaceId={workspaceId} onClose={() => setShowSkills(false)} />}
    </div>
  );
};
