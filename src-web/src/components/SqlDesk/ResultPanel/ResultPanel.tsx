import React from "react";
import { save as saveFileDialog } from "@tauri-apps/plugin-dialog";
import { Download, Loader2, Maximize2, Minimize2, RefreshCw } from "lucide-react";
import { useSqlStore } from "../../../stores/sqlStore";
import { sqlService } from "../../../services/sqlService";
import { useToastStore } from "../../shared/Toast";
import { formatError } from "../../../utils/error";
import { ResultTableView } from "./ResultTableView";
import { ResultTextView } from "./ResultTextView";
import type { Cell, ExecuteResult } from "../../../types/bindings";

interface ResultPanelProps {
  maximized: boolean;
  onToggleMaximized: () => void;
}

function csvEscape(s: string): string {
  if (/[",\n\r]/.test(s)) return `"${s.replace(/"/g, '""')}"`;
  return s;
}

function resultToCsv(result: ExecuteResult): string {
  const cellText = (c: Cell) => (c.is_null ? "" : c.text);
  const lines = [result.columns.map((c) => csvEscape(c.name)).join(",")];
  for (const row of result.rows) lines.push(row.map((c) => csvEscape(cellText(c))).join(","));
  return lines.join("\r\n");
}

function resultToJson(result: ExecuteResult): string {
  const objects = result.rows.map((row) =>
    Object.fromEntries(result.columns.map((c, i) => [c.name, row[i].is_null ? null : row[i].text])),
  );
  return JSON.stringify(objects, null, 2);
}

/** 下栏结果区：表格/纯文本双模式容器 + 运行中/错误/DDL 确认/写操作确认这几种
 * 状态的展示（docs/SQL_DESKTOP_PLAN.md §3.3、§7）。最大化开关放在这里自己的
 * 工具条上（DBeaver 风格的一个图标），而不是外层工作区的独立按钮。 */
export const ResultPanel: React.FC<ResultPanelProps> = ({ maximized, onToggleMaximized }) => {
  const activeTabId = useSqlStore((s) => s.activeTabId);
  const tabs = useSqlStore((s) => s.tabs);
  const tabRuns = useSqlStore((s) => s.tabRuns);
  const setResultViewMode = useSqlStore((s) => s.setResultViewMode);
  const runSql = useSqlStore((s) => s.runSql);
  const cancelQuery = useSqlStore((s) => s.cancelQuery);
  const confirmWrite = useSqlStore((s) => s.confirmWrite);
  const rollbackWrite = useSqlStore((s) => s.rollbackWrite);
  const history = useSqlStore((s) => s.history);
  const source = useSqlStore((s) => s.dataSources.find((d) => d.id === s.currentDataSourceId));
  const [showHistory, setShowHistory] = React.useState(false);
  const push = useToastStore((s) => s.push);

  const tab = tabs.find((t) => t.id === activeTabId);
  const run = activeTabId ? tabRuns[activeTabId] : undefined;

  if (!tab) {
    return <div className="empty-state" style={{ padding: 16, fontSize: 12 }}>新建或选择一个标签页开始查询</div>;
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      <div style={{ display: "flex", alignItems: "center", gap: 6, padding: 4, borderBottom: "1px solid var(--border-subtle)" }}>
        <button className="btn ghost sm" onClick={() => { setShowHistory(!showHistory); void useSqlStore.getState().loadHistory(); }}>{showHistory ? "返回结果" : "查询历史"}</button>
        <div className="segmented">
          <button
            className={`seg-btn ${tab.result_view_mode === "table" ? "active" : ""}`}
            onClick={() => void setResultViewMode(tab.id, "table")}
          >
            表格
          </button>
          <button
            className={`seg-btn ${tab.result_view_mode === "text" ? "active" : ""}`}
            onClick={() => void setResultViewMode(tab.id, "text")}
          >
            文本
          </button>
        </div>
        {run?.result?.rows_affected != null && (
          <span style={{ fontSize: 11, color: "var(--text-secondary)" }}>{run.result.rows_affected} 行受影响</span>
        )}
        <button
          className="btn ghost sm"
          disabled={!run?.lastSql || run.running}
          title="重新执行当前结果对应的语句"
          onClick={() => void runSql(tab.id, run!.lastSql, false)}
        >
          <RefreshCw size={13} /> 刷新
        </button>
        <button
          className="btn ghost sm"
          disabled={!run?.result || run.result.columns.length === 0}
          title="导出当前网格里已加载的数据（不是整张表）"
          onClick={() => {
            void (async () => {
              const path = await saveFileDialog({
                defaultPath: `${tab.title}.csv`,
                filters: [
                  { name: "CSV", extensions: ["csv"] },
                  { name: "JSON", extensions: ["json"] },
                ],
              });
              if (!path || !run?.result) return;
              const content = path.toLowerCase().endsWith(".json") ? resultToJson(run.result) : resultToCsv(run.result);
              await sqlService.writeTextFile(path, content);
              push("success", `已导出到 ${path}`);
            })().catch((e) => push("error", `导出失败：${formatError(e)}`));
          }}
        >
          <Download size={13} /> 导出
        </button>
        <button
          className="icon-btn"
          style={{ marginLeft: "auto" }}
          title={maximized ? "恢复编辑器" : "最大化结果区"}
          onClick={onToggleMaximized}
        >
          {maximized ? <Minimize2 size={14} /> : <Maximize2 size={14} />}
        </button>
      </div>
      <div style={{ flex: 1, minHeight: 0, overflow: "auto" }}>
        {showHistory ? <div style={{ padding: 8 }}>{history.length === 0 ? "暂无查询历史" : history.map((h) => <div key={h.id} style={{ borderBottom: "1px solid var(--border-default)", padding: 8 }}><div>{h.created_at} · {h.status} · {h.duration_ms ?? "—"}ms</div><pre style={{ whiteSpace: "pre-wrap" }}>{h.sql_text}</pre><button className="btn ghost sm" onClick={() => void (async () => { const s = useSqlStore.getState(); await s.createTab(h.title ?? "历史查询"); const id = useSqlStore.getState().activeTabId; if (id) s.setTabContent(id, h.sql_text); setShowHistory(false); })().catch((e) => useSqlStore.setState({ error: String(e) }))}>恢复为新查询</button></div>)}</div> : <>
        {run?.running && (
          <div className="empty-state" style={{ padding: 16, display: "flex", alignItems: "center", gap: 8, justifyContent: "center" }}>
            <Loader2 size={16} className="spin" /> 执行中…
            <button className="btn ghost sm" disabled={!run.queryId} onClick={() => void cancelQuery(tab.id)}>
              取消
            </button>
          </div>
        )}
        {run?.error && (
          <div style={{ padding: 16, fontSize: 12, color: "var(--danger)", whiteSpace: "pre-wrap" }}>{run.error}</div>
        )}
        {run?.needsConfirmation && (
          <div style={{ padding: 16, fontSize: 13 }}>
            <p>这是一条 DDL/需要确认的语句，确认后才会执行。</p>
            <p>{source?.name} · {source?.host} · {source?.environment}（可能无法回滚）</p><pre style={{ whiteSpace: "pre-wrap" }}>{run.lastSql}</pre>
            <div style={{ display: "flex", gap: 8 }}>
              <button className="btn primary sm" onClick={() => void runSql(tab.id, run.lastSql, true)}>
                确认执行
              </button>
              <button className="btn ghost sm" onClick={() => useSqlStore.setState((s) => ({ tabRuns: { ...s.tabRuns, [tab.id]: { ...run, needsConfirmation: false } } }))}>取消执行</button>
            </div>
          </div>
        )}
        {run?.pendingWrite && (
          <div style={{ padding: 16, fontSize: 13 }}>
            <p>该写操作已在事务中执行，受影响 <b>{run.pendingWrite.rows_affected ?? 0}</b> 行，确认提交？</p>
            <p>{source?.name} · {source?.host} · {source?.environment}。事务尚未提交，请及时处理。</p><pre style={{ whiteSpace: "pre-wrap" }}>{run.lastSql}</pre>
            <div style={{ display: "flex", gap: 8 }}>
              <button className="btn primary sm" disabled={run.running} onClick={() => void confirmWrite(tab.id)}>
                提交
              </button>
              <button className="btn danger sm" disabled={run.running} onClick={() => void rollbackWrite(tab.id)}>
                回滚
              </button>
            </div>
          </div>
        )}
        {run?.result &&
          (tab.result_view_mode === "text" ? (
            <ResultTextView result={run.result} />
          ) : (
            <ResultTableView result={run.result} />
          ))}
        {!run?.running && !run?.error && !run?.result && !run?.needsConfirmation && !run?.pendingWrite && (
          <div className="empty-state" style={{ padding: 16, fontSize: 12 }}>运行查询（Ctrl+Enter）后在这里查看结果</div>
        )}
        </>}
      </div>
    </div>
  );
};
