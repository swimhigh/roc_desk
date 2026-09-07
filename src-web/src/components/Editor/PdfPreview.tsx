import React, { useCallback, useEffect, useRef, useState } from "react";
import * as pdfjsLib from "pdfjs-dist";
import type { PDFDocumentProxy, RenderTask, TextLayer as PdfTextLayer } from "pdfjs-dist";
import pdfWorkerUrl from "pdfjs-dist/build/pdf.worker.mjs?url";
import { ChevronDown, ChevronRight, ChevronUp, List, Search, X, ZoomIn, ZoomOut } from "lucide-react";

pdfjsLib.GlobalWorkerOptions.workerSrc = pdfWorkerUrl;

interface PdfPreviewProps {
  /** base64（不含 data: 前缀）。 */
  base64: string;
}

interface OutlineEntry {
  title: string;
  dest: string | unknown[] | null;
  items: OutlineEntry[];
}

interface PageTextIndex {
  divs: HTMLElement[];
  ranges: [number, number][];
  text: string;
}

interface SearchMatch {
  pageNum: number;
  divs: HTMLElement[];
}

function normalizeOutline(nodes: any[] | null | undefined): OutlineEntry[] {
  if (!nodes) return [];
  return nodes.map((n) => ({
    title: String(n?.title ?? "未命名"),
    dest: n?.dest ?? null,
    items: normalizeOutline(n?.items),
  }));
}

const MIN_ZOOM = 0.25;
const MAX_ZOOM = 4;
const ZOOM_STEP = 1.2;

const OutlineNode: React.FC<{ item: OutlineEntry; depth: number; onNavigate: (dest: OutlineEntry["dest"]) => void }> = ({
  item,
  depth,
  onNavigate,
}) => {
  const [expanded, setExpanded] = useState(true);
  const hasChildren = item.items.length > 0;
  return (
    <li>
      <div
        className="pdf-outline-item"
        style={{ paddingLeft: depth * 12 + 4 }}
        title={item.title}
        onClick={() => onNavigate(item.dest)}
      >
        {hasChildren ? (
          <span
            className="pdf-outline-toggle"
            onClick={(e) => {
              e.stopPropagation();
              setExpanded((v) => !v);
            }}
          >
            {expanded ? <ChevronDown style={{ width: 11, height: 11 }} /> : <ChevronRight style={{ width: 11, height: 11 }} />}
          </span>
        ) : (
          <span className="pdf-outline-toggle" />
        )}
        <span className="pdf-outline-title">{item.title}</span>
      </div>
      {hasChildren && expanded && (
        <ul className="pdf-outline-list">
          {item.items.map((child, i) => (
            <OutlineNode key={i} item={child} depth={depth + 1} onNavigate={onNavigate} />
          ))}
        </ul>
      )}
    </li>
  );
};

/**
 * PDF 只读预览（2026-08-28 用户反馈：`<iframe src="data:application/pdf;...">`
 * 指望 WebView2 内置 PDF 插件接管渲染，实测在这个环境里是一片空白——WebView2 是否
 * 内置可用的 PDF 查看器和版本/系统配置有关，不可靠）。改用 `pdfjs-dist`（Mozilla
 * PDF.js，Chrome 自带 PDF 查看器就是它）在纯 JS 里把每一页画到 `<canvas>` 上，
 * 不依赖浏览器/WebView 自己的 PDF 插件，跨环境更稳。
 *
 * 2026-09-07 补充缩放/搜索/大纲目录：每页额外叠一层 pdfjs 的 `TextLayer`（透明文字，
 * 位置和画布一一对应）——既让搜索能高亮命中的文字段，也顺带让用户能选中/复制正文。
 * 搜索命中粒度是"文字段"（pdf.js 内部按字体/位置切片的 span），不是逐字符精确范围：
 * 完全照抄 pdf.js 自带 viewer 的 find controller 需要引入一整套 web/ 层（事件总线、
 * LinkService 等），对这个轻量预览来说成本远大于收益，span 级高亮已经足够定位到目标。
 */
