import React, { useEffect, useRef, useState } from "react";
import * as pdfjsLib from "pdfjs-dist";
import pdfWorkerUrl from "pdfjs-dist/build/pdf.worker.mjs?url";

pdfjsLib.GlobalWorkerOptions.workerSrc = pdfWorkerUrl;

interface PdfPreviewProps {
  /** base64（不含 data: 前缀）。 */
  base64: string;
}

/**
 * PDF 只读预览（2026-08-28 用户反馈：`<iframe src="data:application/pdf;...">`
 * 指望 WebView2 内置 PDF 插件接管渲染，实测在这个环境里是一片空白——WebView2 是否
 * 内置可用的 PDF 查看器和版本/系统配置有关，不可靠）。改用 `pdfjs-dist`（Mozilla
 * PDF.js，Chrome 自带 PDF 查看器就是它）在纯 JS 里把每一页画到 `<canvas>` 上，
 * 不依赖浏览器/WebView 自己的 PDF 插件，跨环境更稳。连续滚动展示全部页，不做分页/
 * 缩放控件——先解决"能看见内容"，更完整的阅读体验（缩放、跳页、搜索）按需再加。
 */
export const PdfPreview: React.FC<PdfPreviewProps> = ({ base64 }) => {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [pageCount, setPageCount] = useState(0);

  useEffect(() => {
    let cancelled = false;
    const renderTasks: pdfjsLib.RenderTask[] = [];
    setError(null);
    setPageCount(0);

    const binary = atob(base64);
    const bytes = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);

    const loadingTask = pdfjsLib.getDocument({ data: bytes });
    loadingTask.promise.then(
      async (pdf) => {
        if (cancelled) return;
        setPageCount(pdf.numPages);
        const container = containerRef.current;
        if (!container) return;
        container.innerHTML = "";

        // 按容器宽度算缩放比例，让页面撑满可视宽度——固定 scale 在高分屏/窄窗口下
        // 要么太糊要么裁不下，都不如按实际宽度自适应。
        const targetWidth = container.clientWidth || 800;

        for (let pageNum = 1; pageNum <= pdf.numPages; pageNum++) {
          if (cancelled) return;
          const page = await pdf.getPage(pageNum);
          const unscaledViewport = page.getViewport({ scale: 1 });
          const scale = targetWidth / unscaledViewport.width;
          const viewport = page.getViewport({ scale });

          const canvas = document.createElement("canvas");
          canvas.width = viewport.width;
          canvas.height = viewport.height;
          canvas.style.display = "block";
          canvas.style.margin = "0 auto 12px";
          canvas.style.boxShadow = "0 1px 4px rgba(0,0,0,0.3)";
          container.appendChild(canvas);

          const ctx = canvas.getContext("2d");
          if (!ctx) continue;
          const task = page.render({ canvasContext: ctx, viewport, canvas });
          renderTasks.push(task);
          await task.promise.catch(() => {});
        }
      },
      (e) => {
        if (cancelled) return;
        setError(e instanceof Error ? e.message : String(e));
      },
    );

    return () => {
      cancelled = true;
      renderTasks.forEach((t) => t.cancel());
      loadingTask.destroy();
    };
  }, [base64]);

  if (error) {
    return (
      <div style={{ padding: 16, fontSize: 12, color: "var(--danger)" }}>
        PDF 解析失败：{error}
      </div>
    );
  }

  return (
    <div style={{ flex: 1, minHeight: 0, overflow: "auto", background: "var(--bg-base)" }}>
      {/* containerRef 这个节点完全交给上面的 effect 用 innerHTML/appendChild 命令式
          管理——不能让 React 也往里塞子节点（之前"加载中…"和 <canvas> 共享同一个
          容器，effect 里的 container.innerHTML = "" 会把 React 渲染的"加载中" 节点
          从 DOM 里删掉而 React 自己不知道；等 setPageCount 触发重新渲染、React 想把
          它认为还在的那个节点摘掉时，节点已经不在了，抛
          "Failed to execute 'removeChild'...: The node to be removed is not a child
          of this node"）。"加载中" 提示挪到外层这个纯 React 管理的兄弟节点里，两者
          互不干扰。 */}
      {pageCount === 0 && <div style={{ padding: 16, fontSize: 12, color: "var(--text-secondary)" }}>加载中…</div>}
      <div ref={containerRef} style={{ padding: 16 }} />
    </div>
  );
};
