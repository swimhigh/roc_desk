import React, { useEffect, useRef } from "react";
import Editor from "@monaco-editor/react";
import { KeyCode, KeyMod, Range, type editor as monacoEditorApi, type languages as monacoLanguagesApi } from "monaco-editor";
import { Play, Plus, Square, X } from "lucide-react";
import { useSqlStore } from "../../stores/sqlStore";
import { useThemeStore } from "../../stores/themeStore";
import { statementAt } from "./sqlText";
import { SQL_DRAG_MIME, type SqlDragPayload } from "./ObjectExplorer";
import { sqlService } from "../../services/sqlService";
import type { ColumnDef } from "../../types/bindings";

const SQL_KEYWORDS = [
  "SELECT", "FROM", "WHERE", "INSERT INTO", "UPDATE", "DELETE FROM", "JOIN",
  "LEFT JOIN", "INNER JOIN", "GROUP BY", "ORDER BY", "LIMIT", "VALUES", "SET",
  "AND", "OR", "NOT", "NULL", "AS", "DISTINCT", "HAVING", "ON",
];

/** 表名/字段名自动补全（2026-09 用户反馈）——表清单直接读全局 store（已经
 * 加载好，不用额外请求）；字段清单按需懒加载 + 缓存，不在每次敲键盘时都打
 * 一次 `sql_describe_object`。整个补全 provider 只注册一次（和
 * `CodeEditor.tsx` 的"转到定义" provider 同一种模式），靠 provider 回调里
 * 现取 `useSqlStore.getState()`，不会有闭包过期的问题。 */
let sqlCompletionRegistered = false;
const columnCache = new Map<string, ColumnDef[]>();

function registerSqlCompletionOnce(monaco: typeof import("monaco-editor")) {
  if (sqlCompletionRegistered) return;
  sqlCompletionRegistered = true;

  monaco.languages.registerCompletionItemProvider("sql", {
    triggerCharacters: ["."],
    provideCompletionItems: async (model, position) => {
      const state = useSqlStore.getState();
      const dsId = state.currentDataSourceId;
      const objects = state.objects;
      const textBefore = model.getValueInRange({
        startLineNumber: position.lineNumber,
        startColumn: 1,
        endLineNumber: position.lineNumber,
        endColumn: position.column,
      });
      const word = model.getWordUntilPosition(position);
      const range = {
        startLineNumber: position.lineNumber,
        endLineNumber: position.lineNumber,
        startColumn: word.startColumn,
        endColumn: word.endColumn,
      };

      const dotMatch = /([A-Za-z_][\w]*)\.$/.exec(textBefore);
      if (dotMatch && dsId) {
        const prefix = dotMatch[1];
        // 前缀可能是 schema（列出这个 schema 下的表）或表名（列出这张表的字段）。
        const schemaTables = objects.filter((o) => o.schema.toLowerCase() === prefix.toLowerCase());
        if (schemaTables.length > 0) {
          return {
            suggestions: schemaTables.map((o) => ({
              label: o.name,
              kind: monaco.languages.CompletionItemKind.Struct,
              insertText: o.name,
              range,
            })),
          };
        }
        const tableRef = objects.find((o) => o.name.toLowerCase() === prefix.toLowerCase());
        if (tableRef) {
          const cacheKey = `${dsId}:${tableRef.schema}.${tableRef.name}`;
          let columns = columnCache.get(cacheKey);
          if (!columns) {
            try {
              const def = await sqlService.describeObject(dsId, tableRef);
              columns = def.columns;
              columnCache.set(cacheKey, columns);
            } catch {
              columns = [];
            }
          }
          return {
            suggestions: columns.map((c) => ({
              label: c.name,
              detail: c.data_type,
              kind: monaco.languages.CompletionItemKind.Field,
              insertText: c.name,
              range,
            })),
          };
        }
        return { suggestions: [] };
      }

      const suggestions: monacoLanguagesApi.CompletionItem[] = [
        ...SQL_KEYWORDS.map((kw) => ({
          label: kw,
          kind: monaco.languages.CompletionItemKind.Keyword,
          insertText: kw,
          range,
        })),
        ...objects.map((o) => ({
          label: `${o.schema}.${o.name}`,
          kind: monaco.languages.CompletionItemKind.Struct,
          insertText: `${o.schema}.${o.name}`,
          range,
        })),
      ];
      return { suggestions };
    },
  });
}

interface SqlEditorTabsProps {
  onRun: (sql: string) => void;
  onCancel: () => void;
}

/** 中栏 SQL 标签页/编辑器（docs/SQL_DESKTOP_PLAN.md §3.3、§4.4）。不复用
 * `CodeEditor`——那个组件深度耦合"工作区文件 buffer"语义（详见调研结论），
 * 这里只用 `@monaco-editor/react` 最外层的受控 `<Editor>`，标签页状态、
 * 保存节流都在 `sqlStore` 里自己管。 */
