import { useEffect, useRef, useState } from "react";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";

/**
 * 单个目标区域接收"从 Windows 资源管理器等外部窗口把真实文件拖进来"——不能用
 * 浏览器原生 HTML5 Drag and Drop（`ondrop`/`dataTransfer.files`）：Tauri 窗口
 * `dragDropEnabled` 默认是 `true`，tauri-utils 里 `drag_drop_enabled` 字段的
 * 文档原话是"Disabling it is required to use HTML5 drag and drop on the
 * frontend on Windows"——默认配置下 Windows 的 WebView2 会整个吃掉 HTML5 拖拽
 * 事件，原生的 onDragOver/onDrop 处理器实际上根本不会触发，`dataTransfer.files`
 * 永远是空的。`SftpBrowser`/`AgentBrowser` 的双栏拖拽（见 `useDualPaneDnd.ts`）
 * 已经踩过这个坑，这里是同一个模式的单目标区简化版——AI 对话框需要接收拖进来
 * 的本地文件当附件，同样绕不开这个限制（2026-09 用户反馈"PDF 拖不进输入框"，
 * 根因就是这个，不是附件处理逻辑本身的问题）。
 *
 * Tauri 的 `onDragDropEvent` 是整个窗口级别的事件流（坐标是物理像素，要按
 * `devicePixelRatio` 换算成 CSS 像素），不是绑在某个 DOM 元素上的——用传进来的
 * `targetRef` 做命中测试，判断当前指针是不是悬停/落在这个目标区域内。
 *
 * `event.payload.paths` 是磁盘绝对路径（字符串数组），不是浏览器 `File`
 * 对象——拿到路径之后调用方还需要自己再读一次文件内容（比如后端的
 * `local_read_binary_preview`/`local_read_file`），这一层只负责"识别出这次
 * 外部拖拽落在了我的目标区域，把路径交给你"。
 */
export function useExternalFileDrop(
  targetRef: React.RefObject<HTMLElement | null>,
  onFilesDropped: (paths: string[]) => void,
) {
  const [isDragOver, setIsDragOver] = useState(false);
  const onFilesDroppedRef = useRef(onFilesDropped);
  onFilesDroppedRef.current = onFilesDropped;

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    (async () => {
      const fn = await getCurrentWebviewWindow().onDragDropEvent((event) => {
        if (event.payload.type === "leave") {
          setIsDragOver(false);
          return;
        }
        const rect = targetRef.current?.getBoundingClientRect();
        if (!rect) {
          setIsDragOver(false);
          return;
        }
        const ratio = window.devicePixelRatio || 1;
        const x = event.payload.position.x / ratio;
        const y = event.payload.position.y / ratio;
        const inside = x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom;
        if (event.payload.type === "drop") {
          setIsDragOver(false);
          if (inside && event.payload.paths.length > 0) onFilesDroppedRef.current(event.payload.paths);
          return;
        }
        // 'enter' | 'over'：只在悬停到目标区域上时才显示"可以放这里"的高亮。
        setIsDragOver(inside);
      });
      if (cancelled) fn();
      else unlisten = fn;
    })();
    return () => {
      cancelled = true;
      unlisten?.();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return { isDragOver };
}
