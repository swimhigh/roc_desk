import React from "react";
import { create } from "zustand";

export type ToastVariant = "success" | "info" | "error";

interface ToastEntry {
  id: string;
  variant: ToastVariant;
  message: string;
  action?: { label: string; onClick: () => void };
}

interface ToastState {
  toasts: ToastEntry[];
  push: (variant: ToastVariant, message: string, action?: ToastEntry["action"]) => void;
  dismiss: (id: string) => void;
}

/** 非阻塞提示（UI_DESIGN.md §十一），自动消失 3-5s，除非带 action（如"重试保存"）。*/
export const useToastStore = create<ToastState>((set) => ({
  toasts: [],
  push: (variant, message, action) => {
    const id = crypto.randomUUID();
    set((s) => ({ toasts: [...s.toasts, { id, variant, message, action }] }));
    if (!action) {
      setTimeout(() => {
        set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) }));
      }, 4000);
    }
  },
  dismiss: (id) => set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) })),
}));

const ICONS: Record<ToastVariant, string> = { success: "✓", info: "ℹ", error: "✗" };

export const ToastStack: React.FC = () => {
  const { toasts, dismiss } = useToastStore();
  return (
    <div className="toast-stack">
      {toasts.map((t) => (
        <div key={t.id} className={`toast ${t.variant}`} onClick={() => dismiss(t.id)}>
          <span>{ICONS[t.variant]}</span>
          <span>{t.message}</span>
          {t.action && (
            <button
              className="btn ghost sm"
              onClick={(e) => {
                e.stopPropagation();
                t.action!.onClick();
                dismiss(t.id);
              }}
            >
              {t.action.label}
            </button>
          )}
        </div>
      ))}
    </div>
  );
};
