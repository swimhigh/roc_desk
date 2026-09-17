import React, { useEffect, useRef, useState } from "react";
import { Bot, History, Plus, Send, Settings, Sparkles, Square, User } from "lucide-react";
import { useSqlAgentStore } from "../../stores/sqlAgentStore";
import { useAiChatStore } from "../../stores/aiChatStore";
import { AgentMarkdown } from "../CodingAgent/AgentMarkdown";
import { ThinkingBlock } from "../CodingAgent/CodingAgentPanel";
import { ToolCallProgress, toolLabel } from "../CodingAgent/ToolCallProgress";
import { CodingHistoryDialog } from "../CodingAgent/CodingHistoryDialog";
import { QuestionDialog } from "../CodingAgent/QuestionDialog";
import { SqlAgentConfirmDialog } from "./SqlAgentConfirmDialog";
import { ProviderManagerDialog, hasProviderDraft } from "../AiChat/ProviderManagerDialog";

interface SqlAgentPanelProps {
  dataSourceId: string;
}

/** 右栏 AI 工具——和"工作区"编程助手（`CodingAgentPanel`）同一种多轮 Agent
 * 架构、同一批 UI 组件（时间线消息气泡/工具调用进度条/思考块/历史弹窗/
 * Markdown 渲染全部直接复用那边的组件，不是照着抄一份同名的），后端也共用
 * `agent_llm` 里协议无关的那部分（见 `sql::agent::session` 文档）。裁剪掉的
 * 部分是 SQL 场景里没有对应物的机制：Plan/Build 模式切换、文件 Diff/Git 自动
 * 提交、MCP/Skills、附件——这些不是"以后再做"的遗漏，是范围裁剪。 */
