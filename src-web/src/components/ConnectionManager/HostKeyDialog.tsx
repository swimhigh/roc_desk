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

type HostKeyDialogProps = { open: boolean } & (TofuProps | ChangedProps);

/**
 * 主机指纹确认弹窗（DESIGN.md §3.2.1，UI_DESIGN.md §4.1）。
 * TOFU 与指纹变化是两种完全不同的视觉强度，不能共用一套"温和"样式：
 * TOFU 给默认高亮按钮；指纹变化不给默认按钮、危险操作按钮弱化、且不可通过 Esc/遮罩关闭。
 */
export const HostKeyDialog: React.FC<HostKeyDialogProps> = (props) => {
  if (props.kind === "tofu") {
    const { open, host, fingerprint, onCancel, onTrust } = props;
    return (
      <ConfirmDialog
        open={open}
        severity="info"
        icon="ℹ"
        title="首次连接该主机"
        dismissible
        onDismiss={onCancel}
        actions={
          <>
            <button className="btn ghost sm" onClick={onCancel}>取消</button>
            <button className="btn primary sm" onClick={onTrust}>信任并连接</button>
          </>
        }
      >
        <p>这是你首次连接该主机，请确认其密钥指纹：</p>
        <div style={{ margin: "8px 0" }}>
          <div style={{ fontSize: 12, color: "var(--text-secondary)" }}>host</div>
          <div style={{ fontFamily: "var(--font-mono)", fontSize: 13 }}>{host}</div>
        </div>
        <div style={{ margin: "8px 0" }}>
          <div style={{ fontSize: 12, color: "var(--text-secondary)" }}>指纹</div>
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
      title="安全警告：主机指纹已变化"
      dismissible={false}
      actions={
        <>
          <button className="btn danger-strong sm" onClick={onCancel}>取消连接</button>
          <button className="btn weak sm" onClick={onTrustAnyway}>仍要信任 (危险)</button>
        </>
      }
    >
      <p className="warning-text">该主机的密钥指纹与已保存的记录不一致，可能遭遇中间人攻击或主机被重装。</p>
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
