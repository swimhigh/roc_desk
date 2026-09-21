import React, { useEffect, useLayoutEffect, useRef, useState } from "react";
import { Bot, History, Paperclip, Plus, Send, Settings, Sparkles, Square, User } from "lucide-react";
import { useSqlAgentStore } from "../../stores/sqlAgentStore";
import { useAiChatStore } from "../../stores/aiChatStore";
import { AgentMarkdown } from "../CodingAgent/AgentMarkdown";
import { ThinkingBlock } from "../CodingAgent/CodingAgentPanel";
import { ToolCallProgress, toolLabel } from "../CodingAgent/ToolCallProgress";
import { CodingHistoryDialog } from "../CodingAgent/CodingHistoryDialog";
import { QuestionDialog } from "../CodingAgent/QuestionDialog";
import { SqlAgentConfirmDialog } from "./SqlAgentConfirmDialog";
import { ProviderManagerDialog, hasProviderDraft } from "../AiChat/ProviderManagerDialog";
import { useExternalFileDrop } from "../../hooks/useExternalFileDrop";
import { localFileService } from "../../services/fsService";
import { formatError } from "../../utils/error";
import { formatTokenCount } from "../../utils/formatTokens";
import type { ChatAttachment } from "../../types/bindings";

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
  const [attachments, setAttachments] = useState<Array<ChatAttachment & { id: string; previewUrl?: string }>>([]);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const [selectedProviderId, setSelectedProviderId] = useState("");
  const [showProviders, setShowProviders] = useState(false);
  const [providerDraftPending, setProviderDraftPending] = useState(() => hasProviderDraft());
  const [showHistory, setShowHistory] = useState(false);
  const composerRef = useRef<HTMLDivElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const shouldFollowBottomRef = useRef(true);
  const stableScrollTopRef = useRef(0);
  const suppressScrollEventRef = useRef(false);

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

  useLayoutEffect(() => {
    const list = listRef.current;
    if (!list || !shouldFollowBottomRef.current) return;
    suppressScrollEventRef.current = true;
    list.scrollTop = Math.max(0, list.scrollHeight - list.clientHeight);
    stableScrollTopRef.current = list.scrollTop;
    requestAnimationFrame(() => { suppressScrollEventRef.current = false; });
  }, [timeline]);

  useEffect(() => {
    const list = listRef.current;
    if (!list) return;
    let frame = 0;
    const observer = new ResizeObserver(() => {
      if (!shouldFollowBottomRef.current) return;
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        list.scrollTop = Math.max(0, list.scrollHeight - list.clientHeight);
      });
    });
    observer.observe(list);
    return () => { cancelAnimationFrame(frame); observer.disconnect(); };
  }, []);

  const handleTimelineScroll = () => {
    const list = listRef.current;
    if (!list || suppressScrollEventRef.current) return;
    const distance = list.scrollHeight - list.scrollTop - list.clientHeight;
    shouldFollowBottomRef.current = distance <= 48;
    stableScrollTopRef.current = list.scrollTop;
  };

  const handleStart = () => {
    if (!selectedProviderId) return;
    startedRef.current = dataSourceId;
    void restoreOrStart(dataSourceId, selectedProviderId);
  };

  const handleSend = () => {
    if (!input.trim() && attachments.length === 0) return;
    const outgoing = attachments.map(({ id: _id, previewUrl: _preview, ...item }) => item);
    void sendMessage(input, outgoing);
    setInput("");
    setAttachments([]);
  };

  const handleFiles = (files: FileList | null) => {
    if (!files) return;
    for (const file of Array.from(files).slice(0, 5 - attachments.length)) {
      const isImage = file.type.startsWith("image/");
      const isPdf = file.type === "application/pdf" || file.name.toLowerCase().endsWith(".pdf");
      const id = crypto.randomUUID();
      if (isImage) {
        const maxBytes = 8 * 1024 * 1024;
        if (file.size > maxBytes) {
          window.alert(`${file.name} 太大（上限 8 MB），请先缩小或拆分后再添加。`);
          continue;
        }
        const imageReader = new FileReader();
        imageReader.onload = () => {
          const dataUrl = String(imageReader.result ?? "");
          const comma = dataUrl.indexOf(",");
          if (comma >= 0) setAttachments((items) => [...items, { id, kind: "image", name: file.name, mime: file.type || "image/png", data_base64: dataUrl.slice(comma + 1), previewUrl: dataUrl }]);
        };
        imageReader.readAsDataURL(file);
      } else if (isPdf) {
        // PDF 是二进制格式，不能像下面的普通文本文件那样 `readAsText`（会读出
        // 乱码，模型完全看不懂），要读成 base64 交给后端用 `pdf_extract` 抽取
        // 文本。这里也故意不设大小上限——2026-09 用户反馈"本地的 PDF 文件不该
        // 在加进对话框这一步就被拦下来"，常见的技术文档随便就超过之前误设的
        // 2MB；真要防"离谱地拖进一个几百 MB 文件"，交给上传后的抽取/发送环节
        // 处理更合适，不该在这里用一个拍脑袋的阈值挡掉正常大小的文档。
        const pdfReader = new FileReader();
        pdfReader.onload = () => {
          const dataUrl = String(pdfReader.result ?? "");
          const comma = dataUrl.indexOf(",");
          if (comma >= 0) setAttachments((items) => [...items, { id, kind: "pdf", name: file.name, data_base64: dataUrl.slice(comma + 1) }]);
        };
        pdfReader.readAsDataURL(file);
      } else {
        const maxBytes = 2 * 1024 * 1024;
        if (file.size > maxBytes) {
          window.alert(`${file.name} 太大（上限 2 MB），请先缩小或拆分后再添加。`);
          continue;
        }
        const reader = new FileReader();
        reader.onload = () => setAttachments((items) => [...items, { id, kind: "file", name: file.name, content: String(reader.result ?? "") }]);
        reader.readAsText(file);
      }
    }
  };

  const handleAttachmentPaste = (e: React.ClipboardEvent<HTMLTextAreaElement>) => {
    const files = Array.from(e.clipboardData.items)
      .filter((item) => item.kind === "file")
      .map((item) => item.getAsFile())
      .filter((file): file is File => Boolean(file));
    if (files.length > 0) {
      e.preventDefault();
      const transfer = new DataTransfer();
      files.forEach((file) => transfer.items.add(file));
      handleFiles(transfer.files);
    }
  };

  /** Tauri 原生拖拽（`useExternalFileDrop`）拿到的是磁盘绝对路径，不是浏览器
   * `File` 对象——不能走 `FileReader`，改成调 `localFileService`；图片/PDF 走
   * `readBinaryPreview`（base64，后端本身有 30MB 上限，出错会给清晰提示），
   * 纯文本文件走 `readFile`。和 `handleFiles` 是两条并行的路径，不是同一个
   * 函数分支复用——一个处理浏览器 File 对象，一个处理磁盘路径，读取方式完全
   * 不同。 */
  const handleFilesFromPaths = async (paths: string[]) => {
    for (const path of paths.slice(0, 5 - attachments.length)) {
      const name = path.replace(/\\/g, "/").split("/").pop() ?? path;
      const lower = path.toLowerCase();
      const isImage = /\.(png|jpe?g|gif|webp|bmp|svg)$/.test(lower);
      const isPdf = lower.endsWith(".pdf");
      const id = crypto.randomUUID();
      try {
        if (isImage) {
          const base64 = await localFileService.readBinaryPreview(path);
          const mime = lower.endsWith(".jpg") || lower.endsWith(".jpeg") ? "image/jpeg" : lower.endsWith(".gif") ? "image/gif" : lower.endsWith(".webp") ? "image/webp" : lower.endsWith(".bmp") ? "image/bmp" : lower.endsWith(".svg") ? "image/svg+xml" : "image/png";
          setAttachments((items) => [...items, { id, kind: "image", name, mime, data_base64: base64, previewUrl: `data:${mime};base64,${base64}` }]);
        } else if (isPdf) {
          const base64 = await localFileService.readBinaryPreview(path);
          setAttachments((items) => [...items, { id, kind: "pdf", name, data_base64: base64 }]);
        } else {
          const file = await localFileService.readFile(path);
          setAttachments((items) => [...items, { id, kind: "file", name, content: file.text }]);
        }
      } catch (e) {
        window.alert(`读取 ${name} 失败：${formatError(e)}`);
      }
    }
  };

  const { isDragOver: draggingAttachments } = useExternalFileDrop(composerRef, (paths) => {
    void handleFilesFromPaths(paths);
  });

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
      <div ref={listRef} onScroll={handleTimelineScroll} className="agent-timeline-scroll" style={{ flex: 1, minHeight: 0, overflowY: "auto", display: "flex", flexDirection: "column" }}>
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

        <div className="agent-timeline-content" style={{ padding: "8px 12px", display: "flex", flexDirection: "column", gap: 10 }}>
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
                    本轮对话共消耗 tokens：输入 {formatTokenCount(entry.promptTokens)} · 输出 {formatTokenCount(entry.completionTokens)} · 合计{" "}
                    {formatTokenCount(entry.totalTokens)}
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

      <div
        ref={composerRef}
        className={`agent-composer ${draggingAttachments ? "agent-composer-dragging" : ""}`}
      >
        {attachments.length > 0 && <div className="agent-attachment-list">{attachments.map((file) => <span className="agent-attachment-chip" key={file.id}>{file.kind === "image" ? "图片" : file.kind === "pdf" ? "PDF" : "文件"}：{file.name}<button className="icon-btn" onClick={() => setAttachments((items) => items.filter((item) => item.id !== file.id))}>×</button></span>)}</div>}
        <textarea
          className="agent-composer-input"
          rows={3}
          placeholder="提问，或描述想要生成/优化的 SQL，Enter 发送，Shift+Enter 换行"
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onPaste={handleAttachmentPaste}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              handleSend();
            }
          }}
        />
        <div className="agent-composer-footer">
          <input ref={fileInputRef} type="file" multiple style={{ display: "none" }} onChange={(e) => { handleFiles(e.target.files); e.target.value = ""; }} />
          <button className="agent-composer-icon-btn" onClick={() => fileInputRef.current?.click()} title="添加文件或图片"><Paperclip /></button>
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
            <button className="agent-send-btn" onClick={handleSend} disabled={!input.trim() && attachments.length === 0} title="发送">
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
