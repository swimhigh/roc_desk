import React, { useEffect, useMemo, useState } from "react";
import { Loader2, Plus, RefreshCw, Trash2, X } from "lucide-react";
import { sqlService } from "../../services/sqlService";
import { useToastStore } from "../shared/Toast";
import { formatError } from "../../utils/error";
import { ConfirmDialog } from "../shared/ConfirmDialog";
import type { Cell, ColumnDef, ExecuteResult, NamedCell, ObjectDefinition, ObjectRef } from "../../types/bindings";

interface TableDataDialogProps {
  dataSourceId: string;
  object: ObjectRef;
  readonly: boolean;
  onClose: () => void;
  /** 生成的 ALTER TABLE 语句交给外层塞进一个新 SQL 标签页——复用已有的 DDL
   * 确认闸门，这个弹窗本身不执行任何结构修改（见 data_editor.rs 的说明）。 */
  onOpenAlterSql: (sql: string) => void;
}

const PAGE_SIZE = 100;

function cellToInput(cell: Cell): { is_null: boolean; text: string } {
  return { is_null: cell.is_null, text: cell.text };
}

/** 右键"查看/编辑数据"打开的弹窗（docs/SQL_DESKTOP_PLAN.md，2026-09 用户
 * 反馈：参考 DBeaver 的表编辑界面，属性/数据两个标签页）。数据页支持分页
 * 浏览、双击单元格改值、增删行；结构页展示列/索引，并能生成 ALTER TABLE
 * 语句交给编辑器执行。范围说明：
 * - 编辑/新增/删除都要求表有主键（没有主键没法安全定位到具体是哪一行），
 *   没有主键时这个弹窗整体只读。
 * - 二进制列（`is_binary`）不支持通过这个表格编辑，显示但禁止双击进入
 *   编辑态。
 */
