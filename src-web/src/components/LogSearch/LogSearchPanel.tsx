import React, { useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import { Search, FileInput, FolderInput, Trash2, RefreshCw, HelpCircle, Folder, File as FileIcon, X } from "lucide-react";
import { useLogSearchStore } from "../../stores/logSearchStore";
import { sftpService } from "../../services/sftpService";
import { SegmentedControl } from "../shared/SegmentedControl";
import { useToastStore } from "../shared/Toast";
import { formatError } from "../../utils/error";
import type { FileEntry, LiveSearchResult, LogImportOutcome, LogImportProgressEvent, LogSearchResult } from "../../types/bindings";

interface LogSearchPanelProps {
  workspaceKind: "local" | "remote";
  profileId: string | null;
  workspaceName: string;
  rootPath: string;
}

type Row = LogSearchResult | LiveSearchResult;

/** 实时搜索走的是远程 `rg`（装了的话）/`grep -E`（见 log::remote::search_live），
 * 比左侧目录树的全文搜索（本地 Rust regex，逐文件读内容）更贴近"真正在服务器上跑
 * 一条 grep 命令"，理论上更强（服务端执行、天然支持整个 Linux 正则生态），但一个
 * 光秃秃的输入框不会用的人根本发现不了这一层——2026-08-18 用户原话："这个功能更
 * 高级，但用户体验不好，需要要一些常用的正则选项示例，或是一些高级的LINUX命令
 * 匹配的搜索方法示例"。点一条直接填进查询框并勾上"正则"，不需要自己照着抄。 */
const REGEX_EXAMPLES: { pattern: string; desc: string }[] = [
  { pattern: "ERROR|WARN", desc: "匹配 ERROR 或 WARN 任一关键词" },
  { pattern: "\\berror\\b", desc: "整词匹配 error，不会连带匹配 errorCode 这类子串" },
  { pattern: "^\\[ERROR\\]", desc: "只匹配行首就是 [ERROR] 的行" },
  { pattern: "\\d{4,}", desc: "四位及以上数字，常用来抓错误码/端口号/耗时毫秒数" },
  { pattern: "(\\d{1,3}\\.){3}\\d{1,3}", desc: "IPv4 地址" },
  { pattern: "traceid[=:]\\s*\\w+", desc: "提取 traceid=xxx 或 traceid: xxx 这类字段" },
  { pattern: "Exception|Traceback|panic", desc: "常见异常堆栈/崩溃关键词" },
  { pattern: "cost[:=]\\s*[0-9]{4,}", desc: "耗时字段达到 4 位数以上（排查慢请求）" },
];

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
    importLocalPaths,
    importRemotePaths,
    clearOlderThan,
  } = useLogSearchStore();
  const push = useToastStore((s) => s.push);
  const [importPath, setImportPath] = useState("");
  const [pathSuggestions, setPathSuggestions] = useState<FileEntry[]>([]);
  const [showSuggestions, setShowSuggestions] = useState(false);
  const [showExamples, setShowExamples] = useState(false);
  const suggestTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const blurTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  // 远程批量导入待选路径列表（2026-09 用户反馈：原来一次只能导入一个文件，体验
  // 不好，想支持多选文件/整个目录）——本地有系统原生的多选文件对话框，远程没有
  // 图形化浏览器，靠手动"输入路径 + 加入列表"攒一批，和"递归导入子目录"勾选框
  // 搭配，语义对应后端 `log_import_remote_paths` 的 `paths: string[]` + `recursive`。
  const [pendingRemotePaths, setPendingRemotePaths] = useState<string[]>([]);
  const [recursive, setRecursive] = useState(false);
  const [importProgress, setImportProgress] = useState<{ path: string; done: number; total: number } | null>(null);

  useEffect(() => {
    loadStats();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 实时搜索的目录输入框自动提示（2026-08-18 用户原话："这里输目录时，需要有自动
  // 提示，可下拉选择当前目录下的文件或目录"）：按最后一个 "/" 切成"要列的父目录"+
  // "当前正在打的这一段前缀"，列出父目录内容按前缀过滤，和 shell 里 Tab 补全是
  // 同一个思路。防抖 250ms，避免打字期间每敲一个字符都发一次 SFTP 请求。
  const fetchSuggestions = (path: string) => {
    if (!profileId) return;
    if (suggestTimer.current) clearTimeout(suggestTimer.current);
    suggestTimer.current = setTimeout(async () => {
      const lastSlash = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
      const dir = lastSlash >= 0 ? path.slice(0, lastSlash + 1) : "/";
      const prefix = lastSlash >= 0 ? path.slice(lastSlash + 1) : path;
      try {
        const entries = await sftpService.listDir(profileId, dir || "/");
        const filtered = entries.filter((e) => e.name.toLowerCase().startsWith(prefix.toLowerCase())).slice(0, 30);
        setPathSuggestions(filtered);
        setShowSuggestions(filtered.length > 0);
      } catch {
        // 输入到一半的路径大概率还不存在/还没打完，这是预期内的常见情况，静默失败
        // 就好，不用弹错误提示打断输入。
        setPathSuggestions([]);
        setShowSuggestions(false);
      }
    }, 250);
  };

  const pickSuggestion = (entry: FileEntry) => {
    const lastSlash = Math.max(livePath.lastIndexOf("/"), livePath.lastIndexOf("\\"));
    const dir = lastSlash >= 0 ? livePath.slice(0, lastSlash + 1) : "/";
    const next = entry.is_dir ? `${dir}${entry.name}/` : `${dir}${entry.name}`;
    setLivePath(next);
    if (entry.is_dir) {
      fetchSuggestions(next); // 选中目录后继续往下钻一层的候选列表，不用重新打字触发
    } else {
      setShowSuggestions(false);
    }
  };

  // 实时搜索默认查当前工作区目录（2026-08-18 用户反馈：默认应该是工作区目录，
  // 之前硬编码成 "/"）；只在挂载时设一次，之后用户在输入框里改了就不再覆盖——
  // 切工作区时这个面板会随 App.tsx 的整体卸载/重新挂载一起刷新，不需要额外监听。
  useEffect(() => {
    setLivePath(rootPath);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const rows: Row[] = mode === "index" ? indexResults : liveResults;

  const reportOutcome = (outcome: LogImportOutcome) => {
    if (outcome.failed.length === 0) {
      push("success", `已导入 ${outcome.lines_imported} 行（${outcome.files_imported} 个文件）`);
      return;
    }
    const first = outcome.failed[0];
    const more = outcome.failed.length > 1 ? ` 等 ${outcome.failed.length} 个文件失败` : "";
    push(
      "error",
      `已导入 ${outcome.lines_imported} 行（${outcome.files_imported} 个成功），${first.path}${more}：${first.error}`,
    );
  };

  /** `paths` 为空直接跳过（比如目录选择框被取消）。进度事件按 `requestId` 过滤，
   * 和 SFTP 传输进度（`SftpBrowser.tsx`）用的是同一套"组件本地监听 + 手动
   * unlisten"模式。 */
  const runImport = async (paths: string[], isLocal: boolean, recursiveFlag: boolean) => {
    if (paths.length === 0) return;
    const requestId = crypto.randomUUID();
    setImportProgress({ path: "", done: 0, total: paths.length });
    const unlisten = await listen<LogImportProgressEvent>("log:import-progress", (event) => {
      if (event.payload.requestId !== requestId) return;
      setImportProgress({ path: event.payload.path, done: event.payload.done, total: event.payload.total });
    });
    try {
      const outcome = isLocal
        ? await importLocalPaths(paths, recursiveFlag, workspaceName, requestId)
        : await importRemotePaths(profileId!, paths, recursiveFlag, workspaceName, requestId);
      reportOutcome(outcome);
      setPendingRemotePaths([]);
      setImportPath("");
    } catch (e) {
      push("error", `导入失败：${formatError(e)}`);
    } finally {
      unlisten();
      setImportProgress(null);
    }
  };

  const handleImportLocalFiles = async () => {
    const selected = await open({ directory: false, multiple: true });
    if (!selected) return;
    await runImport(Array.isArray(selected) ? selected : [selected], true, false);
  };

  const handleImportLocalDir = async () => {
    const selected = await open({ directory: true, multiple: false });
    if (!selected || Array.isArray(selected)) return;
    await runImport([selected], true, true);
  };

  const addPendingRemotePath = () => {
    const p = importPath.trim();
    if (!p || pendingRemotePaths.includes(p)) return;
    setPendingRemotePaths((ps) => [...ps, p]);
    setImportPath("");
    setShowSuggestions(false);
  };

  const handleImportRemote = async () => {
    if (!profileId) return;
    // 列表里已经攒了几个就导入那一批；列表是空的话，把输入框里正在打的这一个
    // 当成唯一路径直接导入——不强制用户"先加入列表再点导入"，单文件场景（原来
    // 唯一支持的用法）体验保持不变。
    const paths = pendingRemotePaths.length > 0 ? pendingRemotePaths : importPath.trim() ? [importPath.trim()] : [];
    await runImport(paths, false, recursive);
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
            <div style={{ position: "relative" }}>
              <input
                className="form-input"
                style={{ width: 200, height: 26 }}
                placeholder="远程目录，如 /var/log"
                value={livePath}
                onChange={(e) => {
                  setLivePath(e.target.value);
                  fetchSuggestions(e.target.value);
                }}
                onFocus={() => fetchSuggestions(livePath)}
                onKeyDown={(e) => {
                  if (e.key === "Escape") setShowSuggestions(false);
                }}
                onBlur={() => {
                  // 延迟关闭：不加这个延迟的话，鼠标点候选项时 input 的 blur 会先于
                  // 候选项的 click 触发，下拉框在点击生效前就被收起了。
                  blurTimer.current = setTimeout(() => setShowSuggestions(false), 150);
                }}
              />
              {showSuggestions && pathSuggestions.length > 0 && (
                <div className="path-suggest-dropdown">
                  {pathSuggestions.map((entry) => (
                    <div
                      key={entry.path}
                      className="path-suggest-item"
                      onClick={() => {
                        if (blurTimer.current) clearTimeout(blurTimer.current);
                        pickSuggestion(entry);
                      }}
                    >
                      {entry.is_dir ? <Folder style={{ width: 12, height: 12 }} /> : <FileIcon style={{ width: 12, height: 12 }} />}
                      <span>{entry.name}</span>
                    </div>
                  ))}
                </div>
              )}
            </div>
            <label style={{ fontSize: 12, color: "var(--text-secondary)", display: "flex", alignItems: "center", gap: 4 }}>
              <input type="checkbox" checked={isRegex} onChange={(e) => setIsRegex(e.target.checked)} />
              正则
            </label>
            <div style={{ position: "relative" }}>
              <button className="btn ghost sm" title="常用正则/搜索示例" onClick={() => setShowExamples((v) => !v)}>
                <HelpCircle style={{ width: 14, height: 14 }} />
              </button>
              {showExamples && (
                <div className="path-suggest-dropdown" style={{ width: 320, right: 0, left: "auto" }}>
                  <div style={{ padding: "6px 10px", fontSize: 11, color: "var(--text-secondary)", borderBottom: "1px solid var(--border-subtle)" }}>
                    实时搜索在远程跑 rg（没装则退回 grep -E），点一条直接填入并勾上"正则"
                  </div>
                  {REGEX_EXAMPLES.map((ex) => (
                    <div
                      key={ex.pattern}
                      className="path-suggest-item"
                      style={{ flexDirection: "column", alignItems: "flex-start", gap: 2, padding: "6px 10px" }}
                      onClick={() => {
                        setQuery(ex.pattern);
                        setIsRegex(true);
                        setShowExamples(false);
                      }}
                    >
                      <code style={{ fontFamily: "var(--font-mono)", fontSize: 12, color: "var(--accent)" }}>{ex.pattern}</code>
                      <span style={{ fontSize: 11, color: "var(--text-secondary)" }}>{ex.desc}</span>
                    </div>
                  ))}
                </div>
              )}
            </div>
          </>
        )}
        <button className="btn primary sm" onClick={() => runSearch(workspaceKind, profileId)} disabled={loading}>
          <Search style={{ width: 14, height: 14 }} /> {loading ? "搜索中…" : "搜索"}
        </button>
      </div>

      {mode === "index" && (
        <div style={{ borderTop: "1px solid var(--border-subtle)" }}>
          <div className="editor-toolbar" style={{ height: "auto", minHeight: 32, gap: 8, flexWrap: "wrap" }}>
            <span style={{ fontSize: 12, color: "var(--text-secondary)" }}>
              {stats ? `已索引 ${stats.row_count} 行 · ${stats.job_count} 个导入任务` : "索引统计加载中…"}
            </span>
            {workspaceKind === "local" ? (
              <>
                <button className="btn ghost sm" onClick={handleImportLocalFiles} disabled={importing} title="可一次选中多个文件">
                  <FileInput style={{ width: 14, height: 14 }} /> {importing ? "导入中…" : "导入文件"}
                </button>
                <button className="btn ghost sm" onClick={handleImportLocalDir} disabled={importing} title="导入整个目录（含子目录）">
                  <FolderInput style={{ width: 14, height: 14 }} /> 导入目录
                </button>
              </>
            ) : (
              <>
                <input
                  className="form-input"
                  style={{ width: 220, height: 24 }}
                  placeholder="远程日志文件/目录完整路径"
                  value={importPath}
                  onChange={(e) => setImportPath(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") addPendingRemotePath();
                  }}
                />
                <button
                  className="btn ghost sm"
                  onClick={addPendingRemotePath}
                  disabled={!importPath.trim()}
                  title="加入待导入列表，可以攒好几个路径一起导入"
                >
                  + 加入列表
                </button>
                <label style={{ fontSize: 12, color: "var(--text-secondary)", display: "flex", alignItems: "center", gap: 4 }}>
                  <input type="checkbox" checked={recursive} onChange={(e) => setRecursive(e.target.checked)} />
                  路径是目录时递归导入
                </label>
                <button
                  className="btn ghost sm"
                  onClick={handleImportRemote}
                  disabled={importing || (pendingRemotePaths.length === 0 && !importPath.trim())}
                >
                  <FileInput style={{ width: 14, height: 14 }} />{" "}
                  {importing ? "导入中…" : pendingRemotePaths.length > 0 ? `导入（${pendingRemotePaths.length} 项）` : "导入远程文件"}
                </button>
              </>
            )}
            <button className="btn ghost sm" onClick={loadStats} title="刷新统计">
              <RefreshCw style={{ width: 14, height: 14 }} />
            </button>
            <button className="btn ghost sm" onClick={handleClear} title="清理 30 天前导入的索引（DESIGN.md §十-3 磁盘配额）">
              <Trash2 style={{ width: 14, height: 14 }} />
            </button>
          </div>
          {workspaceKind === "remote" && pendingRemotePaths.length > 0 && (
            <div style={{ display: "flex", flexWrap: "wrap", gap: 6, padding: "0 12px 8px" }}>
              {pendingRemotePaths.map((p) => (
                <span
                  key={p}
                  style={{
                    display: "inline-flex",
                    alignItems: "center",
                    gap: 4,
                    fontSize: 11,
                    padding: "2px 6px",
                    borderRadius: 4,
                    background: "var(--bg-surface-raised)",
                    color: "var(--text-secondary)",
                  }}
                >
                  {p}
                  <X
                    style={{ width: 11, height: 11, cursor: "pointer" }}
                    onClick={() => setPendingRemotePaths((ps) => ps.filter((x) => x !== p))}
                  />
                </span>
              ))}
            </div>
          )}
          {importProgress && (
            <div style={{ padding: "0 12px 8px", fontSize: 11, color: "var(--text-secondary)" }}>
              正在导入第 {Math.min(importProgress.done + 1, Math.max(importProgress.total, 1))}/{importProgress.total} 个：{importProgress.path}
            </div>
          )}
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
