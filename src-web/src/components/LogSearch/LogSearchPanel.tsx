import React, { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { Search, FileInput, Trash2, RefreshCw } from "lucide-react";
import { useLogSearchStore } from "../../stores/logSearchStore";
import { SegmentedControl } from "../shared/SegmentedControl";
import { useToastStore } from "../shared/Toast";
import { formatError } from "../../utils/error";
import type { LiveSearchResult, LogSearchResult } from "../../types/bindings";

interface LogSearchPanelProps {
  workspaceKind: "local" | "remote";
  profileId: string | null;
  workspaceName: string;
  rootPath: string;
}

type Row = LogSearchResult | LiveSearchResult;

/**
 * 日志搜索面板（DESIGN.md §3.4）：模式 B 默认查本地 FTS5 索引，模式 A 对远程工作区
 * 提供实时 rg/grep（不落索引，适合"这次偶发问题"这种一次性排查）。
 */
export const LogSearchPanel: React.FC<LogSearchPanelProps> = ({ workspaceKind, profileId, workspaceName, rootPath }) => {
  const {
    mode,
    query,
    livePath,
    isRegex,
    loading,
    error,
    indexResults,
    liveResults,
    stats,
    selectedIndex,
    importing,
    setMode,
    setQuery,
    setLivePath,
    setIsRegex,
    select,
    runSearch,
    loadStats,
    importLocalFile,
    importRemoteFile,
    clearOlderThan,
  } = useLogSearchStore();
  const push = useToastStore((s) => s.push);
  const [importPath, setImportPath] = useState("");

  useEffect(() => {
    loadStats();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 实时搜索默认查当前工作区目录（2026-08-18 用户反馈：默认应该是工作区目录，
  // 之前硬编码成 "/"）；只在挂载时设一次，之后用户在输入框里改了就不再覆盖——
  // 切工作区时这个面板会随 App.tsx 的整体卸载/重新挂载一起刷新，不需要额外监听。
  useEffect(() => {
    setLivePath(rootPath);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const rows: Row[] = mode === "index" ? indexResults : liveResults;

  const handleImport = async () => {
    if (workspaceKind === "local") {
      const selected = await open({ directory: false, multiple: false });
      if (!selected || Array.isArray(selected)) return;
      try {
        const count = await importLocalFile(selected, workspaceName);
        push("success", `已导入 ${count} 行`);
      } catch (e) {
        push("error", `导入失败：${formatError(e)}`);
      }
      return;
    }
    if (!profileId || !importPath.trim()) return;
    try {
      const count = await importRemoteFile(profileId, importPath.trim(), workspaceName);
      push("success", `已导入 ${count} 行`);
      setImportPath("");
    } catch (e) {
      push("error", `导入失败：${formatError(e)}`);
    }
  };

  const handleClear = async () => {
    try {
      const removed = await clearOlderThan(30);
      push("success", `已清理 ${removed} 行（30 天前的导入）`);
    } catch (e) {
      push("error", `清理失败：${formatError(e)}`);
    }
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      <div className="editor-toolbar" style={{ flexWrap: "wrap", height: "auto", minHeight: 32, gap: 8 }}>
        <SegmentedControl
          value={mode}
          onChange={setMode}
          options={[
            { value: "index", label: "索引搜索" },
            { value: "live", label: workspaceKind === "remote" ? "实时搜索" : "实时搜索（仅远程）" },
          ]}
        />
        <input
          className="form-input"
          style={{ flex: 1, minWidth: 160, height: 26 }}
          placeholder={mode === "index" ? "在已索引的日志中搜索…" : "搜索关键词或正则…"}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && runSearch(workspaceKind, profileId)}
        />
        {mode === "live" && (
          <>
            <input
              className="form-input"
              style={{ width: 200, height: 26 }}
              placeholder="远程目录，如 /var/log"
              value={livePath}
              onChange={(e) => setLivePath(e.target.value)}
            />
            <label style={{ fontSize: 12, color: "var(--text-secondary)", display: "flex", alignItems: "center", gap: 4 }}>
              <input type="checkbox" checked={isRegex} onChange={(e) => setIsRegex(e.target.checked)} />
              正则
            </label>
          </>
        )}
        <button className="btn primary sm" onClick={() => runSearch(workspaceKind, profileId)} disabled={loading}>
          <Search style={{ width: 14, height: 14 }} /> {loading ? "搜索中…" : "搜索"}
        </button>
      </div>

      {mode === "index" && (
        <div
          className="editor-toolbar"
          style={{ height: "auto", minHeight: 32, gap: 8, borderTop: "1px solid var(--border-subtle)" }}
        >
          <span style={{ fontSize: 12, color: "var(--text-secondary)" }}>
            {stats ? `已索引 ${stats.row_count} 行 · ${stats.job_count} 个导入任务` : "索引统计加载中…"}
          </span>
          {workspaceKind === "remote" && (
            <input
              className="form-input"
              style={{ width: 220, height: 24 }}
              placeholder="远程日志文件完整路径"
              value={importPath}
              onChange={(e) => setImportPath(e.target.value)}
            />
          )}
          <button className="btn ghost sm" onClick={handleImport} disabled={importing}>
            <FileInput style={{ width: 14, height: 14 }} /> {importing ? "导入中…" : workspaceKind === "local" ? "导入本地文件" : "导入远程文件"}
          </button>
          <button className="btn ghost sm" onClick={loadStats} title="刷新统计">
            <RefreshCw style={{ width: 14, height: 14 }} />
          </button>
          <button className="btn ghost sm" onClick={handleClear} title="清理 30 天前导入的索引（DESIGN.md §十-3 磁盘配额）">
            <Trash2 style={{ width: 14, height: 14 }} />
          </button>
        </div>
      )}

      {error && (
        <div style={{ padding: "6px 12px", fontSize: 12, color: "var(--danger, #e5484d)" }}>{error}</div>
      )}

      <div style={{ flex: 1, overflowY: "auto" }}>
        {rows.length === 0 ? (
          <div style={{ padding: 24, textAlign: "center", fontSize: 13, color: "var(--text-secondary)" }}>
            {loading ? "搜索中…" : "还没有结果，输入关键词后回车搜索"}
          </div>
        ) : (
          rows.map((r, i) => {
            const line = "line" in r ? r.line : r.snippet;
            return (
              <div
                key={i}
                className={`file-row ${selectedIndex === i ? "active" : ""}`}
                style={{ display: "flex", flexDirection: "column", alignItems: "flex-start", cursor: "pointer", height: "auto", padding: "6px 12px" }}
                onClick={() => select(i)}
              >
                <div style={{ display: "flex", gap: 8, fontSize: 11, color: "var(--text-secondary)" }}>
                  <span>{r.file_path}:{r.line_number}</span>
                  {r.log_level && <span className={`log-level-tag ${r.log_level.toLowerCase()}`}>{r.log_level}</span>}
                  {r.timestamp && <span>{r.timestamp}</span>}
                </div>
                <div
                  style={{ fontSize: 12, fontFamily: "var(--font-mono)", whiteSpace: "pre-wrap", wordBreak: "break-all" }}
                  dangerouslySetInnerHTML={{ __html: "snippet" in r ? r.snippet : escapeHtml(line) }}
                />
              </div>
            );
          })
        )}
      </div>
    </div>
  );
};

function escapeHtml(s: string): string {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}
