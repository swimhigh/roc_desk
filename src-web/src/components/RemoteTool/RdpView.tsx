import React, { useCallback, useEffect, useRef, useState } from "react";
import { MonitorX } from "lucide-react";
import { rdpService, type PanelBounds, type RdpStatus } from "../../services/rdpService";
import { formatError } from "../../utils/error";
import type { ConnectionProfile } from "../../types/bindings";

interface RdpViewProps {
  profile: ConnectionProfile;
  /** 对应这个会话标签是不是当前激活的那个——内嵌窗口是操作系统级别的原生窗口，
   * 不受 CSS display 影响，标签切走/切回必须显式调 rdp_hide/rdp_show（和
   * browser 模块内嵌子 WebView 是同一个道理，见其顶部注释）。*/
  visible: boolean;
}

type ViewStatus = "connecting" | "connected" | "error";

/**
 * RDP 远程桌面（远程工具模式，DESIGN.md §3.9）：不自己渲染画面——后端以 ActiveX
 * 方式激活 Windows 自带的 RDP 客户端控件，把它的窗口叠在这个组件占位 div 的屏幕
 * 区域上（见 rdp/mod.rs 顶部为什么弃用协议库、改走这条路的说明）。这个组件本身
 * 只负责：量测占位区域的屏幕坐标告诉后端摆哪、标签切换时显式隐藏/显示、卸载时
 * 断开，以及把后端报上来的真实连接状态显示出来。
 */
export const RdpView: React.FC<RdpViewProps> = ({ profile, visible }) => {
  const viewportRef = useRef<HTMLDivElement>(null);
  const sessionIdRef = useRef<string | null>(null);
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [status, setStatus] = useState<ViewStatus>("connecting");
  const [message, setMessage] = useState<string | null>(null);
  const [diagnostics, setDiagnostics] = useState<string | null>(null);

  const readBounds = useCallback((): PanelBounds | null => {
    const el = viewportRef.current;
    if (!el) return null;
    const rect = el.getBoundingClientRect();
    if (rect.width <= 0 || rect.height <= 0) return null;
    return { x: rect.left, y: rect.top, width: rect.width, height: rect.height };
  }, []);

  useEffect(() => {
    let cancelled = false;

    const setup = async () => {
      const bounds = readBounds();
      if (!bounds) {
        setStatus("error");
        setMessage("面板尚未就绪，请重新打开这个会话");
        return;
      }
      try {
        const id = await rdpService.connect(profile.id, bounds);
        if (cancelled) {
          void rdpService.disconnect(id);
          return;
        }
        sessionIdRef.current = id;
        setSessionId(id);
        // `rdp_connect` 返回只代表 ActiveX 控件创建/激活成功、`Connect()` 已经发出去
        // 了，**不代表** RDP 服务器已经连上、认证通过、画面开始传输。这里刻意不把
        // 状态置成 connected——之前就是在这里乐观地置了 connected，结果任何连接
        // 阶段的失败（认证失败/证书待确认/会话数超限）在界面上都表现成"一片黑，
        // 没有任何提示"，完全没法区分。真实状态一律由下面的轮询说了算。
        //
        // 下面那个"标签切走/切回"的 effect 只在 `visible` 发生*变化*时才触发——新开
        // 的标签从一开始就是 visible，那个 effect 早在 sessionId 还没拿到手时就跑过
        // 一次（当时拿不到 sessionId 直接短路返回了），之后不会再重跑，所以连接刚
        // 成功这一刻必须自己补一次 show，不能指望它兜底。
        if (visible) void rdpService.show(id, bounds);
      } catch (e) {
        if (!cancelled) {
          setStatus("error");
          setMessage(formatError(e));
        }
      }
    };
    void setup();

    return () => {
      cancelled = true;
      if (sessionIdRef.current) void rdpService.disconnect(sessionIdRef.current);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [profile.id]);

  // 轮询后端读到的控件真实状态（后端每 250ms 问一次控件自己的 `Connected` 属性，
  // 见 rdp/mod.rs 顶部"连接状态：轮询代替真事件下沉"）。
  useEffect(() => {
    if (!sessionId) return;
    const apply = (result: RdpStatus) => {
      setDiagnostics(result.diagnostics);
      if (result.state === "connected") {
        setStatus("connected");
        setMessage(null);
      } else if (result.state === "error" || result.state === "disconnected") {
        setStatus("error");
        setMessage(`RDP 连接已断开${result.reason ? `（错误码 ${result.reason}）` : ""}`);
      }
    };
    const timer = window.setInterval(() => {
      void rdpService.status(sessionId).then(apply).catch(() => undefined);
    }, 500);
    return () => window.clearInterval(timer);
  }, [sessionId]);

  // 标签切走/切回：显式隐藏/重新定位并显示内嵌窗口。
  useEffect(() => {
    const id = sessionIdRef.current;
    if (!id) return;
    if (visible) {
      const bounds = readBounds();
      if (bounds) void rdpService.show(id, bounds);
    } else {
      void rdpService.hide(id);
    }
  }, [visible, readBounds]);

  // 面板尺寸变化（窗口缩放、侧边栏拖拽调宽、下面的状态条出现/消失）时同步内嵌
  // 窗口的位置/大小。
  useEffect(() => {
    if (!visible) return;
    const el = viewportRef.current;
    if (!el) return;
    let raf = 0;
    const sync = () => {
      cancelAnimationFrame(raf);
      raf = requestAnimationFrame(() => {
        const id = sessionIdRef.current;
        if (!id) return;
        const bounds = readBounds();
        if (bounds) void rdpService.setBounds(id, bounds);
      });
    };
    const observer = new ResizeObserver(sync);
    observer.observe(el);
    window.addEventListener("resize", sync);
    return () => {
      cancelAnimationFrame(raf);
      observer.disconnect();
      window.removeEventListener("resize", sync);
    };
  }, [visible, readBounds]);

  return (
    <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
      {/* 状态条放在占位区**外面**（不是叠在上面）——内嵌的是原生窗口，会盖住它
          矩形范围内的一切网页内容，叠在上面的任何提示都看不见。连上之后这一条
          消失，把整块区域还给远程桌面。 */}
      {status !== "connected" && (
        <div
          className="remote-session-empty"
          style={{ flexDirection: "column", alignItems: "flex-start", gap: 6, padding: "10px 14px", flex: "0 0 auto" }}
        >
          <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
            <MonitorX style={{ width: 18, height: 18, color: "var(--text-disabled)" }} />
            <span>
              {status === "connecting" ? "正在连接远程桌面…" : `连接失败：${message ?? "未知错误"}`}
            </span>
            <span style={{ fontSize: 12, color: "var(--text-disabled)" }}>
              {profile.username}@{profile.host}:{profile.port}
            </span>
          </div>
          {diagnostics && (
            <div style={{ fontSize: 11, color: "var(--text-disabled)", fontFamily: "Consolas, monospace", wordBreak: "break-all" }}>
              {diagnostics}
            </div>
          )}
        </div>
      )}
      {/* 内嵌窗口按这个 div 的屏幕坐标摆放，div 本身不显示任何内容——只用来量测，
          必须始终挂载（不能用 display:none），否则量出来的尺寸永远是 0。 */}
      <div ref={viewportRef} style={{ flex: 1, minHeight: 0 }} />
    </div>
  );
};
