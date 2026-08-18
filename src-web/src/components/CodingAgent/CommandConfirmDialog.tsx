import React from "react";
import { ConfirmDialog } from "../shared/ConfirmDialog";

interface CommandConfirmDialogProps {
  open: boolean;
  host?: string; // 未提供表示本地目标
  command: string;
  onReject: () => void;
  onAllowOnce: () => void;
}

/**
 * 高危命令二次确认（DESIGN.md §3.8.2.1）。黑名单命中不用这个弹窗——那种情况直接在
 * 对话流里插入不可操作的 <BlockedCommandMessage>，不给任何绕过按钮，见同目录组件。
 */
export const CommandConfirmDialog: React.FC<CommandConfirmDialogProps> = ({
  open,
  host,
  command,
  onReject,
  onAllowOnce,
}) => (
  <ConfirmDialog
    open={open}
    severity="warning"
    icon="⚠"
    title="确认执行命令"
    dismissible
    onDismiss={onReject}
    actions={
      <>
        <button className="btn ghost sm" onClick={onReject}>拒绝</button>
        <button className="btn primary sm" onClick={onAllowOnce}>仅本次允许</button>
      </>
    }
  >
    {host && <div className="cmd-confirm-host">目标主机: {host} · 远程</div>}
    <div style={{ fontSize: 12, color: "var(--text-secondary)", marginBottom: 4 }}>命令：</div>
    <div className="cmd-confirm-code">$ {command}</div>
    <p style={{ fontSize: 12, color: "var(--text-secondary)" }}>
      此命令将{host ? "在远程主机上" : "在本地"}执行，请确认你了解其影响。
    </p>
  </ConfirmDialog>
);

interface BlockedCommandMessageProps {
  command: string;
}

/** 黑名单命中：不弹窗，直接在对话流中插入不可操作的系统消息（DESIGN.md §3.8.2.1）。*/
export const BlockedCommandMessage: React.FC<BlockedCommandMessageProps> = ({ command }) => (
  <div className="blocked-cmd-msg">
    🛑 已拦截高危命令：{command}，如需执行请前往终端手动操作
  </div>
);
