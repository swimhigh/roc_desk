import { useEffect, useRef, useState } from "react";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";

export type PaneSide = "remote" | "local";

export interface DndPayload {
  side: PaneSide;
  path: string;
  isDir: boolean;
  name: string;
}

interface UseDualPaneDndOptions {
  /** 面板内互拖（远程行拖到本地面板 = 下载，反之上传）。*/
  onInternalTransfer: (payload: DndPayload, targetSide: PaneSide) => void;
  /** 从 Windows 资源管理器等外部窗口把真实文件拖进远程面板——本地面板故意不接受
   * 外部拖入，本地面板本来就是本机文件系统，拖进来没有"传输"语义。*/
  onExternalUpload: (paths: string[]) => void;
}

/**
 * SftpBrowser/AgentBrowser 共用的双栏拖拽交互，拆成两条完全独立的路径：
 *
 * 1. 面板内互拖，刻意不用浏览器原生 HTML5 Drag and Drop（draggable + dataTransfer）
 *    ——Tauri 窗口 `dragDropEnabled` 默认就是 `true`（第 2 点的外部文件拖入要靠它），
 *    而 tauri-utils 里 `drag_drop_enabled` 字段的文档原话是"Disabling it is
 *    required to use HTML5 drag and drop on the frontend on Windows"，也就是说
 *    默认配置下 Windows 的 WebView2 会整个吃掉 HTML5 拖拽事件，原来那套
 *    onDragStart/onDrop 实际上根本不会触发。这里改用最朴素的鼠标事件
 *    （mousedown 记录来源、mousemove 判断当前悬停在哪个面板、mouseup 触发传输）
 *    手搓一套"虚拟拖拽"，完全绕开浏览器的 DnD API，不受这个开关影响。
 *
 * 2. 外部文件拖入用 Tauri 自己的 `onDragDropEvent`（前提正是那个默认打开的
 *    `dragDropEnabled`），坐标是物理像素，要按 devicePixelRatio 换算成 CSS 像素
 *    才能判断落在哪个面板的 DOM 矩形里。这个监听只注册一次，用 ref 转发最新的
 *    回调，避免 profileId/cwd 变化后监听器还拿着第一次渲染时的旧闭包。
 */
export function useDualPaneDnd({ onInternalTransfer, onExternalUpload }: UseDualPaneDndOptions) {
  const remoteRef = useRef<HTMLDivElement>(null);
  const localRef = useRef<HTMLDivElement>(null);
  const [dragOverSide, setDragOverSide] = useState<PaneSide | null>(null);

  const onExternalUploadRef = useRef(onExternalUpload);
  onExternalUploadRef.current = onExternalUpload;

  const sideAtPoint = (clientX: number, clientY: number): PaneSide | null => {
    const remoteRect = remoteRef.current?.getBoundingClientRect();
    if (remoteRect && clientX >= remoteRect.left && clientX <= remoteRect.right && clientY >= remoteRect.top && clientY <= remoteRect.bottom) {
      return "remote";
    }
    const localRect = localRef.current?.getBoundingClientRect();
    if (localRect && clientX >= localRect.left && clientX <= localRect.right && clientY >= localRect.top && clientY <= localRect.bottom) {
      return "local";
    }
    return null;
  };

  const beginDrag = (payload: DndPayload) => (e: React.MouseEvent) => {
    if (e.button !== 0) return; // 只认左键，右键要留给右键菜单
    e.preventDefault();
    const onMove = (ev: MouseEvent) => {
      const side = sideAtPoint(ev.clientX, ev.clientY);
      setDragOverSide(side && side !== payload.side ? side : null);
    };
    const onUp = (ev: MouseEvent) => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
      const side = sideAtPoint(ev.clientX, ev.clientY);
      setDragOverSide(null);
      if (side && side !== payload.side) onInternalTransfer(payload, side);
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  };

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    (async () => {
      const fn = await getCurrentWebviewWindow().onDragDropEvent((event) => {
        if (event.payload.type === "leave") {
          setDragOverSide(null);
          return;
        }
        const ratio = window.devicePixelRatio || 1;
        const side = sideAtPoint(event.payload.position.x / ratio, event.payload.position.y / ratio);
        if (event.payload.type === "drop") {
          setDragOverSide(null);
          if (side === "remote" && event.payload.paths.length > 0) onExternalUploadRef.current(event.payload.paths);
          return;
        }
        // 'enter' | 'over'：只在悬停到远程面板上时高亮——本地面板不接受外部拖入。
        setDragOverSide(side === "remote" ? "remote" : null);
      });
      if (cancelled) fn();
      else unlisten = fn;
    })();
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  return { remoteRef, localRef, dragOverSide, beginDrag };
}
