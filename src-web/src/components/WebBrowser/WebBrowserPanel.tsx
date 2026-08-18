import React, { useCallback, useEffect, useRef, useState } from "react";
import { ArrowLeft, ExternalLink, Globe, Trash2, X } from "lucide-react";
import { useBrowserStore } from "../../stores/browserStore";
import { useToastStore } from "../shared/Toast";
import { formatError } from "../../utils/error";
import { browserService, type PanelBounds } from "../../services/browserService";

function formatTime(iso: string): string {
  const d = new Date(iso);
  return `${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")} ${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}

interface WebBrowserPanelProps {
  /** 对应 App.tsx 的 activeView === "browser"。子 WebView 是操作系统级别的原生
   * 视图，不受这个组件的 CSS display 影响，必须显式调用 browser_hide/browser_show
   * 才能让它跟着 Tab 切换一起隐藏/出现（见 browser 模块顶部注释）。 */
  visible: boolean;
}

/**
 * 网页浏览面板（DESIGN.md §3.5）：地址栏 + 内嵌网页 + 历史记录。
 *
 * 2026-08-18 从"独立弹窗"改成内嵌（用户原话："浏览器输入地址后不应该是弹框，
 * 而是默认在TAB下打开，和编辑器一样"）：网页内容不是这个组件直接渲染出来的
 * （没有 <iframe>，理由见后端 browser 模块注释），而是后端在主窗口内叠加的一个
 * 原生子 WebView，位置/大小跟随 `viewportRef` 这个占位 div 的屏幕矩形。这个 div
 * 本身永远挂载、永远和"网页应该显示的区域"同尺寸——即使当前没有打开任何网页，
 * 也让它保持可测量，用来解决"第一次打开网页时，测量的矩形还不能依赖 pageOpen
 * 状态"这个先有鸡还是先有蛋的问题；没有网页时上面盖一层历史记录列表遮住它。
 */
export const WebBrowserPanel: React.FC<WebBrowserPanelProps> = ({ visible }) => {
  const { history, loading, error, loadHistory, removeEntry, clearHistory } = useBrowserStore();
  const push = useToastStore((s) => s.push);
  const [input, setInput] = useState("");
  const [pageOpen, setPageOpen] = useState(false);
  const [currentUrl, setCurrentUrl] = useState("");
  const [opening, setOpening] = useState(false);
  const viewportRef = useRef<HTMLDivElement | null>(null);
  const pageOpenRef = useRef(pageOpen);
  pageOpenRef.current = pageOpen;

  useEffect(() => {
    loadHistory();
    return () => {
      // 面板彻底卸载（回到工作区选择页）时清理子 WebView，避免留下孤儿原生视图。
      browserService.close().catch(() => {});
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const readBounds = useCallback((): PanelBounds | null => {
    const el = viewportRef.current;
    if (!el) return null;
    const rect = el.getBoundingClientRect();
    if (rect.width <= 0 || rect.height <= 0) return null;
    return { x: rect.left, y: rect.top, width: rect.width, height: rect.height };
  }, []);

  useEffect(() => {
    if (visible) {
      if (pageOpenRef.current) {
        const bounds = readBounds();
        if (bounds) browserService.show(bounds).catch(() => {});
      }
    } else {
      browserService.hide().catch(() => {});
    }
  }, [visible, readBounds]);

  useEffect(() => {
    if (!visible || !pageOpen) return;
    const el = viewportRef.current;
    if (!el) return;
    let raf = 0;
    const sync = () => {
      cancelAnimationFrame(raf);
      raf = requestAnimationFrame(() => {
        const bounds = readBounds();
        if (bounds) browserService.setBounds(bounds).catch(() => {});
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
  }, [visible, pageOpen, readBounds]);

  const handleOpen = async (url: string) => {
    const bounds = readBounds();
    if (!bounds) {
      push("error", "浏览器面板尚未就绪，请稍后重试");
      return;
    }
    setOpening(true);
    try {
      const normalized = await browserService.open(url, bounds);
      setCurrentUrl(normalized);
      setPageOpen(true);
      await loadHistory();
    } catch (e) {
      push("error", `打开失败：${formatError(e)}`);
    } finally {
      setOpening(false);
    }
  };

  const handleBack = () => {
    browserService.hide().catch(() => {});
    setPageOpen(false);
  };

  const addressValue = pageOpen ? currentUrl : input;
  const setAddressValue = pageOpen ? setCurrentUrl : setInput;
  const submit = () => {
    const value = addressValue.trim();
    if (!value) return;
    handleOpen(value);
    if (!pageOpen) setInput("");
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      <div className="editor-toolbar" style={{ gap: 8 }}>
        {pageOpen && (
          <button className="btn ghost sm" title="返回历史记录" onClick={handleBack}>
            <ArrowLeft style={{ width: 14, height: 14 }} />
          </button>
        )}
        <Globe style={{ width: 14, height: 14, color: "var(--text-secondary)", flexShrink: 0 }} />
        <input
          className="form-input"
          style={{ flex: 1, height: 26 }}
          placeholder="输入网址或搜索内容，回车打开"
          value={addressValue}
          onChange={(e) => setAddressValue(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") submit();
          }}
        />
        <button className="btn primary sm" disabled={!addressValue.trim() || opening} onClick={submit}>
          {opening ? "打开中…" : pageOpen ? "跳转" : "打开"}
        </button>
      </div>

      {!pageOpen && (
        <div style={{ padding: "6px 12px", fontSize: 11, color: "var(--text-secondary)" }}>
          网页内嵌显示在本面板内（独立 WebView 承载，不与应用共享权限），下面是访问历史
        </div>
      )}
      {error && <div style={{ padding: "0 12px 6px", fontSize: 12, color: "var(--danger)" }}>{error}</div>}

      <div style={{ flex: 1, position: "relative", minHeight: 0 }}>
        {/* 网页由后端摆放的原生子 WebView 渲染在这块区域的屏幕坐标上，这个 div 本身
            不显示任何内容——只用来量测/占位（ResizeObserver 读它的 rect），必须
            始终挂载，不能用 display:none 隐藏，否则测出来的尺寸永远是 0。*/}
        <div ref={viewportRef} style={{ position: "absolute", inset: 0 }} />
        {!pageOpen && (
          <div style={{ position: "absolute", inset: 0, overflowY: "auto", background: "var(--bg-surface)" }}>
            <div style={{ display: "flex", alignItems: "center", padding: "4px 12px", borderBottom: "1px solid var(--border-subtle)" }}>
              <span style={{ fontSize: 12, color: "var(--text-secondary)" }}>历史记录</span>
              {history.length > 0 && (
                <button className="btn ghost sm" style={{ marginLeft: "auto" }} onClick={() => clearHistory()}>
                  <Trash2 style={{ width: 12, height: 12 }} /> 清空
                </button>
              )}
            </div>
            {loading ? (
              <div style={{ padding: 16, fontSize: 12, color: "var(--text-secondary)" }}>加载中…</div>
            ) : history.length === 0 ? (
              <div style={{ padding: 16, fontSize: 12, color: "var(--text-secondary)" }}>还没有访问记录，在上面输入网址试试</div>
            ) : (
              history.map((entry) => (
                <div
                  key={entry.id}
                  className="file-row"
                  style={{ gridTemplateColumns: "1fr auto auto", cursor: "pointer" }}
                  onClick={() => handleOpen(entry.url)}
                >
                  <span style={{ display: "flex", alignItems: "center", gap: 6, overflow: "hidden" }}>
                    <ExternalLink style={{ width: 13, height: 13, flexShrink: 0, color: "var(--text-secondary)" }} />
                    <span className="file-name">{entry.title ?? entry.url}</span>
                  </span>
                  <span style={{ fontSize: 11, color: "var(--text-secondary)" }}>{formatTime(entry.visited_at)}</span>
                  <button
                    className="btn ghost sm"
                    title="删除这条记录"
                    onClick={(e) => {
                      e.stopPropagation();
                      removeEntry(entry.id);
                    }}
                  >
                    <X style={{ width: 12, height: 12 }} />
                  </button>
                </div>
              ))
            )}
          </div>
        )}
      </div>
    </div>
  );
};
