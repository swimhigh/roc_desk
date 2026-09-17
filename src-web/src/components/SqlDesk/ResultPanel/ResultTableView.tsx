import React, { useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { ArrowDown, ArrowUp } from "lucide-react";
import { ContextMenu, type ContextMenuItem } from "../../shared/ContextMenu";
import { useToastStore } from "../../shared/Toast";
import type { Cell, ExecuteResult } from "../../../types/bindings";

interface ResultTableViewProps {
  result: ExecuteResult;
}

const ROW_HEIGHT = 26;
const GUTTER_WIDTH = 44;

function cellDisplayText(cell: Cell): string {
  return cell.is_null ? "" : cell.text;
}

function cellSortKey(cell: Cell): [number, number | string] {
  if (cell.is_null) return [1, ""]; // NULL 始终排在最后，不管升序降序
  const n = Number(cell.text);
  return [0, cell.text !== "" && !Number.isNaN(n) ? n : cell.text];
}

function rowsToTsv(columns: { name: string }[], rows: Cell[][], withHeader: boolean): string {
  const lines: string[] = [];
  if (withHeader) lines.push(columns.map((c) => c.name).join("\t"));
  for (const row of rows) lines.push(row.map(cellDisplayText).join("\t"));
  return lines.join("\n");
}

/** 表格模式：虚拟滚动网格（docs/SQL_DESKTOP_PLAN.md §3.3，2026-09 用户反馈：
 * 参考 DBeaver——单元格要有 Excel 表格那样清晰的网格线/选中框，行可以选中
 * 并复制，点列头能排序）。排序/选择都是纯前端的，只作用在当前已经取回的这
 * 页数据上，不会重新请求数据库（这只是给用户一个"整理一下当前结果"的手段，
 * 不是服务端 ORDER BY 的替代品）。 */
export const ResultTableView: React.FC<ResultTableViewProps> = ({ result }) => {
  const parentRef = useRef<HTMLDivElement>(null);
  const push = useToastStore((s) => s.push);
  const [sort, setSort] = useState<{ col: number; dir: "asc" | "desc" } | null>(null);
  const [selectedRows, setSelectedRows] = useState<Set<number>>(new Set());
  const [anchorRow, setAnchorRow] = useState<number | null>(null);
  const [activeCell, setActiveCell] = useState<{ row: number; col: number } | null>(null);
  const [menu, setMenu] = useState<{ x: number; y: number; items: ContextMenuItem[] } | null>(null);

  const orderedRows = useMemo(() => {
    const indexed = result.rows.map((row, i) => ({ row, i }));
    if (!sort) return indexed;
    const dir = sort.dir === "asc" ? 1 : -1;
    return indexed.sort((a, b) => {
      const ka = cellSortKey(a.row[sort.col]);
      const kb = cellSortKey(b.row[sort.col]);
      if (ka[0] !== kb[0]) return ka[0] - kb[0];
      if (ka[1] < kb[1]) return -1 * dir;
      if (ka[1] > kb[1]) return 1 * dir;
      return 0;
    });
  }, [result.rows, sort]);

  const rowVirtualizer = useVirtualizer({
    count: orderedRows.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: 12,
  });

  const copyToClipboard = (text: string, message: string) => {
    void navigator.clipboard.writeText(text).then(
      () => push("success", message),
      (e) => push("error", `复制失败：${String(e)}`),
    );
  };

  const copySelectedRows = () => {
    const rows = orderedRows.filter((r) => selectedRows.has(r.i)).map((r) => r.row);
    copyToClipboard(rowsToTsv(result.columns, rows, false), `已复制 ${rows.length} 行`);
  };

  const toggleSort = (col: number) => {
    setSort((cur) => {
      if (!cur || cur.col !== col) return { col, dir: "asc" };
      if (cur.dir === "asc") return { col, dir: "desc" };
      return null;
    });
  };

  const handleRowGutterClick = (displayIndex: number, rowKey: number, e: React.MouseEvent) => {
    if (e.shiftKey && anchorRow != null) {
      const [lo, hi] = anchorRow <= displayIndex ? [anchorRow, displayIndex] : [displayIndex, anchorRow];
      setSelectedRows(new Set(orderedRows.slice(lo, hi + 1).map((r) => r.i)));
      return;
    }
    if (e.ctrlKey || e.metaKey) {
      setSelectedRows((cur) => {
        const next = new Set(cur);
        if (next.has(rowKey)) next.delete(rowKey);
        else next.add(rowKey);
        return next;
      });
      setAnchorRow(displayIndex);
      return;
    }
    setAnchorRow(displayIndex);
    setSelectedRows(new Set([rowKey]));
  };

  if (result.columns.length === 0) {
    return (
      <div className="empty-state" style={{ padding: 16, fontSize: 12 }}>
        {result.rows_affected != null ? `${result.rows_affected} 行受影响` : "没有返回结果集"}
      </div>
    );
  }

  return (
    <div
      style={{ display: "flex", flexDirection: "column", height: "100%" }}
      onKeyDown={(e) => {
        if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "c") {
          if (selectedRows.size > 0) {
            copySelectedRows();
          } else if (activeCell) {
            copyToClipboard(cellDisplayText(orderedRows[activeCell.row].row[activeCell.col]), "已复制单元格内容");
          }
        }
      }}
      tabIndex={0}
    >
      <div style={{ display: "flex", borderBottom: "1px solid var(--border-default)", background: "var(--bg-surface)" }}>
        <div
          style={{
            flex: `0 0 ${GUTTER_WIDTH}px`,
            width: GUTTER_WIDTH,
            padding: "4px 8px",
            fontSize: 12,
            borderRight: "1px solid var(--border-subtle)",
            textAlign: "right",
            color: "var(--text-secondary)",
          }}
        >
          #
        </div>
        {result.columns.map((col, i) => (
          <div
            key={i}
            onClick={() => toggleSort(i)}
            style={{
              flex: "1 0 140px",
              minWidth: 140,
              padding: "4px 8px",
              fontSize: 12,
              fontWeight: 600,
              borderRight: "1px solid var(--border-subtle)",
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
              cursor: "pointer",
              display: "flex",
              alignItems: "center",
              gap: 4,
              userSelect: "none",
            }}
            title={`${col.type_name}（点击排序）`}
          >
            <span style={{ overflow: "hidden", textOverflow: "ellipsis" }}>{col.name}</span>
            {sort?.col === i && (sort.dir === "asc" ? <ArrowUp size={11} /> : <ArrowDown size={11} />)}
          </div>
        ))}
      </div>
      <div ref={parentRef} style={{ flex: 1, overflow: "auto" }}>
        <div style={{ height: rowVirtualizer.getTotalSize(), position: "relative" }}>
          {rowVirtualizer.getVirtualItems().map((vRow) => {
            const { row, i: rowKey } = orderedRows[vRow.index];
            const isRowSelected = selectedRows.has(rowKey);
            return (
              <div
                key={rowKey}
                style={{
                  position: "absolute",
                  top: 0,
                  left: 0,
                  width: "100%",
                  height: ROW_HEIGHT,
                  transform: `translateY(${vRow.start}px)`,
                  display: "flex",
                  borderBottom: "1px solid var(--border-subtle)",
                  background: isRowSelected
                    ? "var(--bg-selected)"
                    : vRow.index % 2 === 1
                      ? "color-mix(in srgb, var(--text-primary) 4%, transparent)"
                      : undefined,
                }}
                onContextMenu={(e) => {
                  e.preventDefault();
                  setMenu({
                    x: e.clientX,
                    y: e.clientY,
                    items: [
                      { label: "复制整行", onClick: () => copyToClipboard(rowsToTsv(result.columns, [row], false), "已复制 1 行") },
                      { label: `复制选中的 ${selectedRows.size || 1} 行`, onClick: () => (selectedRows.size > 0 ? copySelectedRows() : copyToClipboard(rowsToTsv(result.columns, [row], false), "已复制 1 行")) },
                      { label: "复制（含表头）", onClick: () => copyToClipboard(rowsToTsv(result.columns, selectedRows.size > 0 ? orderedRows.filter((r) => selectedRows.has(r.i)).map((r) => r.row) : [row], true), "已复制") },
                    ],
                  });
                }}
              >
                <div
                  onClick={(e) => handleRowGutterClick(vRow.index, rowKey, e)}
                  style={{
                    flex: `0 0 ${GUTTER_WIDTH}px`,
                    width: GUTTER_WIDTH,
                    padding: "3px 8px",
                    fontSize: 12,
                    borderRight: "1px solid var(--border-subtle)",
                    textAlign: "right",
                    color: "var(--text-secondary)",
                    background: isRowSelected ? "var(--bg-selected)" : "var(--bg-surface)",
                    cursor: "pointer",
                    userSelect: "none",
                  }}
                  title="点击选中整行；Ctrl/Shift 多选"
                >
                  {vRow.index + 1}
                </div>
                {row.map((cell, colIndex) => {
                  const isActive = activeCell?.row === vRow.index && activeCell.col === colIndex;
                  return (
                    <div
                      key={colIndex}
                      onClick={() => setActiveCell({ row: vRow.index, col: colIndex })}
                      onContextMenu={(e) => {
                        e.preventDefault();
                        e.stopPropagation();
                        setActiveCell({ row: vRow.index, col: colIndex });
                        setMenu({
                          x: e.clientX,
                          y: e.clientY,
                          items: [
                            { label: "复制单元格", onClick: () => copyToClipboard(cellDisplayText(cell), "已复制单元格内容") },
                            { label: "复制整行", onClick: () => copyToClipboard(rowsToTsv(result.columns, [row], false), "已复制 1 行") },
                          ],
                        });
                      }}
                      style={{
                        flex: "1 0 140px",
                        minWidth: 140,
                        padding: "3px 8px",
                        fontSize: 12,
                        fontFamily: cell.is_binary ? "var(--font-mono)" : undefined,
                        color: cell.is_null ? "var(--text-tertiary, var(--text-secondary))" : undefined,
                        fontStyle: cell.is_null ? "italic" : undefined,
                        overflow: "hidden",
                        textOverflow: "ellipsis",
                        whiteSpace: "nowrap",
                        borderRight: "1px solid var(--border-subtle)",
                        outline: isActive ? "2px solid var(--accent)" : undefined,
                        outlineOffset: isActive ? "-2px" : undefined,
                        cursor: "cell",
                      }}
                      title={cell.is_null ? "NULL" : cell.text}
                    >
                      {cell.is_null ? "NULL" : cell.text}
                    </div>
                  );
                })}
              </div>
            );
          })}
        </div>
      </div>
      <div style={{ padding: "2px 8px", fontSize: 11, color: "var(--text-secondary)", borderTop: "1px solid var(--border-subtle)", display: "flex", gap: 8 }}>
        <span>
          {result.rows.length} 行{result.truncated ? "（已截断，导出获取全量）" : ""} · {result.duration_ms}ms
        </span>
        {selectedRows.size > 0 && <span>已选中 {selectedRows.size} 行（Ctrl+C 复制，或右键）</span>}
      </div>
      {menu && <ContextMenu x={menu.x} y={menu.y} items={menu.items} onClose={() => setMenu(null)} />}
    </div>
  );
};
