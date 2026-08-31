import React, { useState } from "react";
import { ConfirmDialog } from "../shared/ConfirmDialog";

interface PasswordPromptDialogProps {
  open: boolean;
  connectionName: string;
  onCancel: () => void;
  onSubmit: (password: string) => void;
  submitting?: boolean;
  /** 复用同一个弹窗给 Agent 连接补录配对令牌时传 "配对令牌"，默认 "密码"
   * （AGENT_DESIGN.md：Agent 协议用配对令牌而不是密码，但补救交互完全一样，
   * 不值得单独建一个弹窗组件）。 */
  secretLabel?: string;
}

/**
 * 连接缺少已保存密码时的补救弹窗（真实 bug：`keyring = "3"` 没开
 * `windows-native` feature，之前所有"已保存"的密码其实都没真的写进系统密钥链，
 * 导致已保存的连接一律"missing password"且没有任何办法补救——加这个弹窗，
 * 让用户在连接失败的当下直接把密码补上并持久化，而不必去一个还不存在的
 * "编辑连接"页面）。
 */
export const PasswordPromptDialog: React.FC<PasswordPromptDialogProps> = ({
  open,
  connectionName,
  onCancel,
  onSubmit,
  submitting,
  secretLabel = "密码",
}) => {
  const [password, setPassword] = useState("");
  const [visible, setVisible] = useState(false);

  return (
    <ConfirmDialog
      open={open}
      severity="warning"
      icon="🔑"
      title={`需要重新输入${secretLabel}`}
      dismissible={!submitting}
      onDismiss={onCancel}
      actions={
        <>
          <button className="btn ghost sm" onClick={onCancel} disabled={submitting}>
            取消
          </button>
          <button
            className="btn primary sm"
            disabled={!password || submitting}
            onClick={() => onSubmit(password)}
          >
            {submitting ? "连接中…" : "保存并连接"}
          </button>
        </>
      }
    >
      <p style={{ marginBottom: 8 }}>
        连接 <strong>{connectionName}</strong> 没有已保存的{secretLabel}（或已失效），请重新输入。
      </p>
      <div className="form-input-group">
        <input
          className="form-input"
          type={visible ? "text" : "password"}
          value={password}
          autoFocus
          onChange={(e) => setPassword(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && password && !submitting) onSubmit(password);
          }}
        />
        <button className="btn ghost sm" onClick={() => setVisible((v) => !v)}>
          {visible ? "隐藏" : "显示"}
        </button>
      </div>
    </ConfirmDialog>
  );
};
