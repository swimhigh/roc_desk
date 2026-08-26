import { invoke } from "@tauri-apps/api/core";

export interface PanelBounds {
  x: number;
  y: number;
  width: number;
  height: number;
}
export interface RdpStatus {
  state: "connecting" | "connected" | "disconnected" | "error";
  reason: number | null;
  /** 内嵌窗口的体检结果（句柄/可见性/尺寸）——嵌入的是 wfreerdp.exe（FreeRDP）的
   * 窗口，拿不到 RDP 协议层面的连接状态，只能诚实报告"眼下嵌的是哪个窗口"。 */
  diagnostics: string;
}

/**
 * RDP 内嵌窗口（远程工具模式，DESIGN.md §3.9）：后端拉起 wfreerdp.exe（FreeRDP）
 * 并把它的窗口挪进 roc_desk 主窗口、定位到这里传的屏幕区域——不是自己实现协议，
 * 见 rdp/mod.rs 顶部注释。没有 sendInput 之类的命令：这是一个真正的原生窗口，
 * 输入直接由操作系统送给它，不需要我们代理。
 */
export const rdpService = {
  connect(profileId: string, bounds: PanelBounds): Promise<string> {
    return invoke("rdp_connect", { profileId, bounds });
  },
  setBounds(sessionId: string, bounds: PanelBounds): Promise<void> {
    return invoke("rdp_set_bounds", { sessionId, bounds });
  },
  hide(sessionId: string): Promise<void> {
    return invoke("rdp_hide", { sessionId });
  },
  show(sessionId: string, bounds: PanelBounds): Promise<void> {
    return invoke("rdp_show", { sessionId, bounds });
  },
  disconnect(sessionId: string): Promise<void> {
    return invoke("rdp_disconnect", { sessionId });
  },
  status(sessionId: string): Promise<RdpStatus> {
    return invoke("rdp_status", { sessionId });
  },
};
