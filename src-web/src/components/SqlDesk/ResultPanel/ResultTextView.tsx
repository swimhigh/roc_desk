import React, { useMemo } from "react";
import type { ExecuteResult } from "../../../types/bindings";

interface ResultTextViewProps {
  result: ExecuteResult;
}

const MAX_TEXT_ROWS = 500;

function cellText(text: string, isNull: boolean): string {
  return isNull ? "NULL" : text;
}

/** 纯文本模式：等宽对齐渲染，类似 `psql`/`mysql` CLI 的输出（docs/
 * SQL_DESKTOP_PLAN.md §3.3）——一次性可全选复制，方便贴到聊天/工单/AI
 * 对话里。行数超过预览上限时提示导出获取全量，不做无限增长的 `<pre>`。 */
export const ResultTextView: React.FC<ResultTextViewProps> = ({ result }) => {
  const text = useMemo(() => {
    if (result.columns.length === 0) {
      return result.rows_affected != null ? `${result.rows_affected} 行受影响` : "没有返回结果集";
    }
    const headers = result.columns.map((c) => c.name);
    const rows = result.rows.slice(0, MAX_TEXT_ROWS).map((row) => row.map((c) => cellText(c.text, c.is_null)));
    const widths = headers.map((h, i) => Math.max(h.length, ...rows.map((r) => (r[i] ?? "").length), 3));
    const formatRow = (cells: string[]) => cells.map((c, i) => c.padEnd(widths[i])).join(" | ");
    const separator = widths.map((w) => "-".repeat(w)).join("-+-");
    const lines = [formatRow(headers), separator, ...rows.map(formatRow)];
    if (result.rows.length > MAX_TEXT_ROWS) {
      lines.push(`... 仅预览前 ${MAX_TEXT_ROWS} 行，导出获取全量`);
    }
    return lines.join("\n");
  }, [result]);

  return (
    <div style={{ height: "100%", display: "flex", flexDirection: "column" }}>
      <pre
        style={{
          flex: 1,
          margin: 0,
          padding: 8,
          overflow: "auto",
          fontFamily: "var(--font-mono)",
          fontSize: 12,
          whiteSpace: "pre",
          userSelect: "text",
        }}
      >
        {text}
      </pre>
      <div style={{ padding: "2px 8px", fontSize: 11, color: "var(--text-secondary)", borderTop: "1px solid var(--border-subtle)" }}>
        {result.rows.length} 行{result.truncated ? "（已截断，导出获取全量）" : ""} · {result.duration_ms}ms
      </div>
    </div>
  );
};
