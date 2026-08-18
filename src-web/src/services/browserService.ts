import { invoke } from "@tauri-apps/api/core";
import type { BrowserHistoryEntry } from "../types/bindings";

export interface PanelBounds {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** IPC 边界（CODE_DESIGN.md §一分层原则）：网页浏览（DESIGN.md §3.5）。
 * 网页内容由后端在主窗口内嵌若干个独立的子 WebView 承载（每个标签页一个，IPC 隔离，
 * 不共享主窗口的 Tauri 权限），定位到前端传来的面板矩形区域——不是弹出独立窗口
 * （2026-08-18 由用户明确要求改为内嵌+多标签页，见 browser 模块的注释）。`tabId` 由
 * 前端生成的 UUID 标识每个标签页对应的子 WebView。前端负责测量面板区域 + 在窗口/
 * 侧边栏尺寸变化时同步。 */
export const browserService = {
  /** 打开或复用已有子 WebView 导航到 url，返回规整后的实际 URL。 */
  open(tabId: string, url: string, bounds: PanelBounds): Promise<string> {
    return invoke("browser_open", { tabId, url, bounds });
  },
  setBounds(tabId: string, bounds: PanelBounds): Promise<void> {
    return invoke("browser_set_bounds", { tabId, bounds });
  },
  hide(tabId: string): Promise<void> {
    return invoke("browser_hide", { tabId });
  },
  show(tabId: string, bounds: PanelBounds): Promise<void> {
    return invoke("browser_show", { tabId, bounds });
  },
  close(tabId: string): Promise<void> {
    return invoke("browser_close", { tabId });
  },
  closeAll(): Promise<void> {
    return invoke("browser_close_all");
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