export const SqlEditorTabs: React.FC<SqlEditorTabsProps> = ({ onRun, onCancel }) => {
  const tabs = useSqlStore((s) => s.tabs);
  const activeTabId = useSqlStore((s) => s.activeTabId);
  const tabContents = useSqlStore((s) => s.tabContents);
  const tabRuns = useSqlStore((s) => s.tabRuns);
  const createTab = useSqlStore((s) => s.createTab);
  const selectTab = useSqlStore((s) => s.selectTab);
  const deleteTab = useSqlStore((s) => s.deleteTab);
  const setTabContent = useSqlStore((s) => s.setTabContent);
  const saveTabContent = useSqlStore((s) => s.saveTabContent);
  const monacoTheme = useThemeStore((s) => (s.theme === "dark" ? "roc-dark" : "roc-light"));

  const editorRef = useRef<monacoEditorApi.IStandaloneCodeEditor | null>(null);
  const saveStatus = useSqlStore((s) => s.saveStatus);
  const closedTabs = useSqlStore((s) => s.closedTabs);
  const reopenTab = useSqlStore((s) => s.reopenTab);
  const renameTab = useSqlStore((s) => s.renameTab);
  const updateTabCursor = useSqlStore((s) => s.updateTabCursor);
  const selectAllRequestTabId = useSqlStore((s) => s.selectAllRequestTabId);
  const clearSelectAllRequest = useSqlStore((s) => s.clearSelectAllRequest);
  const creatingRef = useRef(false);
  const [renaming, setRenaming] = React.useState<string | null>(null);
  const [title, setTitle] = React.useState("");
  // 这一次应用进程生命周期内，每个标签页只在"第一次真正显示出来"时用
  // `cursor_json` 还原光标——同一次会话里来回切标签页靠 Monaco 自己的
  // `saveViewState` 记，不需要也不应该每次切回来都强制跳去 `cursor_json`
  // 记录的（可能是很久以前的）位置，覆盖掉这次会话里刚移动到的地方。
  const restoredCursorRef = useRef<Set<string>>(new Set());

  useEffect(() => {
    if (tabs.length === 0 && !creatingRef.current) {
      creatingRef.current = true;
      void createTab("查询 1").catch((e) => useSqlStore.setState({ error: String(e) })).finally(() => { creatingRef.current = false; });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tabs.length]);

  useEffect(() => {
    if (!activeTabId || restoredCursorRef.current.has(activeTabId)) return;
    restoredCursorRef.current.add(activeTabId);
    const tab = tabs.find((t) => t.id === activeTabId);
    if (!tab?.cursor_json) return;
    // 等 `<Editor>` 完成这次 `path` 切换对应的 model 挂载之后再设置光标——
    // 和 `activeTabId` 的 state 更新不在同一个 tick，用 rAF 错开一帧，避免
    // 抢在 Monaco 内部切 model 前调 `setPosition` 被后续挂载流程覆盖掉。
    const raf = requestAnimationFrame(() => {
      try {
        const pos = JSON.parse(tab.cursor_json!) as { lineNumber: number; column: number };
        const editor = editorRef.current;
        if (editor && editor.getModel()?.uri.toString().includes(activeTabId)) {
          editor.setPosition(pos);
          editor.revealPositionInCenter(pos);
        }
      } catch {
        // 忽略损坏的 cursor_json，不影响正常打开标签页。
      }
    });
    return () => cancelAnimationFrame(raf);
  }, [activeTabId, tabs]);

  // 双击表/生成模板刚插入一段 SQL 后，把这段内容整体选中——这样用户不用再
  // 手动框选就能直接点"运行语句/选区"（2026-09 用户反馈）。和上面的光标恢复
  // 同理，要等这次 `activeTabId` 切换对应的 model 真正挂载完才能设置选区，
  // 用 rAF 错开一帧。
  useEffect(() => {
    if (!activeTabId || selectAllRequestTabId !== activeTabId) return;
    const raf = requestAnimationFrame(() => {
      const editor = editorRef.current;
      const model = editor?.getModel();
      if (editor && model && model.uri.toString().includes(activeTabId)) {
        editor.setSelection(model.getFullModelRange());
        editor.focus();
      }
      clearSelectAllRequest();
    });
    return () => cancelAnimationFrame(raf);
  }, [activeTabId, selectAllRequestTabId, tabContents, clearSelectAllRequest]);

  const activeContent = activeTabId ? (tabContents[activeTabId] ?? "") : "";
  const activeRun = activeTabId ? tabRuns[activeTabId] : undefined;
  const running = Boolean(activeRun?.running);

  const runCurrent = (all = false) => {
    const editor = editorRef.current;
    if (!editor) return;
    const selection = editor.getSelection();
    const model = editor.getModel();
    const selected = selection && model ? model.getValueInRange(selection) : "";
    onRun(all ? editor.getValue() : selected.trim() || statementAt(editor.getValue(), model?.getOffsetAt(editor.getPosition()!) ?? 0));
  };
  const runRef = useRef(runCurrent);
  runRef.current = runCurrent;

  const handleChange = (value: string | undefined) => {
    if (!activeTabId) return;
    setTabContent(activeTabId, value ?? "");
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      <div className="editor-tabs">
        {/* 标签页多到超出宽度时，只让这一段自己横向滚动——保存/运行这些操作
            按钮固定在最右侧，不会被挤出可视区或被裁切（2026-09 用户反馈：
            标签多的时候运行按钮被覆盖了）。 */}
        <div style={{ display: "flex", alignItems: "center", overflowX: "auto", flex: 1, minWidth: 0, height: "100%" }}>
          {tabs.map((tab) => (
            <div
              key={tab.id}
              className={`editor-tab ${tab.id === activeTabId ? "active" : ""}`}
              onClick={() => void selectTab(tab.id)}
              onDoubleClick={() => { setRenaming(tab.id); setTitle(tab.title); }}
            >
              {renaming === tab.id ? <input autoFocus value={title} onChange={(e) => setTitle(e.target.value)}
                onClick={(e) => e.stopPropagation()} onBlur={() => { void renameTab(tab.id, title); setRenaming(null); }}
                onKeyDown={(e) => { if (e.key === "Enter") e.currentTarget.blur(); if (e.key === "Escape") setRenaming(null); }} /> : <span title="双击重命名">{tab.title}</span>}
              <span
                className="editor-tab-close"
                title="关闭标签（保留草稿）"
                onClick={(e) => {
                  e.stopPropagation();
                  void deleteTab(tab.id);
                }}
              >
                <X size={12} />
              </span>
            </div>
          ))}
          <button className="quick-tool-btn" title="新建查询标签页" onClick={() => void createTab(`查询 ${tabs.length + 1}`)}>
            <Plus size={14} />
          </button>
          <button className="btn ghost sm" disabled={!closedTabs.length} onClick={() => void reopenTab()}>恢复关闭的查询</button>
        </div>
        <div className="quick-tools" style={{ flexShrink: 0 }}>
          <button className="btn ghost sm" onClick={() => activeTabId && void saveTabContent(activeTabId)} title="保存草稿（Ctrl+S）">
            {activeTabId && saveStatus[activeTabId] === "saving" ? "保存中…" : activeTabId && saveStatus[activeTabId] === "error" ? "保存失败，重试" : "已保存"}
          </button>
          {running ? (
            <button className="btn danger sm" onClick={onCancel} title="停止">
              <Square size={13} /> 停止
            </button>
          ) : (
            <><button className="btn primary sm" onClick={() => runCurrent()} disabled={!activeTabId || !!activeRun?.pendingWrite || !!activeRun?.needsConfirmation || !activeContent.trim()} title="有选区时运行选区，否则运行光标所在语句（Ctrl+Enter）；复杂脚本请明确选区">
              <Play size={13} /> 运行语句/选区
            </button><button className="btn ghost sm" disabled={!activeContent.trim() || !!activeRun?.pendingWrite || !!activeRun?.needsConfirmation} onClick={() => runCurrent(true)}>运行全部</button></>
          )}
        </div>
      </div>
      <div
        style={{ flex: 1, minHeight: 0 }}
        onDragOver={(e) => {
          if (e.dataTransfer.types.includes(SQL_DRAG_MIME) || e.dataTransfer.types.includes("text/plain")) {
            e.preventDefault();
            e.dataTransfer.dropEffect = "copy";
          }
        }}
        onDrop={(e) => {
          const editor = editorRef.current;
          if (!editor) return;
          const raw = e.dataTransfer.getData(SQL_DRAG_MIME);
          let text = e.dataTransfer.getData("text/plain");
          if (raw) {
            try {
              const payload = JSON.parse(raw) as SqlDragPayload;
              text = payload.kind === "table" ? `${payload.schema}.${payload.name}` : payload.column;
            } catch {
              // 解析失败就用上面已经拿到的 text/plain 兜底。
            }
          }
          if (!text) return;
          e.preventDefault();
          const target = editor.getTargetAtClientPoint(e.clientX, e.clientY);
          const position = target?.position ?? editor.getPosition();
          if (!position) return;
          editor.executeEdits("drag-drop-object", [
            { range: new Range(position.lineNumber, position.column, position.lineNumber, position.column), text },
          ]);
          editor.focus();
        }}
      >
        {activeTabId && (
          <Editor
            path={`sql-query://${activeTabId}`}
            saveViewState
            keepCurrentModel
            language="sql"
            theme={monacoTheme}
            value={activeContent}
            onChange={handleChange}
            options={{ minimap: { enabled: false }, fontSize: 13, automaticLayout: true }}
            onMount={(editor, monaco) => {
              editorRef.current = editor;
              registerSqlCompletionOnce(monaco);
              editor.addCommand(KeyMod.CtrlCmd | KeyCode.Enter, () => runRef.current());
              editor.addCommand(KeyMod.CtrlCmd | KeyCode.KeyS, () => {
                const s = useSqlStore.getState();
                if (s.activeTabId) void s.saveTabContent(s.activeTabId);
              });
              editor.onDidChangeCursorPosition((e) => {
                const id = useSqlStore.getState().activeTabId;
                if (id) updateTabCursor(id, JSON.stringify({ lineNumber: e.position.lineNumber, column: e.position.column }));
              });
            }}
          />
        )}
      </div>
    </div>
  );
};