export const SqlAgentPanel: React.FC<SqlAgentPanelProps> = ({ dataSourceId }) => {
  const {
    sessionInfo,
    timeline,
    sending,
    error,
    confirmRequest,
    questionRequest,
    setProvider,
    sendMessage,
    cancelTurn,
    toggleToolOutput,
    resolveConfirm,
    answerQuestion,
    histories,
    loadHistories,
    openHistory,
    deleteHistory,
    renameHistory,
    newSession,
    restoreOrStart,
  } = useSqlAgentStore();
  const providers = useAiChatStore((s) => s.providers);
  const loadProviders = useAiChatStore((s) => s.loadProviders);
  const [input, setInput] = useState("");
  const [selectedProviderId, setSelectedProviderId] = useState("");
  const [showProviders, setShowProviders] = useState(false);
  const [providerDraftPending, setProviderDraftPending] = useState(() => hasProviderDraft());
  const [showHistory, setShowHistory] = useState(false);
  const listRef = useRef<HTMLDivElement>(null);

  const [liveTick, setLiveTick] = useState(0);
  useEffect(() => {
    if (!sending) return;
    const timer = setInterval(() => setLiveTick((t) => t + 1), 500);
    return () => clearInterval(timer);
  }, [sending]);

  useEffect(() => {
    loadProviders();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (providers.length > 0 && !providers.some((p) => p.id === selectedProviderId)) {
      setSelectedProviderId(providers[0].id);
    }
  }, [providers, selectedProviderId]);

  const startedRef = useRef<string | null>(null);
  useEffect(() => {
    if (providers.length === 0) return;
    if (startedRef.current === dataSourceId) return;
    startedRef.current = dataSourceId;
    void restoreOrStart(dataSourceId, selectedProviderId || providers[0].id);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [dataSourceId, providers]);

  useEffect(() => {
    listRef.current?.scrollTo({ top: listRef.current.scrollHeight });
  }, [timeline]);

  const handleStart = () => {
    if (!selectedProviderId) return;
    startedRef.current = dataSourceId;
    void restoreOrStart(dataSourceId, selectedProviderId);
  };

  const handleSend = () => {
    if (!input.trim()) return;
    void sendMessage(input);
    setInput("");
  };

  if (!sessionInfo) {
    return (
      <div style={{ display: "flex", flexDirection: "column", alignItems: "center", justifyContent: "center", height: "100%", gap: 12, padding: 16 }}>
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
                <option key={p.id} value={p.id}>{p.name}</option>
              ))}
            </select>
            <button className="btn primary sm" onClick={handleStart}>开始 AI 会话</button>
            {error && <div style={{ padding: "0 12px", fontSize: 12, color: "var(--danger)", textAlign: "center", maxWidth: 280 }}>{error}</div>}
          </>
        )}
      </div>
    );
  }

  const activeProvider = providers.find((p) => p.id === sessionInfo.provider_id);
  const activeThinkingId = [...timeline].reverse().find((entry) => entry.kind === "note")?.id;
  const hasDraft = providerDraftPending || hasProviderDraft();
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
      <div style={{ flex: 1, minHeight: 0, overflowY: "auto", display: "flex", flexDirection: "column" }}>
        <div className="editor-toolbar" style={{ gap: 8, flexWrap: "wrap", height: "auto", minHeight: 32 }}>
          <Sparkles size={13} style={{ color: "var(--accent)" }} />
          <select
            className="form-select agent-model-select"
            style={{ minWidth: 0, flex: 1 }}
            value={sessionInfo.provider_id}
            onChange={(e) => void setProvider(e.target.value)}
            disabled={sending}
            title="切换后续消息使用的 Provider / 模型"
          >
            {providers.map((p) => (
              <option key={p.id} value={p.id}>{p.name} · {p.model}</option>
            ))}
          </select>
          <button className="btn ghost sm" onClick={() => { setShowHistory(true); void loadHistories(dataSourceId); }} title="历史会话">
            <History style={{ width: 13, height: 13 }} /> 历史{histories.length ? ` (${histories.length})` : ""}
          </button>
          <button className="btn ghost sm" onClick={() => void newSession(sessionInfo.provider_id)} disabled={sending} title="新建会话">
            <Plus style={{ width: 13, height: 13 }} /> 新会话
          </button>
          <button className={`btn ghost sm ${hasDraft ? "active" : ""}`} onClick={() => setShowProviders(true)}>
            <Settings style={{ width: 13, height: 13 }} /> {hasDraft ? "继续配置" : "模型管理"}
          </button>
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
                <span>{todo.status === "completed" ? "☑" : todo.status === "in_progress" ? "◐" : "☐"}</span>
                <span>{todo.content}</span>
              </div>
            ))}
          </div>
        )}

        <div ref={listRef} style={{ padding: "8px 12px", display: "flex", flexDirection: "column", gap: 10 }}>
          {timeline.length === 0 ? (
            <div style={{ textAlign: "center", color: "var(--text-secondary)", fontSize: 13, marginTop: 24 }}>
              直接提问，或让我先看看表结构再帮你写查询
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
                      {entry.kind === "assistant" ? <AgentMarkdown content={entry.text} /> : <div className="agent-user-text">{entry.text}</div>}
                    </div>
                  </div>
                );
              }
              if (entry.kind === "tool") {
                return (
                  <ToolCallProgress
                    key={entry.id}
                    tool={entry.tool}
                    elapsedMs={entry.running && entry.startedAt ? Date.now() - entry.startedAt : 0}
                    done={!entry.running}
                    detail={entry.detail}
                    output={entry.output}
                    expanded={entry.expanded}
                    onToggleOutput={() => toggleToolOutput(entry.id)}
                  />
                );
              }
              if (entry.kind === "note") {
                return <ThinkingBlock key={entry.id} text={entry.text} active={sending && entry.id === activeThinkingId} />;
              }
              if (entry.kind === "progress") {
                return <div key={entry.id} className="agent-progress-note"><Sparkles /><span>{entry.text}</span></div>;
              }
              if (entry.kind === "usage") {
                return entry.isTurnTotal ? (
                  <div key={entry.id} style={{ fontSize: 12, color: "var(--text-secondary)", fontWeight: 600, padding: "2px 0" }}>
                    本轮对话共消耗 tokens：输入 {entry.promptTokens} · 输出 {entry.completionTokens} · 合计 {entry.totalTokens}
                  </div>
                ) : null;
              }
              return null;
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
        <textarea
          className="agent-composer-input"
          rows={3}
          placeholder="提问，或描述想要生成/优化的 SQL，Enter 发送，Shift+Enter 换行"
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
          </div>
          <span className="agent-input-hint">Enter 发送 · Shift+Enter 换行</span>
          {sending ? (
            <button className="agent-send-btn agent-stop-btn" onClick={() => void cancelTurn()} title="停止">
              <Square fill="currentColor" />
            </button>
          ) : (
            <button className="agent-send-btn" onClick={handleSend} disabled={!input.trim()} title="发送">
              <Send />
            </button>
          )}
        </div>
      </div>

      {confirmRequest && (
        <SqlAgentConfirmDialog
          open
          sql={confirmRequest.sql}
          onReject={() => void resolveConfirm(false)}
          onAllow={() => void resolveConfirm(true)}
        />
      )}
      {questionRequest && (
        <QuestionDialog open question={questionRequest.question} options={questionRequest.options} onAnswer={(answer) => void answerQuestion(answer)} />
      )}
      {showProviders && <ProviderManagerDialog onClose={(draft) => { setShowProviders(false); setProviderDraftPending(draft); }} />}
      {showHistory && (
        <CodingHistoryDialog
          title="SQL 会话历史"
          emptyText="还没有已保存的 SQL 会话"
          histories={histories}
          onOpen={(id) => { void openHistory(id); setShowHistory(false); }}
          onDelete={(id) => void deleteHistory(id)}
          onRename={(id, title) => void renameHistory(id, title)}
          onClose={() => setShowHistory(false)}
        />
      )}
    </div>
  );
};
