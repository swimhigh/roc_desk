import React, { useEffect, useRef, useState } from "react";
import { Send, Bot, User, GitCommitHorizontal, Brain, ChevronRight, Sparkles, Settings, History, Plus, ShieldCheck, Plug, BookOpen, CircleDot, CircleCheck, Circle, Paperclip, Wand2, X } from "lucide-react";
import { useCodingStore } from "../../stores/codingStore";
import { useAiChatStore } from "../../stores/aiChatStore";
import { SegmentedControl } from "../shared/SegmentedControl";
import { ToggleSwitch } from "../shared/ToggleSwitch";
import { TargetBadge } from "./TargetBadge";
import { RemoteCapabilityBadge } from "./RemoteCapabilityBadge";
import { ToolCallProgress } from "./ToolCallProgress";
import { FileChangeCard, type DiffLine as CardDiffLine } from "./FileChangeCard";
import { CommandConfirmDialog, BlockedCommandMessage } from "./CommandConfirmDialog";
import { QuestionDialog } from "./QuestionDialog";
import { PermissionRulesDialog } from "./PermissionRulesDialog";
import { McpServerManagerDialog } from "./McpServerManagerDialog";
import { AgentMarkdown } from "./AgentMarkdown";
import { ProviderManagerDialog, hasProviderDraft } from "../AiChat/ProviderManagerDialog";
import { CodingHistoryDialog } from "./CodingHistoryDialog";
import type { CodingTarget, TodoStatus } from "../../types/bindings";

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
    sendMessage,
    attachments,
    addAttachments,
    removeAttachment,
    optimizing,
    optimizePrompt,
    acceptChange,
    rejectChange,
    undoChange,
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
  const listRef = useRef<HTMLDivElement>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);

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
          </>
        )}
      </div>
    );
  }

  const isRemote = sessionInfo.target.kind === "Remote";
  const activeProvider = providers.find((provider) => provider.id === sessionInfo.provider_id);
  const activeThinkingId = [...timeline].reverse().find((entry) => entry.kind === "note")?.id;
  const hasDraft = providerDraftPending || hasProviderDraft();

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
                  elapsedMs={0}
                  done={!entry.running}
                  detail={entry.detail}
                  onOpenFile={hasFileTarget && entry.detail && onOpenFile ? () => onOpenFile(entry.detail!) : undefined}
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
            const change = changesById[entry.changeId];
            if (!change) return null;
            const diff: CardDiffLine[] = change.diff.map((l) => ({ sign: l.sign, content: l.content }));
            const status = change.status === "undone" ? "rejected" : change.status;
            return (
              <FileChangeCard
                key={entry.id}
                path={change.path}
                status={status}
                diff={diff}
                onViewDiff={() => onOpenFile?.(change.path)}
                onAccept={viewingHistoryId ? undefined : () => acceptChange(change.id)}
                onReject={viewingHistoryId ? undefined : () => rejectChange(change.id)}
                onUndo={viewingHistoryId ? undefined : () => undoChange(change.id)}
              />
            );
          })
        )}
      </div>
      </div>

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
          placeholder="提问或描述任务，Enter 发送，Shift+Enter 换行"
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              handleSend();
            }
          }}
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
          <button
            className="agent-send-btn"
            onClick={handleSend}
            disabled={sending || Boolean(viewingHistoryId) || (!input.trim() && attachments.length === 0)}
            title="发送"
          >
            <Send />
          </button>
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
    </div>
  );
};
