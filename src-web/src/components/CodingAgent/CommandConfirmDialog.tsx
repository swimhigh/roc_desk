import React, { useMemo, useState } from "react";
import { ConfirmDialog } from "../shared/ConfirmDialog";
import { highlightShellCommand } from "../../utils/shellHighlight";

interface CommandConfirmDialogProps {
  open: boolean;
  host?: string; // 未提供表示本地目标
  command: string;
  kind: "command" | "mcp";
  /** 用户点"记住此模式"时的默认建议模式（可编辑）——命令类是"首个词 + ' *'"，
   * MCP 类是后端传回的 `matchKey`（`"<server>:<tool>"`）。 */
  suggestedPattern: string;
  onReject: () => void;
  onAllowOnce: () => void;
  onAllowAndRemember: (pattern: string) => void;
}

/**
 * 高危命令 / MCP 工具调用二次确认（DESIGN.md §3.8.2.1、REQUIREMENTS.md §3.7 权限
 * 引擎升级）。黑名单命中不用这个弹窗——那种情况直接在对话流里插入不可操作的
 * <BlockedCommandMessage>，不给任何绕过按钮，见同目录组件。
 *
 * "记住此模式"不是简单的"以后都不问了"——落地成一条 `allow` 权限规则（按用户可
 * 编辑的模式匹配），用户随时能在权限规则管理里看到、改、删，不是一个隐藏的
 * 会话内开关。
 */
export const CommandConfirmDialog: React.FC<CommandConfirmDialogProps> = ({
  open,
  host,
  command,
  kind,
  suggestedPattern,
  onReject,
  onAllowOnce,
  onAllowAndRemember,
}) => {
  const [rememberMode, setRememberMode] = useState(false);
  const [pattern, setPattern] = useState(suggestedPattern);

  React.useEffect(() => {
    setRememberMode(false);
    setPattern(suggestedPattern);
  }, [suggestedPattern, open]);

  const title = kind === "mcp" ? "确认 MCP 工具调用" : "确认执行命令";
  const label = kind === "mcp" ? "调用：" : "命令：";
  const highlightedCommand = useMemo(() => (kind === "mcp" ? null : highlightShellCommand(command)), [kind, command]);

  return (
    <ConfirmDialog
      open={open}
      severity="warning"
      icon="⚠"
      title={title}
      dismissible
      onDismiss={onReject}
      actions={
        <>
          <button className="btn ghost sm" onClick={onReject}>拒绝</button>
          <button className="btn ghost sm" onClick={onAllowOnce}>仅本次允许</button>
          {rememberMode ? (
            <button className="btn primary sm" onClick={() => onAllowAndRemember(pattern)} disabled={!pattern.trim()}>
              确认并保存规则
            </button>
          ) : (
            <button className="btn primary sm" onClick={() => setRememberMode(true)}>允许并记住</button>
          )}
        </>
      }
    >
      {host && <div className="cmd-confirm-host">目标主机: {host} · 远程</div>}
      <div style={{ fontSize: 12, color: "var(--text-secondary)", marginBottom: 4 }}>{label}</div>
      <div className="cmd-confirm-code">
        {kind === "mcp" ? (
          command
        ) : (
          <>
            <span className="cmd-confirm-prompt">$</span>
            <span dangerouslySetInnerHTML={{ __html: highlightedCommand ?? "" }} />
          </>
        )}
      </div>
      <p style={{ fontSize: 12, color: "var(--text-secondary)" }}>
        此{kind === "mcp" ? "工具调用" : "命令"}将{host ? "在远程主机上" : "本地"}执行，请确认你了解其影响。
      </p>
      {rememberMode && (
        <div className="form-row" style={{ marginTop: 8 }}>
          <label className="form-label">以后自动放行匹配此模式的{kind === "mcp" ? "调用" : "命令"}（支持 * / ?）</label>
          <input className="form-input" value={pattern} onChange={(e) => setPattern(e.target.value)} />
        </div>
      )}
    </ConfirmDialog>
  );
};

interface BlockedCommandMessageProps {
  command: string;
}

/** 黑名单命中：不弹窗，直接在对话流中插入不可操作的系统消息（DESIGN.md §3.8.2.1）。*/
export const BlockedCommandMessage: React.FC<BlockedCommandMessageProps> = ({ command }) => (
  <div className="blocked-cmd-msg">
    🛑 已拦截高危命令：{command}，如需执行请前往终端手动操作
  </div>
);
