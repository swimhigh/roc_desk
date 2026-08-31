import React from "react";
import { ConfirmDialog } from "../shared/ConfirmDialog";

interface TofuProps {
  kind: "tofu";
  host: string;
  fingerprint: string;
  onCancel: () => void;
  onTrust: () => void;
}

interface ChangedProps {
  kind: "changed";
  host: string;
  oldFingerprint: string;
  newFingerprint: string;
  onCancel: () => void;
  onTrustAnyway: () => void;
}

type AgentCertDialogProps = { open: boolean } & (TofuProps | ChangedProps);

/**
 * Agent TLS 证书指纹确认弹窗（AGENT_DESIGN.md §3.1）：和 `HostKeyDialog` 是同一套
 * 视觉/交互模式（TOFU 温和高亮，指纹变化不可通过 Esc/遮罩关闭），文案换成
 * "证书指纹"而不是 SSH 的"密钥指纹"。
 */
export const AgentCertDialog: React.FC<AgentCertDialogProps> = (props) => {
  if (props.kind === "tofu") {
    const { open, host, fingerprint, onCancel, onTrust } = props;
    return (
      <ConfirmDialog
        open={open}
        severity="info"
        icon="ℹ"
        title="首次连接该 Agent"
        dismissible
        onDismiss={onCancel}
        actions={
          <>
            <button className="btn ghost sm" onClick={onCancel}>取消</button>
            <button className="btn primary sm" onClick={onTrust}>信任并连接</button>
          </>
        }
      >
        <p>这是你首次连接该 roc_desk_agent，请确认其 TLS 证书指纹：</p>
        <div style={{ margin: "8px 0" }}>
          <div style={{ fontSize: 12, color: "var(--text-secondary)" }}>host</div>
          <div style={{ fontFamily: "var(--font-mono)", fontSize: 13 }}>{host}</div>
        </div>
        <div style={{ margin: "8px 0" }}>
          <div style={{ fontSize: 12, color: "var(--text-secondary)" }}>证书指纹（SHA-256）</div>
          <div className="fingerprint-block">{fingerprint}</div>
        </div>
      </ConfirmDialog>
    );
  }

  const { open, host, oldFingerprint, newFingerprint, onCancel, onTrustAnyway } = props;
  return (
    <ConfirmDialog
      open={open}
      severity="danger"
      icon="🛑"
      title="安全警告：Agent 证书指纹已变化"
      dismissible={false}
      actions={
        <>
          <button className="btn danger-strong sm" onClick={onCancel}>取消连接</button>
          <button className="btn weak sm" onClick={onTrustAnyway}>仍要信任 (危险)</button>
        </>
      }
    >
      <p className="warning-text">该 Agent 的证书指纹与已保存的记录不一致，可能是重装了 Agent，也可能遭遇中间人攻击。</p>
      <div style={{ margin: "12px 0" }}>
        <div style={{ fontSize: 12, color: "var(--text-secondary)" }}>host</div>
        <div style={{ fontFamily: "var(--font-mono)", fontSize: 13 }}>{host}</div>
      </div>
      <div style={{ margin: "8px 0" }}>
        <div style={{ fontSize: 12, color: "var(--text-secondary)" }}>旧指纹</div>
        <div className="fingerprint-block" style={{ color: "var(--text-secondary)" }}>{oldFingerprint}</div>
      </div>
      <div style={{ margin: "8px 0" }}>
        <div style={{ fontSize: 12, color: "var(--text-secondary)" }}>新指纹</div>
        <div className="fingerprint-block" style={{ color: "var(--danger)" }}>{newFingerprint}</div>
      </div>
      <p style={{ fontSize: 12, color: "var(--text-secondary)", marginTop: 12 }}>
        此弹窗不可通过点击遮罩或按 Esc 关闭，必须显式选择。
      </p>
    </ConfirmDialog>
  );
};
