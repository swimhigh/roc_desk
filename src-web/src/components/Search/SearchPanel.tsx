import React, { useEffect } from "react";
import { ChevronDown, ChevronRight, Loader2, Replace, ReplaceAll, Search as SearchIcon } from "lucide-react";
import { useSearchStore } from "../../stores/searchStore";
import type { SearchOptions } from "../../types/bindings";

interface SearchPanelProps {
  workspaceId: string;
  onOpenResult: (path: string, line: number) => void;
}

function fileName(path: string): string {
  return path.split(/[/\\]/).filter(Boolean).pop() ?? path;
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
 */
export const SearchPanel: React.FC<SearchPanelProps> = ({ workspaceId, onOpenResult }) => {
  const {
    query,
    replacement,
    replaceOpen,
    options,
    results,
    truncated,
    loading,
    replacing,
    error,
    collapsed,
    setQuery,
    setReplacement,
    setReplaceOpen,
    setOption,
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

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      <div style={{ padding: "8px 8px 4px", display: "flex", flexDirection: "column", gap: 4 }}>
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
              style={{ width: "100%", height: 26, paddingRight: 76 }}
              placeholder="搜索"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && runSearch(workspaceId)}
            />
            <div style={{ position: "absolute", right: 3, top: 3, display: "flex", gap: 2 }}>
              {optionBtn("case_sensitive", "Aa", "区分大小写")}
              {optionBtn("whole_word", "ab", "全字匹配")}
              {optionBtn("use_regex", ".*", "使用正则表达式")}
            </div>
          </div>
        </div>

        {replaceOpen && (
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
          {results.length} 个文件，{totalMatches} 处匹配
          {truncated && "（结果过多，只展示部分，建议缩小搜索范围）"}
        </div>
      )}

      <div style={{ flex: 1, overflowY: "auto" }}>
        {results.length === 0 ? (
          <div style={{ padding: 24, textAlign: "center", fontSize: 12, color: "var(--text-secondary)" }}>
            {loading ? "搜索中…" : "输入关键词后回车，在当前工作区全部文件中搜索"}
          </div>
        ) : (
          results.map((file) => {
            const isCollapsed = collapsed.has(file.path);
            return (
              <div key={file.path}>
                <div
                  className="file-row"
                  style={{ cursor: "pointer", gridTemplateColumns: "auto 1fr auto auto" }}
                  onClick={() => toggleCollapsed(file.path)}
                >
                  {isCollapsed ? <ChevronRight style={{ width: 12, height: 12 }} /> : <ChevronDown style={{ width: 12, height: 12 }} />}
                  <span className="file-name" title={file.path}>
                    {fileName(file.path)}
                  </span>
                  <span style={{ fontSize: 11, color: "var(--text-secondary)" }}>{file.matches.length}</span>
                  {replaceOpen && (
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
                  file.matches.map((m, i) => (
                    <div
                      key={i}
                      className="file-row"
                      style={{ cursor: "pointer", paddingLeft: 28, gridTemplateColumns: "auto 1fr" }}
                      onClick={() => onOpenResult(file.path, m.line_number)}
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
