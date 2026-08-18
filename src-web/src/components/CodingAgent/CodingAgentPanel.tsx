import React, { useEffect, useRef, useState } from "react";
import { Send, Bot, User, GitCommitHorizontal } from "lucide-react";
import { useCodingStore } from "../../stores/codingStore";
import { useAiChatStore } from "../../stores/aiChatStore";
import { SegmentedControl } from "../shared/SegmentedControl";
import { ToggleSwitch } from "../shared/ToggleSwitch";
import { TargetBadge } from "./TargetBadge";
import { RemoteCapabilityBadge } from "./RemoteCapabilityBadge";
import { ToolCallProgress } from "./ToolCallProgress";
import { FileChangeCard, type DiffLine as CardDiffLine } from "./FileChangeCard";
import { CommandConfirmDialog, BlockedCommandMessage } from "./CommandConfirmDialog";
import type { CodingTarget } from "../../types/bindings";

interface CodingAgentPanelProps {
  workspaceId: string;
}

function targetLabel(target: CodingTarget): string {
  return target.kind === "Local" ? "本地" : target.host_label;
}

/**
 * AI 编程助手主面板（DESIGN.md §3.8.1）。DESIGN.md 草图是三栏布局（Explorer +
 * 对话 + 右栏 Monaco 实时编辑器），这里裁剪掉了独立的右栏编辑器——文件改动的 Diff
 * 已经用 FileChangeCard 内嵌展示，维护两份 Monaco 状态同步在 MVP 阶段收益不高，
 * 属于本轮有意识的范围裁剪（REQUIREMENTS.md §3.7 有记录）。左栏复用的是
 * App.tsx 里已经常驻的全局 Explorer，不在这个组件里重复渲染。
 */
export const CodingAgentPanel: React.FC<CodingAgentPanelProps> = ({ workspaceId }) => {
  const {
    sessionInfo,
    timeline,
    changesById,
    sending,
    error,
    confirmRequest,
    start,
    setMode,
    setAutoAllowReadonly,
    setAutoGitCommit,
    sendMessage,
    acceptChange,
    rejectChange,
    undoChange,
    resolveConfirm,
  } = useCodingStore();
  const providers = useAiChatStore((s) => s.providers);
  const loadProviders = useAiChatStore((s) => s.loadProviders);
  const [input, setInput] = useState("");
  const [selectedProviderId, setSelectedProviderId] = useState<string>("");
  const listRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    loadProviders();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    if (!selectedProviderId && providers.length > 0) setSelectedProviderId(providers[0].id);
  }, [providers, selectedProviderId]);

  useEffect(() => {
    listRef.current?.scrollTo({ top: listRef.current.scrollHeight });
  }, [timeline]);

  const handleStart = () => {
    if (!selectedProviderId) return;
    start(workspaceId, selectedProviderId);
  };

  const handleSend = () => {
    if (!input.trim()) return;
    sendMessage(input);
    setInput("");
  };

  if (!sessionInfo) {
    return (
      <div style={{ display: "flex", flexDirection: "column", alignItems: "center", justifyContent: "center", height: "100%", gap: 12 }}>
        {providers.length === 0 ? (
          <div style={{ fontSize: 13, color: "var(--text-secondary)" }}>请先在 AI 问答面板里配置一个 Provider</div>
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
              开始编程会话
            </button>
          </>
        )}
      </div>
    );
  }

  const isRemote = sessionInfo.target.kind === "Remote";

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      <div className="editor-toolbar" style={{ gap: 12, flexWrap: "wrap", height: "auto", minHeight: 32 }}>
        <SegmentedControl
          value={sessionInfo.mode}
          onChange={(m) => setMode(m)}
          options={[
            { value: "plan", label: "Plan" },
            { value: "build", label: "Build" },
          ]}
        />
        <TargetBadge targetLabel={targetLabel(sessionInfo.target)} isRemote={isRemote} />
        {isRemote && <RemoteCapabilityBadge />}
        {sessionInfo.mode === "build" && (
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

      <div ref={listRef} style={{ flex: 1, overflowY: "auto", padding: "8px 12px", display: "flex", flexDirection: "column", gap: 10 }}>
        {timeline.length === 0 ? (
          <div style={{ textAlign: "center", color: "var(--text-secondary)", fontSize: 13, marginTop: 24 }}>
            {sessionInfo.mode === "plan" ? "Plan 模式：AI 只分析建议，不会修改文件" : "描述你想做的改动"}
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
                  <div style={{ flex: 1, fontSize: 13, whiteSpace: "pre-wrap", wordBreak: "break-word", lineHeight: 1.6 }}>
                    {entry.text}
                  </div>
                </div>
              );
            }
            if (entry.kind === "tool") {
              return entry.running ? <ToolCallProgress key={entry.id} tool={entry.tool} elapsedMs={0} /> : null;
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
                onAccept={() => acceptChange(change.id)}
                onReject={() => rejectChange(change.id)}
                onUndo={() => undoChange(change.id)}
              />
            );
          })
        )}
      </div>

      {error && <div style={{ padding: "4px 12px", fontSize: 12, color: "var(--danger)" }}>{error}</div>}

      <div style={{ display: "flex", gap: 8, padding: 8, borderTop: "1px solid var(--border-default)" }}>
        <textarea
          className="form-input"
          style={{ flex: 1, resize: "none", fontFamily: "inherit" }}
          rows={2}
          placeholder="描述任务，Enter 发送，Shift+Enter 换行"
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              handleSend();
            }
          }}
        />
        <button className="btn primary sm" onClick={handleSend} disabled={sending || !input.trim()}>
          <Send style={{ width: 14, height: 14 }} />
        </button>
      </div>

      {confirmRequest && (
        <CommandConfirmDialog
          open
          host={confirmRequest.host ?? undefined}
          command={confirmRequest.command}
          onReject={() => resolveConfirm(false)}
          onAllowOnce={() => resolveConfirm(true)}
        />
      )}
    </div>
  );
};