export const PdfPreview: React.FC<PdfPreviewProps> = ({ base64 }) => {
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const pagesContainerRef = useRef<HTMLDivElement | null>(null);
  const pageWrapperRefs = useRef<Map<number, HTMLDivElement>>(new Map());
  const textIndexRef = useRef<Map<number, PageTextIndex>>(new Map());
  const highlightedRef = useRef<HTMLElement[]>([]);
  const pdfRef = useRef<PDFDocumentProxy | null>(null);
  const baseScaleRef = useRef(1);
  const searchQueryRef = useRef("");

  const [error, setError] = useState<string | null>(null);
  const [pageCount, setPageCount] = useState(0);
  const [outline, setOutline] = useState<OutlineEntry[] | null>(null);
  const [outlineOpen, setOutlineOpen] = useState(false);
  const [zoom, setZoom] = useState(1);
  const [searchOpen, setSearchOpen] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const [matches, setMatches] = useState<SearchMatch[]>([]);
  const [currentMatch, setCurrentMatch] = useState(0);

  useEffect(() => {
    searchQueryRef.current = searchQuery;
  }, [searchQuery]);

  // 加载文档 + 目录大纲；base64 变化（切换文件）时整体重置。
  useEffect(() => {
    let cancelled = false;
    setError(null);
    setPageCount(0);
    setOutline(null);
    setOutlineOpen(false);
    setZoom(1);
    setSearchOpen(false);
    setSearchQuery("");
    setMatches([]);
    setCurrentMatch(0);

    const binary = atob(base64);
    const bytes = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);

    const loadingTask = pdfjsLib.getDocument({ data: bytes });
    loadingTask.promise.then(
      async (pdf) => {
        if (cancelled) return;
        pdfRef.current = pdf;
        setPageCount(pdf.numPages);
        try {
          const nodes = normalizeOutline(await pdf.getOutline());
          if (!cancelled) setOutline(nodes.length ? nodes : null);
        } catch {
          if (!cancelled) setOutline(null);
        }
      },
      (e) => {
        if (cancelled) return;
        setError(e instanceof Error ? e.message : String(e));
      },
    );

    return () => {
      cancelled = true;
      pdfRef.current = null;
      loadingTask.destroy();
    };
  }, [base64]);

  const runSearch = useCallback(() => {
    const query = searchQueryRef.current.trim().toLowerCase();
    if (!query) {
      setMatches([]);
      setCurrentMatch(0);
      return;
    }
    const found: SearchMatch[] = [];
    for (const [pageNum, idx] of textIndexRef.current) {
      const lower = idx.text.toLowerCase();
      let from = 0;
      for (;;) {
        const at = lower.indexOf(query, from);
        if (at === -1) break;
        const end = at + query.length;
        const divs = idx.divs.filter((_, i) => {
          const [s, e] = idx.ranges[i];
          return s < end && e > at;
        });
        if (divs.length) found.push({ pageNum, divs });
        from = at + 1;
      }
    }
    found.sort((a, b) => a.pageNum - b.pageNum);
    setMatches(found);
    setCurrentMatch(0);
  }, []);

  // 渲染每一页画布 + 文字层；文档或缩放变化时整页重绘。
  useEffect(() => {
    if (pageCount === 0) return;
    let cancelled = false;
    const renderTasks: RenderTask[] = [];
    const textLayers: PdfTextLayer[] = [];

    (async () => {
      const pdf = pdfRef.current;
      const container = pagesContainerRef.current;
      if (!pdf || !container) return;
      container.innerHTML = "";
      pageWrapperRefs.current.clear();
      textIndexRef.current.clear();
      highlightedRef.current = [];

      const targetWidth = (scrollRef.current?.clientWidth || 800) - 32;

      for (let pageNum = 1; pageNum <= pdf.numPages; pageNum++) {
        if (cancelled) return;
        const page = await pdf.getPage(pageNum);
        const unscaledViewport = page.getViewport({ scale: 1 });
        if (pageNum === 1) {
          baseScaleRef.current = targetWidth / unscaledViewport.width;
        }
        const viewport = page.getViewport({ scale: baseScaleRef.current * zoom });

        const wrapper = document.createElement("div");
        wrapper.className = "pdf-page-wrapper";
        wrapper.style.width = `${viewport.width}px`;
        wrapper.style.height = `${viewport.height}px`;
        container.appendChild(wrapper);
        pageWrapperRefs.current.set(pageNum, wrapper);

        const canvas = document.createElement("canvas");
        canvas.width = viewport.width;
        canvas.height = viewport.height;
        canvas.style.display = "block";
        wrapper.appendChild(canvas);

        const ctx = canvas.getContext("2d");
        if (ctx) {
          const task = page.render({ canvasContext: ctx, viewport, canvas });
          renderTasks.push(task);
          await task.promise.catch(() => {});
        }
        if (cancelled) return;

        const textLayerDiv = document.createElement("div");
        textLayerDiv.className = "textLayer";
        textLayerDiv.style.width = `${viewport.width}px`;
        textLayerDiv.style.height = `${viewport.height}px`;
        textLayerDiv.style.setProperty("--total-scale-factor", "1");
        wrapper.appendChild(textLayerDiv);

        try {
          const textContent = await page.getTextContent();
          const textLayer = new pdfjsLib.TextLayer({ textContentSource: textContent, container: textLayerDiv, viewport });
          textLayers.push(textLayer);
          await textLayer.render();
          if (cancelled) return;
          const divs = textLayer.textDivs;
          const strs = textLayer.textContentItemsStr;
          let text = "";
          const ranges: [number, number][] = [];
          for (let i = 0; i < divs.length; i++) {
            const s = strs[i] ?? "";
            ranges.push([text.length, text.length + s.length]);
            text += s;
          }
          textIndexRef.current.set(pageNum, { divs, ranges, text });
        } catch {
          // 文字层构建失败不影响画面预览，只是这一页搜不到文字。
        }
      }

      if (!cancelled) runSearch();
    })();

    return () => {
      cancelled = true;
      renderTasks.forEach((t) => t.cancel());
      textLayers.forEach((t) => t.cancel());
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pageCount, zoom]);

  useEffect(() => {
    const t = setTimeout(runSearch, 150);
    return () => clearTimeout(t);
  }, [searchQuery, runSearch]);

  useEffect(() => {
    highlightedRef.current.forEach((el) => el.classList.remove("pdf-search-hit", "pdf-search-hit-active"));
    const highlighted: HTMLElement[] = [];
    matches.forEach((m, i) => {
      m.divs.forEach((d) => {
        d.classList.add("pdf-search-hit");
        if (i === currentMatch) d.classList.add("pdf-search-hit-active");
        highlighted.push(d);
      });
    });
    highlightedRef.current = highlighted;
    matches[currentMatch]?.divs[0]?.scrollIntoView({ block: "center", behavior: "smooth" });
  }, [matches, currentMatch]);

  const goToMatch = (delta: number) => {
    setCurrentMatch((c) => {
      if (matches.length === 0) return 0;
      return (c + delta + matches.length) % matches.length;
    });
  };

  const goToOutlineDest = useCallback(async (dest: OutlineEntry["dest"]) => {
    const pdf = pdfRef.current;
    if (!pdf || !dest) return;
    try {
      const explicit = typeof dest === "string" ? await pdf.getDestination(dest) : dest;
      if (!Array.isArray(explicit) || explicit.length === 0) return;
      const ref = explicit[0];
      const pageIndex = typeof ref === "number" ? ref : await pdf.getPageIndex(ref);
      pageWrapperRefs.current.get(pageIndex + 1)?.scrollIntoView({ block: "start", behavior: "smooth" });
    } catch {
      // 目录跳转失败静默忽略，不影响继续查看 PDF。
    }
  }, []);

  const closeSearch = () => {
    setSearchOpen(false);
    setSearchQuery("");
  };

  if (error) {
    return <div style={{ padding: 16, fontSize: 12, color: "var(--danger)" }}>PDF 解析失败：{error}</div>;
  }

  return (
    <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
      <div className="pdf-toolbar">
        <button
          className={`btn ghost sm ${outlineOpen ? "active" : ""}`}
          disabled={false}
          onClick={() => setOutlineOpen((v) => !v)}
          title={outline ? "目录大纲" : "此文档没有目录"}
        >
          <List style={{ width: 14, height: 14 }} /> 目录
        </button>
        <div className="pdf-toolbar-sep" />
        <button className="btn ghost sm" onClick={() => setZoom((z) => Math.max(MIN_ZOOM, z / ZOOM_STEP))} title="缩小">
          <ZoomOut style={{ width: 14, height: 14 }} />
        </button>
        <span className="pdf-zoom-label" onClick={() => setZoom(1)} title="重置为适应宽度">
          {Math.round(zoom * 100)}%
        </span>
        <button className="btn ghost sm" onClick={() => setZoom((z) => Math.min(MAX_ZOOM, z * ZOOM_STEP))} title="放大">
          <ZoomIn style={{ width: 14, height: 14 }} />
        </button>
        <div className="pdf-toolbar-sep" />
        {searchOpen ? (
          <div className="pdf-search-box">
            <input
              autoFocus
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") goToMatch(e.shiftKey ? -1 : 1);
                if (e.key === "Escape") closeSearch();
              }}
              placeholder="搜索文本…"
            />
            <span className="pdf-search-count">{searchQuery ? `${matches.length ? currentMatch + 1 : 0}/${matches.length}` : ""}</span>
            <button className="btn ghost sm" onClick={() => goToMatch(-1)} disabled={!matches.length} title="上一个 (Shift+Enter)">
              <ChevronUp style={{ width: 14, height: 14 }} />
            </button>
            <button className="btn ghost sm" onClick={() => goToMatch(1)} disabled={!matches.length} title="下一个 (Enter)">
              <ChevronDown style={{ width: 14, height: 14 }} />
            </button>
            <button className="btn ghost sm" onClick={closeSearch} title="关闭搜索 (Esc)">
              <X style={{ width: 14, height: 14 }} />
            </button>
          </div>
        ) : (
          <button className="btn ghost sm" onClick={() => setSearchOpen(true)} title="搜索">
            <Search style={{ width: 14, height: 14 }} /> 搜索
          </button>
        )}
      </div>
      <div style={{ flex: 1, minHeight: 0, display: "flex", overflow: "hidden" }}>
        {outlineOpen && (
          <div className="pdf-outline-panel">
            {outline?.length ? <ul className="pdf-outline-list pdf-outline-root">
              {outline.map((item, i) => <OutlineNode key={i} item={item} depth={0} onNavigate={goToOutlineDest} />)}
            </ul> : <div style={{ padding: 12, color: "var(--text-secondary)", fontSize: 12 }}>此 PDF 没有内置大纲</div>}
          </div>
        )}
        <div ref={scrollRef} style={{ flex: 1, minHeight: 0, overflow: "auto", background: "var(--bg-base)" }}>
          {/* pagesContainerRef 这个节点完全交给渲染 effect 用 innerHTML/appendChild 命令式
              管理——不能让 React 也往里塞子节点（之前"加载中…"和 <canvas> 共享同一个
              容器，effect 里的 container.innerHTML = "" 会把 React 渲染的"加载中" 节点
              从 DOM 里删掉而 React 自己不知道；等 setPageCount 触发重新渲染、React 想把
              它认为还在的那个节点摘掉时，节点已经不在了，抛
              "Failed to execute 'removeChild'...: The node to be removed is not a child
              of this node"）。"加载中" 提示挪到外层这个纯 React 管理的兄弟节点里，两者
              互不干扰。 */}
          {pageCount === 0 && <div style={{ padding: 16, fontSize: 12, color: "var(--text-secondary)" }}>加载中…</div>}
          <div ref={pagesContainerRef} style={{ padding: 16 }} />
        </div>
      </div>
    </div>
  );
};
