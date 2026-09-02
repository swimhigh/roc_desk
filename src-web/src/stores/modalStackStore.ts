import { create } from "zustand";

interface ModalStackState {
  /** 当前处于 open 状态的 ConfirmDialog 系弹窗数量（可能同时叠多层）。 */
  count: number;
  push: () => void;
  pop: () => void;
}

/**
 * 全局弹窗计数：原生子 WebView（浏览器面板）不参与 CSS 层叠，任何 HTML 弹窗打开
 * 时都必须显式隐藏它，否则弹窗会被浏览器内容盖住（见 browser 模块顶部注释）。
 * ConfirmDialog 在 open 时 push、关闭/卸载时 pop，WebBrowserPanel 订阅 count>0
 * 来临时隐藏当前激活标签页的子 WebView。
 */
export const useModalStackStore = create<ModalStackState>((set) => ({
  count: 0,
  push: () => set((s) => ({ count: s.count + 1 })),
  pop: () => set((s) => ({ count: Math.max(0, s.count - 1) })),
}));
