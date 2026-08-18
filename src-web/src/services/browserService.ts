import { invoke } from "@tauri-apps/api/core";
import type { BrowserHistoryEntry } from "../types/bindings";

export interface PanelBounds {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** IPC 边界（CODE_DESIGN.md §一分层原则）：网页浏览（DESIGN.md §3.5）。
 * 网页内容由后端在主窗口内嵌一个独立的子 WebView 承载（IPC 隔离，不共享主窗口的
 * Tauri 权限），定位到前端传来的面板矩形区域——不是弹出独立窗口（2026-08-18 由
 * 用户明确要求改为内嵌，见 browser 模块的注释）。前端负责测量面板区域 + 在窗口/
 * 侧边栏尺寸变化时同步。 */
export const browserService = {
  /** 打开或复用已有子 WebView 导航到 url，返回规整后的实际 URL。 */
  open(url: string, bounds: PanelBounds): Promise<string> {
    return invoke("browser_open", { url, bounds });
  },
  setBounds(bounds: PanelBounds): Promise<void> {
    return invoke("browser_set_bounds", { bounds });
  },
  hide(): Promise<void> {
    return invoke("browser_hide");
  },
  show(bounds: PanelBounds): Promise<void> {
    return invoke("browser_show", { bounds });
  },
  close(): Promise<void> {
    return invoke("browser_close");
  },
  historyList(limit?: number): Promise<BrowserHistoryEntry[]> {
    return invoke("browser_history_list", { limit: limit ?? null });
  },
  historyRemove(id: string): Promise<void> {
    return invoke("browser_history_remove", { id });
  },
  historyClear(): Promise<void> {
    return invoke("browser_history_clear");
  },
};
