import React, { useEffect } from "react";
import { ChevronDown, ChevronRight, FileSearch, Loader2, Replace, ReplaceAll, Search as SearchIcon, X } from "lucide-react";
import { useSearchStore } from "../../stores/searchStore";
import type { PendingHighlight } from "../../stores/editorStore";
import type { SearchMode, SearchOptions } from "../../types/bindings";

interface SearchPanelProps {
  workspaceId: string;
  onOpenResult: (path: string, line: number, highlights: PendingHighlight[]) => void;
}

function fileName(path: string): string {
  return path.split(/[/\\]/).filter(Boolean).pop() ?? path;
}

/** 目录部分（不含文件名），同名文件在不同目录下时用来区分——之前只显示裸文件名，
 * 一个 Lua 项目里好几个目录各自有一份 init.lua 时看着像"同一个文件重复了 4 次"，
 * 实际是 4 个不同的文件（真实用户反馈）。 */
function dirName(path: string): string {
  const idx = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  return idx >= 0 ? path.slice(0, idx) : "";
}

/** 按字符下标高亮命中片段（后端返回的是字符下标，不是字节下标，见 fsops::SearchMatch 注释）。*/
function highlightLine(text: string, start: number, end: number): React.ReactNode {
  const chars = Array.from(text);
  const before = chars.slice(0, start).join("");
  const hit = chars.slice(start, end).join("");
  const after = chars.slice(end).join("");
  return (
    <>
      {before}
      <mark className="search-match-mark">{hit}</mark>
      {after}
    </>
  );
}

/**
 * 左侧目录树的全文搜索面板（参考 VS Code 全局搜索，2026-08-18 需求）：
 * "浏览器输入地址后不应该是弹框"那次一样，是在同一块 UI 区域内完成，不是弹窗；
 * 这里替换的是侧边栏内容本身（和 Explorer 树互斥切换，见 App.tsx 的 sidebarMode）。
 *
 * 结果是流式渲染的——`results` 由 `registerSearchListeners` 在事件到达时逐条
 * 写进 store，这个组件只是订阅 store 状态，不需要额外做"边搜边展示"的特殊处理，
 * 搜索还没跑完时 `results` 已经有的部分会先显示出来（2026-08-18 用户反馈"这个
 * 搜索功能太慢了...能否一个一个目录搜，搜到一部分先展示一部分"）。
 */
