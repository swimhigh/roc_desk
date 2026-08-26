import React, { useEffect, useRef, useState } from "react";
import { Send, Settings, Trash2, Bot, User, ShieldAlert } from "lucide-react";
import { useAiChatStore } from "../../stores/aiChatStore";
import { ToggleSwitch } from "../shared/ToggleSwitch";
import { ProviderManagerDialog, hasProviderDraft } from "./ProviderManagerDialog";
import { AgentMarkdown } from "../CodingAgent/AgentMarkdown";

/**
 * AI 问答面板（DESIGN.md §3.6）：与工作区本地/远程无关，始终可用——它只消费
 * 已经读进前端内存的文本，不直接读写文件系统（那是 AI 编程助手的事）。
 */
export const ChatPanel: React.FC = () => {
  const {
    providers,
    activeProviderId,
    messages,
    redactEnabled,
    webSearchEnabled,
    sending,
    error,
    loadProviders,
    setActiveProvider,
    setRedactEnabled,
    setWebSearchEnabled,
    sendMessage,
    clearChat,
  } = useAiChatStore();
  const [input, setInput] = useState("");
  const [showProviders, setShowProviders] = useState(false);
  const [providerDraftPending, setProviderDraftPending] = useState(() => hasProviderDraft());
  const listRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    loadProviders();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    listRef.current?.scrollTo({ top: listRef.current.scrollHeight });
  }, [messages]);

  const activeProvider = providers.find((p) => p.id === activeProviderId);
  const hasDraft = providerDraftPending || hasProviderDraft();

  const handleSend = () => {
    if (!input.trim()) return;
    sendMessage(input);
    setInput("");
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      <div className="editor-toolbar" style={{ gap: 8 }}>
        {providers.length === 0 ? (
          <span style={{ fontSize: 12, color: "var(--text-secondary)" }}>还没有配置 AI Provider</span>
        ) : (
          <select
            className="form-select"
            style={{ height: 26, fontSize: 12 }}
            value={activeProviderId ?? ""}
            onChange={(e) => setActiveProvider(e.target.value)}
          >
            {providers.map((p) => (
              <option key={p.id} value={p.id}>
                {p.name} {p.is_local ? "(本地)" : "(云端)"}
              </option>
            ))}
          </select>
        )}
        {activeProvider && !activeProvider.is_local && (
          <label
            style={{ display: "flex", alignItems: "center", gap: 6, fontSize: 12, color: "var(--text-secondary)" }}
            title="云端 Provider：常见密钥/密码模式会被遮蔽后再发送"
          >
            <ToggleSwitch checked={redactEnabled} onChange={setRedactEnabled} label="发送前脱敏" />
            发送前脱敏
            <ShieldAlert style={{ width: 12, height: 12 }} />
          </label>
        )}
        {activeProvider && (
          <label
            style={{ display: "flex", alignItems: "center", gap: 6, fontSize: 12, color: "var(--text-secondary)" }}
            title="默认先搜索互联网，再结合搜索结果回答"
          >
            <ToggleSwitch checked={webSearchEnabled} onChange={setWebSearchEnabled} label="联网搜索" />
            联网搜索
          </label>
        )}
        <div style={{ marginLeft: "auto", display: "flex", gap: 4 }}>
          <button className="btn ghost sm" onClick={clearChat} title="清空对话">
            <Trash2 style={{ width: 14, height: 14 }} />
          </button>
          <button className={`btn ghost sm ${hasDraft ? "active" : ""}`} onClick={() => setShowProviders(true)} title="管理 Provider">
            <Settings style={{ width: 14, height: 14 }} />
            {hasDraft ? "继续配置" : "模型管理"}
          </button>
        </div>
      </div>

      <div ref={listRef} style={{ flex: 1, overflowY: "auto", padding: "8px 12px", display: "flex", flexDirection: "column", gap: 12 }}>
        {messages.length === 0 ? (
          <div style={{ textAlign: "center", color: "var(--text-secondary)", fontSize: 13, marginTop: 24 }}>
            {providers.length === 0 ? "先点右上角 ⚙ 添加一个 Provider" : "输入问题开始对话"}
          </div>
        ) : (
          messages.map((m) => (
            <div key={m.id} style={{ display: "flex", gap: 8, alignItems: "flex-start" }}>
              <div
                style={{
                  width: 22, height: 22, borderRadius: "50%", flexShrink: 0,
                  display: "flex", alignItems: "center", justifyContent: "center",
                  background: m.role === "user" ? "var(--bg-hover)" : "var(--accent-dim)",
                  color: m.role === "user" ? "var(--text-secondary)" : "var(--accent)",
                }}
              >
                {m.role === "user" ? <User style={{ width: 13, height: 13 }} /> : <Bot style={{ width: 13, height: 13 }} />}
              </div>
              <div style={{ flex: 1, fontSize: 13, whiteSpace: "pre-wrap", wordBreak: "break-word", lineHeight: 1.6 }}>
                <AgentMarkdown content={m.content} />
                {m.streaming && <span className="chat-cursor" />}
                {m.error && <div style={{ color: "var(--danger)", fontSize: 12, marginTop: 4 }}>出错：{m.error}</div>}
              </div>
            </div>
          ))
        )}
      </div>

      {error && <div style={{ padding: "4px 12px", fontSize: 12, color: "var(--danger)" }}>{error}</div>}

      <div style={{ display: "flex", gap: 8, padding: 8, borderTop: "1px solid var(--border-default)" }}>
        <textarea
          className="form-input"
          style={{ flex: 1, resize: "vertical", minHeight: 88, fontFamily: "inherit" }}
          rows={4}
          placeholder={activeProviderId ? "输入消息，Enter 发送，Shift+Enter 换行" : "先配置一个 Provider"}
          value={input}
          disabled={!activeProviderId}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              handleSend();
            }
          }}
        />
        <button className="btn primary sm" onClick={handleSend} disabled={!activeProviderId || sending || !input.trim()}>
          <Send style={{ width: 14, height: 14 }} />
        </button>
      </div>

      {showProviders && <ProviderManagerDialog onClose={(hasDraft) => { setShowProviders(false); setProviderDraftPending(hasDraft); }} />}
    </div>
  );
};