export const TableDataDialog: React.FC<TableDataDialogProps> = ({
  dataSourceId,
  object,
  readonly,
  onClose,
  onOpenAlterSql,
}) => {
  const push = useToastStore((s) => s.push);
  const [tab, setTab] = useState<"data" | "structure">("data");
  const [def, setDef] = useState<ObjectDefinition | null>(null);
  const [page, setPage] = useState(0);
  const [totalRows, setTotalRows] = useState<number | null>(null);
  const [result, setResult] = useState<ExecuteResult | null>(null);
  const [loading, setLoading] = useState(false);
  const [editing, setEditing] = useState<{ row: number; col: number } | null>(null);
  const [editValue, setEditValue] = useState("");
  const [selectedRows, setSelectedRows] = useState<Set<number>>(new Set());
  const [deleteConfirm, setDeleteConfirm] = useState(false);
  const [newColumn, setNewColumn] = useState({ name: "", data_type: "", nullable: true });

  const pkColumns = useMemo(() => def?.columns.filter((c) => c.is_primary_key).map((c) => c.name) ?? [], [def]);
  const canEdit = !readonly && pkColumns.length > 0;

  useEffect(() => {
    sqlService
      .describeObject(dataSourceId, object)
      .then(setDef)
      .catch((e) => push("error", `获取表结构失败：${formatError(e)}`));
    sqlService
      .tableRowCount(dataSourceId, object)
      .then(setTotalRows)
      .catch(() => setTotalRows(null));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [dataSourceId, object.schema, object.name]);

  const loadPage = () => {
    setLoading(true);
    setSelectedRows(new Set());
    sqlService
      .tablePage(dataSourceId, object, PAGE_SIZE, page * PAGE_SIZE)
      .then(setResult)
      .catch((e) => push("error", `加载数据失败：${formatError(e)}`))
      .finally(() => setLoading(false));
  };

  useEffect(() => {
    if (tab === "data") loadPage();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tab, page, dataSourceId, object.schema, object.name]);

  const pkForRow = (row: Cell[]): NamedCell[] | null => {
    if (!result || pkColumns.length === 0) return null;
    return pkColumns.map((name) => {
      const idx = result.columns.findIndex((c) => c.name === name);
      return { name, value: cellToInput(row[idx]) };
    });
  };

  const commitEdit = async (rowIndex: number, colIndex: number) => {
    if (!result) return;
    const row = result.rows[rowIndex];
    const pk = pkForRow(row);
    if (!pk) return;
    const column = result.columns[colIndex].name;
    try {
      await sqlService.tableUpdateCell(dataSourceId, object, pk, column, { is_null: false, text: editValue });
      setEditing(null);
      loadPage();
    } catch (e) {
      push("error", `保存失败：${formatError(e)}`);
    }
  };

  const deleteSelected = async () => {
    if (!result) return;
    setDeleteConfirm(false);
    for (const idx of selectedRows) {
      const pk = pkForRow(result.rows[idx]);
      if (!pk) continue;
      try {
        await sqlService.tableDeleteRow(dataSourceId, object, pk);
      } catch (e) {
        push("error", `删除失败：${formatError(e)}`);
        break;
      }
    }
    loadPage();
    sqlService.tableRowCount(dataSourceId, object).then(setTotalRows).catch(() => {});
  };

  const addRow = async () => {
    if (!def) return;
    const values: NamedCell[] = def.columns.map((c) => ({ name: c.name, value: { is_null: !c.nullable ? false : true, text: "" } }));
    try {
      await sqlService.tableInsertRow(dataSourceId, object, values);
      push("success", "已插入一行空白记录，请双击单元格填值");
      loadPage();
    } catch (e) {
      push("error", `新增行失败：${formatError(e)}`);
    }
  };

  const generateAdd = async () => {
    if (!newColumn.name.trim() || !newColumn.data_type.trim()) return;
    try {
      const sql = await sqlService.generateAlterTable(dataSourceId, object, [
        { op: "add_column", name: newColumn.name.trim(), data_type: newColumn.data_type.trim(), nullable: newColumn.nullable },
      ]);
      onOpenAlterSql(sql.join(";\n") + ";");
      setNewColumn({ name: "", data_type: "", nullable: true });
    } catch (e) {
      push("error", `生成语句失败：${formatError(e)}`);
    }
  };

  const generateDrop = async (col: ColumnDef) => {
    try {
      const sql = await sqlService.generateAlterTable(dataSourceId, object, [{ op: "drop_column", name: col.name }]);
      onOpenAlterSql(sql.join(";\n") + ";");
    } catch (e) {
      push("error", `生成语句失败：${formatError(e)}`);
    }
  };

  const generateRename = async (col: ColumnDef) => {
    const newName = window.prompt(`将字段 "${col.name}" 重命名为：`, col.name);
    if (!newName || newName === col.name) return;
    try {
      const sql = await sqlService.generateAlterTable(dataSourceId, object, [
        { op: "rename_column", old_name: col.name, new_name: newName },
      ]);
      onOpenAlterSql(sql.join(";\n") + ";");
    } catch (e) {
      push("error", `生成语句失败：${formatError(e)}`);
    }
  };

  return (
    <div className="dialog-overlay" onClick={onClose}>
      <div className="dialog" style={{ width: "min(1000px, 92vw)", height: "min(680px, 88vh)", display: "flex", flexDirection: "column" }} onClick={(e) => e.stopPropagation()}>
        <div className="dialog-title-bar">
          <span>{object.schema}.{object.name}</span>
          <button className="icon-btn" onClick={onClose}><X size={16} /></button>
        </div>
        <div className="segmented" style={{ margin: "8px 12px", alignSelf: "flex-start" }}>
          <button className={`seg-btn ${tab === "data" ? "active" : ""}`} onClick={() => setTab("data")}>数据</button>
          <button className={`seg-btn ${tab === "structure" ? "active" : ""}`} onClick={() => setTab("structure")}>属性/结构</button>
        </div>

        {!canEdit && (
          <div style={{ margin: "0 12px 8px", fontSize: 12, color: "var(--text-secondary)" }}>
            {readonly ? "该数据源为只读模式，仅可浏览数据。" : "该表没有检测到主键，无法安全定位单行，仅可浏览数据。"}
          </div>
        )}

        {tab === "data" ? (
          <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column", padding: "0 12px 12px" }}>
            <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 6 }}>
              <button className="btn ghost sm" onClick={loadPage} title="刷新"><RefreshCw size={13} /></button>
              <button className="btn ghost sm" disabled={!canEdit} onClick={() => void addRow()}><Plus size={13} /> 新增行</button>
              <button className="btn danger sm" disabled={!canEdit || selectedRows.size === 0} onClick={() => setDeleteConfirm(true)}>
                <Trash2 size={13} /> 删除所选（{selectedRows.size}）
              </button>
              <div style={{ marginLeft: "auto", display: "flex", alignItems: "center", gap: 6, fontSize: 12 }}>
                <button className="btn ghost sm" disabled={page === 0} onClick={() => setPage((p) => Math.max(0, p - 1))}>上一页</button>
                <span>
                  第 {page + 1} 页{totalRows != null ? ` / 共 ${Math.max(1, Math.ceil(totalRows / PAGE_SIZE))} 页（${totalRows} 行）` : ""}
                </span>
                <button
                  className="btn ghost sm"
                  disabled={result ? result.rows.length < PAGE_SIZE : true}
                  onClick={() => setPage((p) => p + 1)}
                >
                  下一页
                </button>
              </div>
            </div>
            <div style={{ flex: 1, minHeight: 0, overflow: "auto", border: "1px solid var(--border-default)" }}>
              {loading && (
                <div className="empty-state" style={{ padding: 16 }}><Loader2 size={16} className="spin" /> 加载中…</div>
              )}
              {!loading && result && (
                <table style={{ borderCollapse: "collapse", width: "100%", fontSize: 12 }}>
                  <thead>
                    <tr>
                      <th style={{ width: 24 }} />
                      {result.columns.map((c) => (
                        <th key={c.name} style={{ textAlign: "left", padding: "4px 8px", borderBottom: "1px solid var(--border-default)", position: "sticky", top: 0, background: "var(--bg-surface)" }}>
                          {c.name}
                          {pkColumns.includes(c.name) ? " 🔑" : ""}
                        </th>
                      ))}
                    </tr>
                  </thead>
                  <tbody>
                    {result.rows.map((row, rowIndex) => (
                      <tr key={rowIndex} style={{ background: selectedRows.has(rowIndex) ? "var(--bg-selected)" : undefined }}>
                        <td style={{ textAlign: "center" }}>
                          <input
                            type="checkbox"
                            checked={selectedRows.has(rowIndex)}
                            onChange={(e) =>
                              setSelectedRows((cur) => {
                                const next = new Set(cur);
                                if (e.target.checked) next.add(rowIndex);
                                else next.delete(rowIndex);
                                return next;
                              })
                            }
                          />
                        </td>
                        {row.map((cell, colIndex) => {
                          const isEditing = editing?.row === rowIndex && editing.col === colIndex;
                          const isPk = pkColumns.includes(result.columns[colIndex].name);
                          return (
                            <td
                              key={colIndex}
                              style={{ padding: "2px 8px", borderBottom: "1px solid var(--border-subtle)", cursor: canEdit && !isPk && !cell.is_binary ? "text" : "default" }}
                              onDoubleClick={() => {
                                if (!canEdit || isPk || cell.is_binary) return;
                                setEditing({ row: rowIndex, col: colIndex });
                                setEditValue(cell.is_null ? "" : cell.text);
                              }}
                            >
                              {isEditing ? (
                                <input
                                  autoFocus
                                  className="form-input"
                                  style={{ height: 22, fontSize: 12 }}
                                  value={editValue}
                                  onChange={(e) => setEditValue(e.target.value)}
                                  onBlur={() => void commitEdit(rowIndex, colIndex)}
                                  onKeyDown={(e) => {
                                    if (e.key === "Enter") void commitEdit(rowIndex, colIndex);
                                    if (e.key === "Escape") setEditing(null);
                                  }}
                                />
                              ) : (
                                <span style={{ color: cell.is_null ? "var(--text-secondary)" : undefined, fontStyle: cell.is_null ? "italic" : undefined }}>
                                  {cell.is_null ? "NULL" : cell.text}
                                </span>
                              )}
                            </td>
                          );
                        })}
                      </tr>
                    ))}
                  </tbody>
                </table>
              )}
            </div>
          </div>
        ) : (
          <div style={{ flex: 1, minHeight: 0, overflow: "auto", padding: "0 12px 12px" }}>
            {def?.comment && <p style={{ color: "var(--text-secondary)", fontStyle: "italic" }}>{def.comment}</p>}
            <table style={{ borderCollapse: "collapse", width: "100%", fontSize: 12, marginBottom: 16 }}>
              <thead>
                <tr>
                  {["字段", "类型", "可空", "主键", "注释", ""].map((h) => (
                    <th key={h} style={{ textAlign: "left", padding: "4px 8px", borderBottom: "1px solid var(--border-default)" }}>{h}</th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {def?.columns.map((c) => (
                  <tr key={c.name}>
                    <td style={{ padding: "3px 8px" }}>{c.name}</td>
                    <td style={{ padding: "3px 8px" }}>{c.data_type}</td>
                    <td style={{ padding: "3px 8px" }}>{c.nullable ? "是" : "否"}</td>
                    <td style={{ padding: "3px 8px" }}>{c.is_primary_key ? "✓" : ""}</td>
                    <td style={{ padding: "3px 8px", color: "var(--text-secondary)" }}>{c.comment ?? ""}</td>
                    <td style={{ padding: "3px 8px", whiteSpace: "nowrap" }}>
                      <button className="btn ghost sm" onClick={() => void generateRename(c)}>重命名</button>{" "}
                      <button className="btn danger sm" onClick={() => void generateDrop(c)}>删除</button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>

            <div className="form" style={{ maxWidth: 420 }}>
              <div style={{ fontSize: 13, fontWeight: 600 }}>添加字段</div>
              <div className="form-row" style={{ flexDirection: "row", gap: 8 }}>
                <input className="form-input" placeholder="字段名" value={newColumn.name} onChange={(e) => setNewColumn((c) => ({ ...c, name: e.target.value }))} />
                <input className="form-input" placeholder="类型，如 VARCHAR(255)" value={newColumn.data_type} onChange={(e) => setNewColumn((c) => ({ ...c, data_type: e.target.value }))} />
              </div>
              <div className="form-row" style={{ flexDirection: "row", alignItems: "center", gap: 6 }}>
                <input type="checkbox" checked={newColumn.nullable} onChange={(e) => setNewColumn((c) => ({ ...c, nullable: e.target.checked }))} />
                <label className="form-label" style={{ margin: 0 }}>允许 NULL</label>
              </div>
              <div className="form-actions">
                <button className="btn primary sm" onClick={() => void generateAdd()} disabled={!newColumn.name.trim() || !newColumn.data_type.trim()}>
                  生成 ALTER TABLE 语句
                </button>
              </div>
              <p style={{ fontSize: 11, color: "var(--text-secondary)" }}>
                生成的语句会打开一个新 SQL 标签页，需要你确认后再运行——不会直接改表结构。
              </p>
            </div>
          </div>
        )}
      </div>

      <ConfirmDialog open={deleteConfirm} severity="danger" title="删除所选行？" onDismiss={() => setDeleteConfirm(false)}
        actions={<><button className="btn ghost sm" onClick={() => setDeleteConfirm(false)}>取消</button><button className="btn danger sm" onClick={() => void deleteSelected()}>删除</button></>}>
        <p>将删除 {selectedRows.size} 行数据，不可撤销。</p>
      </ConfirmDialog>
    </div>
  );
};
