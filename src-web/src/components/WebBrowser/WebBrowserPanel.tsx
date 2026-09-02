import React, { useCallback, useEffect, useRef, useState } from "react";
import { ExternalLink, Globe, Plus, Trash2, X } from "lucide-react";
import { useBrowserStore } from "../../stores/browserStore";
import { useModalStackStore } from "../../stores/modalStackStore";
import { useToastStore } from "../shared/Toast";
import { formatError } from "../../utils/error";
import { browserService, type PanelBounds } from "../../services/browserService";

function formatTime(iso: string): string {
  const d = new Date(iso);
  return `${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")} ${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}

interface BrowserTab {
  id: string;
  /** null = 还没打开任何网页的空白标签页（显示地址栏 + 历史记录）。 */
  url: string | null;
  title: string;
}

function newTab(): BrowserTab {
  return { id: crypto.randomUUID(), url: null, title: "新标签页" };
}

interface WebBrowserPanelProps {
  /** 对应 App.tsx 的 activeView === "browser"。子 WebView 是操作系统级别的原生
   * 视图，不受这个组件的 CSS display 影响，必须显式调用 browser_hide/browser_show
   * 才能让它跟着 Tab 切换一起隐藏/出现（见 browser 模块顶部注释）。 */
  visible: boolean;
}

/**
 * 网页浏览面板（DESIGN.md §3.5）：多标签页 + 地址栏 + 内嵌网页 + 历史记录。
 *
 * 2026-08-18 两轮需求变更：先是从"独立弹窗"改成内嵌（"浏览器输入地址后不应该是
 * 弹框，而是默认在TAB下打开，和编辑器一样"），随后又要求"支持鼠标缩放，支持加TAB
 * 打开别的页面"——缩放是后端 `zoom_hotkeys_enabled(true)` 一行的事（WebView2 默认
 * 关着），多标签页则是这个组件的核心改动：每个标签页对应后端一个独立的子 WebView
 * （`tabs` 状态 + `activeTabId`），同一时刻只有当前激活标签页对应的子 WebView 是
 * `show()` 状态，其余全部 `hide()`——原生子 WebView 不参与 CSS 层叠，必须这样显式
 * 管理可见性，不能只是把 DOM 元素切走了事。
 */
export const WebBrowserPanel: React.FC<WebBrowserPanelProps> = ({ visible }) => {
  const { history, loading, error, loadHistory, removeEntry, clearHistory } = useBrowserStore();
  const push = useToastStore((s) => s.push);
  // 任何 ConfirmDialog 系弹窗打开时，即使浏览器面板本身仍是激活 Tab，也要临时把
  // 原生子 WebView 隐藏掉，否则弹窗会被它盖住（见 modalStackStore 注释）。
  const hasOpenModal = useModalStackStore((s) => s.count > 0);
  const effectiveVisible = visible && !hasOpenModal;
  const [tabs, setTabs] = useState<BrowserTab[]>(() => [newTab()]);
  const [activeTabId, setActiveTabId] = useState(() => tabs[0].id);
  const [input, setInput] = useState("");
  const [opening, setOpening] = useState(false);
  const viewportRef = useRef<HTMLDivElement | null>(null);
  const prevActiveRef = useRef<string | null>(null);

  const activeTab = tabs.find((t) => t.id === activeTabId) ?? null;
  const activeTabRef = useRef(activeTab);
  activeTabRef.current = activeTab;

  useEffect(() => {
    loadHistory();
    return () => {
      // 面板彻底卸载（回到工作区选择页）时清理全部子 WebView，避免留下孤儿原生视图。
      browserService.closeAll().catch(() => {});
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 切换激活标签页时，地址栏内容跟着切到那个标签页自己的地址（不是全局共享一个输入框）。
  useEffect(() => {
    setInput(activeTab?.url ?? "");
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeTabId]);

  // 没有任何标签页时自动补一个空白标签页，面板不会出现"关光了就什么都不能操作"的死角。
  useEffect(() => {
    if (tabs.length === 0) {
      const t = newTab();
      setTabs([t]);
      setActiveTabId(t.id);
    }
  }, [tabs.length]);

  const readBounds = useCallback((): PanelBounds | null => {
    const el = viewportRef.current;
    if (!el) return null;
    const rect = el.getBoundingClientRect();
    if (rect.width <= 0 || rect.height <= 0) return null;
    return { x: rect.left, y: rect.top, width: rect.width, height: rect.height };
  }, []);

  // 面板整体（顶层"网页浏览"Tab）可见性变化：显示/隐藏当前激活标签页对应的子 WebView。
  // 弹窗打开导致的临时隐藏也走这里（effectiveVisible 叠加了 hasOpenModal）。
  useEffect(() => {
    const tab = activeTabRef.current;
    if (!tab?.url) return;
    if (effectiveVisible) {
      const bounds = readBounds();
      if (bounds) browserService.show(tab.id, bounds).catch(() => {});
    } else {
      browserService.hide(tab.id).catch(() => {});
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [effectiveVisible]);

  // 面板内部切换标签页：隐藏旧标签页的子 WebView，显示新标签页的（如果它已经打开过网页）。
  useEffect(() => {
    const prev = prevActiveRef.current;
    prevActiveRef.current = activeTabId;
    if (prev && prev !== activeTabId) {
      browserService.hide(prev).catch(() => {});
    }
    if (effectiveVisible && activeTab?.url) {
      const bounds = readBounds();
      if (bounds) browserService.show(activeTabId, bounds).catch(() => {});
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeTabId]);

  // 面板尺寸变化（侧边栏拖拽调宽、窗口缩放、终端面板展开收起）时同步当前激活标签页的
  // 子 WebView 位置/大小。
  useEffect(() => {
    if (!effectiveVisible || !activeTab?.url) return;
    const el = viewportRef.current;
    if (!el) return;
    let raf = 0;
    const sync = () => {
      cancelAnimationFrame(raf);
      raf = requestAnimationFrame(() => {
        const bounds = readBounds();
        if (bounds) browserService.setBounds(activeTabId, bounds).catch(() => {});
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
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [effectiveVisible, activeTabId, activeTab?.url]);

  const handleOpen = async (url: string) => {
    const tab = activeTabRef.current;
    if (!tab) return;
    const bounds = readBounds();
    if (!bounds) {
      push("error", "浏览器面板尚未就绪，请稍后重试");
      return;
    }
    setOpening(true);
    try {
      const normalized = await browserService.open(tab.id, url, bounds);
      setTabs((ts) => ts.map((t) => (t.id === tab.id ? { ...t, url: normalized, title: normalized } : t)));
      await loadHistory();
    } catch (e) {
      push("error", `打开失败：${formatError(e)}`);
    } finally {
      setOpening(false);
    }
  };

  const addTab = () => {
    const t = newTab();
    setTabs((ts) => [...ts, t]);
    setActiveTabId(t.id);
  };

  const closeTab = (id: string) => {
    browserService.close(id).catch(() => {});
    setTabs((ts) => {
      const idx = ts.findIndex((t) => t.id === id);
      const next = ts.filter((t) => t.id !== id);
      if (id === activeTabId && next.length > 0) {
        const neighbor = next[idx] ?? next[idx - 1];
        setActiveTabId(neighbor.id);
      }
      return next;
    });
  };

  const submit = () => {
    const value = input.trim();
    if (!value) return;
    handleOpen(value);
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      <div className="editor-tabs">
        {tabs.map((tab) => (
          <div
            key={tab.id}
            className={`editor-tab ${tab.id === activeTabId ? "active" : ""}`}
            title={tab.url ?? tab.title}
            onClick={() => setActiveTabId(tab.id)}
          >
            <span className="editor-tab-name">{tab.title}</span>
            <button
              className="editor-tab-close"
              title="关闭标签页"
              onClick={(e) => {
                e.stopPropagation();
                closeTab(tab.id);
              }}
            >
              <X />
            </button>
          </div>
        ))}
        <button className="btn ghost sm" title="新建标签页" onClick={addTab} style={{ margin: "0 4px" }}>
          <Plus style={{ width: 14, height: 14 }} />
        </button>
      </div>

      <div className="editor-toolbar" style={{ gap: 8 }}>
        <Globe style={{ width: 14, height: 14, color: "var(--text-secondary)", flexShrink: 0 }} />
        <input
          className="form-input"
          style={{ flex: 1, height: 26 }}
          placeholder="输入网址或搜索内容，回车打开"
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") submit();
          }}
        />
        <button className="btn primary sm" disabled={!input.trim() || opening} onClick={submit}>
          {opening ? "打开中…" : activeTab?.url ? "跳转" : "打开"}
        </button>
      </div>

      {!activeTab?.url && (
        <div style={{ padding: "6px 12px", fontSize: 11, color: "var(--text-secondary)" }}>
          网页内嵌显示在本面板内（独立 WebView 承载，不与应用共享权限；支持 Ctrl+滚轮缩放），下面是访问历史
        </div>
      )}
      {error && <div style={{ padding: "0 12px 6px", fontSize: 12, color: "var(--danger)" }}>{error}</div>}

      <div style={{ flex: 1, position: "relative", minHeight: 0 }}>
        {/* 网页由后端摆放的原生子 WebView 渲染在这块区域的屏幕坐标上，这个 div 本身
            不显示任何内容——只用来量测/占位（ResizeObserver 读它的 rect），必须
            始终挂载，不能用 display:none 隐藏，否则测出来的尺寸永远是 0。*/}
        <div ref={viewportRef} style={{ position: "absolute", inset: 0 }} />
        {!activeTab?.url && (
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
