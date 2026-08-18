import React from "react";

export type DialogSeverity = "info" | "warning" | "danger";

interface ConfirmDialogProps {
  open: boolean;
  severity?: DialogSeverity;
  icon?: React.ReactNode;
  title: string;
  children: React.ReactNode;
  actions: React.ReactNode;
  /** 高危确认类弹窗不响应 Esc / 点击遮罩关闭（UI_DESIGN.md §4.1、§十二）*/
  dismissible?: boolean;
  onDismiss?: () => void;
}

/**
 * 所有阻塞式确认弹窗的基座：HostKeyDialog、CommandConfirmDialog、ConflictDialog
 * 都基于它二次封装，保证视觉/键盘行为统一（docs/prototypes/devhub-dialogs.html §1/§2/§9）。
 */
export const ConfirmDialog: React.FC<ConfirmDialogProps> = ({
  open,
  severity = "info",
  icon,
  title,
  children,
  actions,
  dismissible = true,
  onDismiss,
}) => {
  React.useEffect(() => {
    if (!open || !dismissible) return;
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") onDismiss?.();
    };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [open, dismissible, onDismiss]);

  if (!open) return null;

  const titleBarClass = severity === "danger" ? "dialog-title-bar danger" : `dialog-title-bar ${severity}`;

  return (
    <div
      className="dialog-overlay"
      onClick={(e) => {
        if (dismissible && e.target === e.currentTarget) onDismiss?.();
      }}
    >
      <div className="dialog" role="alertdialog" aria-modal="true">
        <div className={titleBarClass}>
          {icon && <span>{icon}</span>}
          <span>{title}</span>
        </div>
        <div className="dialog-body">{children}</div>
        <div className="dialog-actions">{actions}</div>
      </div>
    </div>
  );
};