export const SearchPanel: React.FC<SearchPanelProps> = ({ workspaceId, onOpenResult }) => {
  const {
    query,
    replacement,
    replaceOpen,
    mode,
    options,
    scopePath,
    scopeLabel,
    results,
    truncated,
    loading,
    replacing,
    error,
    collapsed,
    setQuery,
    setReplacement,
    setReplaceOpen,
    setMode,
    setOption,
    setScope,
    toggleCollapsed,
    runSearch,
    replaceAll,
    replaceInFile,
    clear,
  } = useSearchStore();

  useEffect(() => {
    return () => clear();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const totalMatches = results.reduce((n, f) => n + f.matches.length, 0);

  const toggleOption = (key: keyof SearchOptions) => setOption(key, !options[key]);

  const optionBtn = (key: keyof SearchOptions, label: string, title: string) => (
    <button
      className={`search-opt-btn ${options[key] ? "active" : ""}`}
      title={title}
      onClick={() => toggleOption(key)}
    >
      {label}
    </button>
  );

  /** 内容搜索模式下把这个文件命中的全部位置转成编辑器要的高亮结构；文件名搜索模式
   * 下 match 的字符下标指的是文件名字符串，不是文件内容里的位置，不能拿去点亮
   * 编辑器正文——那种模式下只负责打开文件，不做高亮。 */
  const highlightsFor = (file: (typeof results)[number]): PendingHighlight[] =>
    mode === "content" ? file.matches.map((m) => ({ line: m.line_number, start: m.match_start, end: m.match_end })) : [];

  const openFile = (file: (typeof results)[number], line: number) => onOpenResult(file.path, line, highlightsFor(file));

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      <div style={{ padding: "8px 8px 4px", display: "flex", flexDirection: "column", gap: 4 }}>
        {scopePath && (
          <div
            style={{
              display: "flex",
              alignItems: "center",
              gap: 6,
              fontSize: 11,
              color: "var(--text-secondary)",
              background: "var(--bg-selected)",
              borderRadius: "var(--radius-sm)",
              padding: "2px 6px",
              width: "fit-content",
            }}
          >
            <FileSearch style={{ width: 12, height: 12 }} />
            <span>仅搜索：{scopeLabel ?? scopePath}</span>
            <button
              className="btn ghost sm"
              title="改为搜索整个工作区"
              style={{ padding: 0, width: 16, height: 16 }}
              onClick={() => setScope(null, null)}
            >
              <X style={{ width: 11, height: 11 }} />
            </button>
          </div>
        )}

        <div style={{ display: "flex", alignItems: "center", gap: 4 }}>
          <button
            className="btn ghost sm"
            title={replaceOpen ? "收起替换" : "展开替换"}
            onClick={() => setReplaceOpen(!replaceOpen)}
            style={{ padding: 2 }}
          >
            {replaceOpen ? <ChevronDown style={{ width: 14, height: 14 }} /> : <ChevronRight style={{ width: 14, height: 14 }} />}
          </button>
          <div style={{ position: "relative", flex: 1 }}>
            <input
              className="form-input"
              style={{ width: "100%", height: 26, paddingRight: mode === "content" ? 76 : 4 }}
              placeholder={mode === "content" ? "搜索文件内容" : "搜索文件名"}
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && runSearch(workspaceId)}
            />
            {mode === "content" && (
              <div style={{ position: "absolute", right: 3, top: 3, display: "flex", gap: 2 }}>
                {optionBtn("case_sensitive", "Aa", "区分大小写")}
                {optionBtn("whole_word", "ab", "全字匹配")}
                {optionBtn("use_regex", ".*", "使用正则表达式")}
              </div>
            )}
          </div>
        </div>

        <div style={{ display: "flex", gap: 4, paddingLeft: 22 }}>
          {(
            [
              ["content", "内容"],
              ["file_name", "文件名"],
            ] as [SearchMode, string][]
          ).map(([m, label]) => (
            <button key={m} className={`search-opt-btn ${mode === m ? "active" : ""}`} style={{ width: "auto", padding: "0 8px" }} onClick={() => setMode(m)}>
              {label}
            </button>
          ))}
        </div>

        {replaceOpen && mode === "content" && (
          <div style={{ display: "flex", alignItems: "center", gap: 4, paddingLeft: 22 }}>
            <input
              className="form-input"
              style={{ flex: 1, height: 26 }}
              placeholder="替换"
              value={replacement}
              onChange={(e) => setReplacement(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && results.length > 0 && replaceAll(workspaceId)}
            />
            <button
              className="btn ghost sm"
              title="全部替换"
              disabled={results.length === 0 || replacing}
              onClick={() => replaceAll(workspaceId)}
            >
              <ReplaceAll style={{ width: 14, height: 14 }} />
            </button>
          </div>
        )}
      </div>

      <div style={{ padding: "0 12px 6px" }}>
        <button className="btn primary sm" onClick={() => runSearch(workspaceId)} disabled={loading || !query.trim()}>
          {loading ? <Loader2 style={{ width: 14, height: 14 }} className="spin" /> : <SearchIcon style={{ width: 14, height: 14 }} />}
          {loading ? "搜索中…" : "搜索"}
        </button>
      </div>

      {error && <div style={{ padding: "0 12px 6px", fontSize: 12, color: "var(--danger)" }}>{error}</div>}

      {results.length > 0 && (
        <div style={{ padding: "0 12px 4px", fontSize: 11, color: "var(--text-secondary)" }}>
          {results.length} 个文件，{totalMatches} 处匹配{loading && "（搜索中…）"}
          {truncated && "（结果过多，只展示部分，建议缩小搜索范围或限定子目录）"}
        </div>
      )}

      <div style={{ flex: 1, overflowY: "auto" }}>
        {results.length === 0 ? (
          <div style={{ padding: 24, textAlign: "center", fontSize: 12, color: "var(--text-secondary)" }}>
            {loading ? "搜索中…" : "输入关键词后回车搜索；右键 Explorer 里的文件夹可以限定只搜那一个子目录"}
          </div>
        ) : (
          results.map((file) => {
            const isCollapsed = collapsed.has(file.path);
            const firstLine = file.matches[0]?.line_number ?? 1;
            return (
              <div key={file.path}>
                <div className="file-row" style={{ cursor: "pointer", gridTemplateColumns: "auto 1fr auto auto" }}>
                  <span
                    onClick={(e) => {
                      e.stopPropagation();
                      toggleCollapsed(file.path);
                    }}
                    style={{ display: "flex", alignItems: "center" }}
                  >
                    {isCollapsed ? <ChevronRight style={{ width: 12, height: 12 }} /> : <ChevronDown style={{ width: 12, height: 12 }} />}
                  </span>
                  {/* 点文件名这一行整体：直接打开文件，命中的全部位置一起点亮，跳到第一处
                      （2026-08-18 用户原话："打开后全部点亮就行"），不需要先展开再挑一行点。*/}
                  <span
                    style={{ display: "flex", flexDirection: "column", overflow: "hidden", cursor: "pointer" }}
                    title={file.path}
                    onClick={() => openFile(file, firstLine)}
                  >
                    <span className="file-name">{fileName(file.path)}</span>
                    {dirName(file.path) && (
                      <span style={{ fontSize: 10, color: "var(--text-secondary)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                        {dirName(file.path)}
                      </span>
                    )}
                  </span>
                  <span style={{ fontSize: 11, color: "var(--text-secondary)" }}>{file.matches.length}</span>
                  {replaceOpen && mode === "content" && (
                    <button
                      className="btn ghost sm"
                      title="替换此文件中的全部匹配"
                      disabled={replacing}
                      onClick={(e) => {
                        e.stopPropagation();
                        replaceInFile(workspaceId, file.path);
                      }}
                    >
                      <Replace style={{ width: 12, height: 12 }} />
                    </button>
                  )}
                </div>
                {!isCollapsed &&
                  mode === "content" &&
                  file.matches.map((m, i) => (
                    <div
                      key={i}
                      className="file-row"
                      style={{ cursor: "pointer", paddingLeft: 28, gridTemplateColumns: "auto 1fr" }}
                      onClick={() => openFile(file, m.line_number)}
                    >
                      <span style={{ fontSize: 11, color: "var(--text-secondary)" }}>{m.line_number}</span>
                      <span
                        style={{
                          fontSize: 12,
                          fontFamily: "var(--font-mono)",
                          whiteSpace: "pre",
                          overflow: "hidden",
                          textOverflow: "ellipsis",
                        }}
                      >
                        {highlightLine(m.line_text, m.match_start, m.match_end)}
                      </span>
                    </div>
                  ))}
              </div>
            );
          })
        )}
      </div>
    </div>
  );
};
