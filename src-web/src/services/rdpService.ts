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
  /** 原生窗口体检结果（句柄/可见性/实际屏幕矩形/控件自报 Connected 值）——黑屏时
   * 用来区分"根本没连上"和"连上了但窗口没摆对/被盖住"，直接显示给用户看。 */
  diagnostics: string;
}

/**
 * RDP 内嵌窗口（远程工具模式，DESIGN.md §3.9）：后端以 ActiveX 方式承载 Windows
 * 自带的 RDP 客户端控件（`mstscax.dll`），把它就地激活出来的窗口定位到这里传的
 * 屏幕区域——不是自己实现协议，见 rdp/mod.rs 顶部注释。没有 sendInput 之类的命令：
 * 这是一个真正的原生窗口，输入直接由操作系统送给它，不需要我们代理。
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
