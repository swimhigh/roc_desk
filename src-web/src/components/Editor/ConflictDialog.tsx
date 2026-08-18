import React from "react";
import { ConfirmDialog } from "../shared/ConfirmDialog";

interface ConflictDialogProps {
  open: boolean;
  path: string;
  onViewDiff: () => void;
  onSaveAsCopy: () => void;
  onOverwrite: () => void;
}

/**
 * 远程文件保存前的 mtime 冲突提示（DESIGN.md §3.1.4）。"仍要覆盖"样式弱化，
 * 不给默认高亮，逼用户先看一眼差异，除非确实清楚自己在做什么。
 */
export const ConflictDialog: React.FC<ConflictDialogProps> = ({
  open,
  path,
  onViewDiff,
  onSaveAsCopy,
  onOverwrite,
}) => (
  <ConfirmDialog
    open={open}
    severity="warning"
    icon="⚠"
    title="远程文件已被修改"
    dismissible
    onDismiss={onViewDiff}
    actions={
      <>
        <button className="btn ghost sm" onClick={onViewDiff}>查看差异</button>
        <button className="btn ghost sm" onClick={onSaveAsCopy}>另存为副本</button>
        <button className="btn weak sm" onClick={onOverwrite}>仍要覆盖</button>
      </>
    }
  >
    <p>
      <span className="fingerprint-block" style={{ fontFamily: "var(--font-mono)" }}>{path}</span>
      {" "}在你编辑期间已被其他进程修改。
    </p>
    <p style={{ fontSize: 12, color: "var(--text-secondary)", marginTop: 8 }}>
      直接覆盖会丢失对方的修改，建议先查看差异。
    </p>
  </ConfirmDialog>
);
