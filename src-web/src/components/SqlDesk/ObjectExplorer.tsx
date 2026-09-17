import React, { useMemo, useState } from "react";
import { ChevronDown, ChevronRight, Database, Loader2, RefreshCw, Table2, TextCursorInput, Eye } from "lucide-react";
import { open as openDirDialog } from "@tauri-apps/plugin-dialog";
import { useSqlStore } from "../../stores/sqlStore";
import { sqlService } from "../../services/sqlService";
import { useToastStore } from "../shared/Toast";
import { formatError } from "../../utils/error";
import { ContextMenu, type ContextMenuItem } from "../shared/ContextMenu";
import type { ObjectDefinition, ObjectRef } from "../../types/bindings";

interface ObjectExplorerProps {
  dataSourceId: string;
  onInsertSelect: (object: ObjectRef) => void;
  onViewData: (object: ObjectRef) => void;
  onExport: (object: ObjectRef) => void;
  onImport: (object: ObjectRef) => void;
}

/** 轮询单个导出任务直到完成/失败——批量导出（右键 Schema/数据库）用这个
 * 顺序等每张表导出完再导下一张，不需要为批量场景另外维护一套并发进度 UI。 */
async function waitForExport(exportId: string): Promise<void> {
  for (;;) {
    const p = await sqlService.exportPoll(exportId);
    if (p.done || p.cancelled || p.error) {
      if (p.error) throw new Error(p.error);
      return;
    }
    await new Promise((r) => setTimeout(r, 400));
  }
}

type DefinitionState = "loading" | ObjectDefinition | { error: string };

function objectKey(object: ObjectRef): string {
  return `${object.schema}.${object.name}`;
}

/** 拖拽表/字段到 SQL 编辑器时携带的载荷——`SqlEditorTabs` 的 `onDrop` 解析这个
 * 私有 MIME 类型；同时总是带一份 `text/plain` 兜底，拖去任何认识纯文本拖放的
 * 地方（比如系统外部编辑器）也能拿到一个可用的名字。 */
export const SQL_DRAG_MIME = "application/x-roc-sql-ref";
const DRAG_MIME = SQL_DRAG_MIME;

export type SqlDragPayload =
  | { kind: "table"; schema: string; name: string }
  | { kind: "column"; schema: string; table: string; column: string };

function makeDragHandlers(payload: SqlDragPayload, plainText: string) {
  return {
    draggable: true,
    onDragStart: (e: React.DragEvent) => {
      e.dataTransfer.setData(DRAG_MIME, JSON.stringify(payload));
      e.dataTransfer.setData("text/plain", plainText);
      e.dataTransfer.effectAllowed = "copy";
    },
  };
}

/** 左栏对象浏览器（docs/SQL_DESKTOP_PLAN.md §3.3，2026-09 用户反馈后重做）：
 * 数据库 → Schema → 表三层，默认全部折叠；单击表名原地展开显示表注释和字段
 * 元数据（不是弹一个 toast），双击/点插入按钮生成 SELECT 模板。 */
export const ObjectExplorer: React.FC<ObjectExplorerProps> = ({
  dataSourceId,
  onInsertSelect,
  onViewData,
  onExport,
  onImport,
}) => {
  const databases = useSqlStore((s) => s.databases);
  const currentDatabase = useSqlStore((s) => s.currentDatabase);
  const switchDatabase = useSqlStore((s) => s.switchDatabase);
  const objects = useSqlStore((s) => s.objects);
  const objectsLoading = useSqlStore((s) => s.objectsLoading);
  const refreshObjects = useSqlStore((s) => s.refreshObjects);
  const push = useToastStore((s) => s.push);

  const [expandedSchemas, setExpandedSchemas] = useState<Set<string>>(new Set());
  const [expandedTables, setExpandedTables] = useState<Record<string, DefinitionState>>({});
  const [filter, setFilter] = useState("");
  const [dbListOpen, setDbListOpen] = useState(false);
  const [menu, setMenu] = useState<{ x: number; y: number; items: ContextMenuItem[] } | null>(null);

  const copyText = (text: string, label: string) => {
    navigator.clipboard
      .writeText(text)
      .then(() => push("success", `已复制${label}`))
      .catch((e) => push("error", `复制失败：${formatError(e)}`));
  };

  const exportGroup = async (tables: ObjectRef[], label: string) => {
    const dir = await openDirDialog({ directory: true });
    if (!dir || typeof dir !== "string") return;
    push("success", `开始导出${label}下 ${tables.length} 张表到 ${dir}`);
    for (const [i, table] of tables.entries()) {
      try {
        const path = `${dir}/${table.schema}.${table.name}.csv`.replace(/\\/g, "/");
        const id = await sqlService.exportStart(dataSourceId, table, "csv", path, false);
        await waitForExport(id);
      } catch (e) {
        push("error", `导出 ${table.schema}.${table.name} 失败：${formatError(e)}，已跳过继续`);
      }
      if (i === tables.length - 1) push("success", `${label}导出完成`);
    }
  };

  const forceDescribe = (object: ObjectRef) => {
    const key = objectKey(object);
    setExpandedTables((cur) => ({ ...cur, [key]: "loading" }));
    sqlService
      .describeObject(dataSourceId, object)
      .then((def) => setExpandedTables((cur) => ({ ...cur, [key]: def })))
      .catch((e) => setExpandedTables((cur) => ({ ...cur, [key]: { error: formatError(e) } })));
  };

  const grouped = useMemo(() => {
    const map = new Map<string, ObjectRef[]>();
    const needle = filter.trim().toLowerCase();
    for (const obj of objects) {
      if (needle && !obj.name.toLowerCase().includes(needle)) continue;
      const list = map.get(obj.schema) ?? [];
      list.push(obj);
      map.set(obj.schema, list);
    }
    return Array.from(map.entries()).sort(([a], [b]) => a.localeCompare(b));
  }, [objects, filter]);

  const toggleSchema = (schema: string) => {
    setExpandedSchemas((cur) => {
      const next = new Set(cur);
      if (next.has(schema)) next.delete(schema);
      else next.add(schema);
      return next;
    });
  };

  const toggleTable = (object: ObjectRef) => {
    const key = objectKey(object);
    setExpandedTables((cur) => {
      if (key in cur) {
        const { [key]: _removed, ...rest } = cur;
        return rest;
      }
      return { ...cur, [key]: "loading" };
    });
    if (!(key in expandedTables)) {
      sqlService
        .describeObject(dataSourceId, object)
        .then((def) => setExpandedTables((cur) => ({ ...cur, [key]: def })))
        .catch((e) => setExpandedTables((cur) => ({ ...cur, [key]: { error: formatError(e) } })));
    }
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      <div style={{ borderBottom: "1px solid var(--border-subtle)" }}>
        <div
          className="tree-item"
          onClick={() => setDbListOpen((v) => !v)}
          onContextMenu={(e) => {
            e.preventDefault();
            setMenu({
              x: e.clientX,
              y: e.clientY,
              items: [
                { label: "复制数据库名", onClick: () => copyText(currentDatabase ?? "", "数据库名") },
                { label: "刷新数据库列表", onClick: () => void useSqlStore.getState().loadDatabases(), separatorBefore: true },
                { label: "导出整个数据库所有表…", onClick: () => void exportGroup(objects, "整个数据库"), separatorBefore: true },
              ],
            });
          }}
          title="点击查看/切换服务器上的其它数据库，右键更多操作"
        >
          {dbListOpen ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
          <Database size={13} />
          <span style={{ fontWeight: 600 }}>{currentDatabase ?? "（未知数据库）"}</span>
        </div>
        {dbListOpen && (
          <div style={{ maxHeight: 160, overflowY: "auto" }}>
            {databases.map((db) => (
              <div
                key={db}
                className="tree-item"
                style={{ paddingLeft: 28, color: db === currentDatabase ? "var(--accent)" : undefined }}
                onClick={() => {
                  if (db !== currentDatabase) {
                    void switchDatabase(db).catch((e) => push("error", `切换数据库失败：${formatError(e)}`));
                  }
                  setDbListOpen(false);
                }}
                onContextMenu={(e) => {
                  e.preventDefault();
                  setMenu({
                    x: e.clientX,
                    y: e.clientY,
                    items: [
                      ...(db !== currentDatabase
                        ? [{ label: "切换到此数据库", onClick: () => void switchDatabase(db).catch((err: unknown) => push("error", `切换数据库失败：${formatError(err)}`)) }]
                        : []),
                      { label: "复制数据库名", onClick: () => copyText(db, "数据库名") },
                    ],
                  });
                }}
              >
                <Database size={12} />
                <span>{db}</span>
                {db === currentDatabase && <span style={{ marginLeft: "auto", fontSize: 11 }}>当前</span>}
              </div>
            ))}
          </div>
        )}
      </div>

      <div style={{ display: "flex", alignItems: "center", gap: 4, padding: 6, borderBottom: "1px solid var(--border-subtle)" }}>
        <input
          className="form-input"
          style={{ flex: 1, height: 24, fontSize: 12 }}
          placeholder="搜索对象"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
        />
        <button className="btn ghost sm" title="刷新" onClick={() => void refreshObjects()} disabled={objectsLoading}>
          <RefreshCw style={{ width: 13, height: 13 }} className={objectsLoading ? "spin" : ""} />
        </button>
      </div>
      <div className="project-tree" style={{ flex: 1, overflowY: "auto" }}>
        {grouped.length === 0 && !objectsLoading && (
          <div className="empty-state" style={{ padding: 16, fontSize: 12 }}>没有找到对象</div>
        )}
        {grouped.map(([schema, list]) => {
          const isExpanded = expandedSchemas.has(schema);
          return (
            <div key={schema}>
              <div
                className="tree-item"
                onClick={() => toggleSchema(schema)}
                onContextMenu={(e) => {
                  e.preventDefault();
                  setMenu({
                    x: e.clientX,
                    y: e.clientY,
                    items: [
                      { label: "复制 Schema 名", onClick: () => copyText(schema, "Schema 名") },
                      { label: "刷新对象列表", onClick: () => void refreshObjects(), separatorBefore: true },
                      { label: "导出该 Schema 所有表…", onClick: () => void exportGroup(list, schema), separatorBefore: true },
                    ],
                  });
                }}
                style={{ fontWeight: 600 }}
              >
                {isExpanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
                <span>{schema}</span>
                <span style={{ marginLeft: 6, color: "var(--text-secondary)", fontSize: 11 }}>{list.length}</span>
              </div>
              {isExpanded &&
                list.map((obj) => {
                  const key = objectKey(obj);
                  const def = expandedTables[key];
                  return (
                    <div key={key}>
                      <div
                        className="tree-item"
                        style={{ paddingLeft: 24 }}
                        onClick={() => toggleTable(obj)}
                        onDoubleClick={(e) => {
                          e.preventDefault();
                          onInsertSelect(obj);
                        }}
                        onContextMenu={(e) => {
                          e.preventDefault();
                          setMenu({
                            x: e.clientX,
                            y: e.clientY,
                            items: [
                              { label: "生成 SELECT 模板", onClick: () => onInsertSelect(obj) },
                              { label: "查看/编辑数据…", onClick: () => onViewData(obj) },
                              { label: key in expandedTables ? "收起结构" : "查看结构", onClick: () => toggleTable(obj) },
                              { label: "刷新结构", onClick: () => forceDescribe(obj) },
                              { label: "导出数据…", onClick: () => onExport(obj), separatorBefore: true },
                              { label: "导入数据…", onClick: () => onImport(obj) },
                              { label: "复制表名", onClick: () => copyText(obj.name, "表名"), separatorBefore: true },
                              { label: "复制完整名称（Schema.表）", onClick: () => copyText(objectKey(obj), "完整名称") },
                            ],
                          });
                        }}
                        title="单击查看表结构，双击/右键生成 SELECT 模板，可拖到编辑器"
                        {...makeDragHandlers({ kind: "table", schema: obj.schema, name: obj.name }, objectKey(obj))}
                      >
                        {key in expandedTables ? (
                          <ChevronDown size={12} />
                        ) : (
                          <ChevronRight size={12} />
                        )}
                        {obj.kind === "view" || obj.kind === "materialized_view" ? (
                          <Eye size={13} />
                        ) : (
                          <Table2 size={13} />
                        )}
                        <span style={{ overflow: "hidden", textOverflow: "ellipsis" }}>{obj.name}</span>
                        <span
                          style={{ marginLeft: "auto" }}
                          onClick={(e) => {
                            e.stopPropagation();
                            onInsertSelect(obj);
                          }}
                          title="生成 SELECT 模板"
                        >
                          <TextCursorInput size={13} />
                        </span>
                      </div>
                      {key in expandedTables && (
                        <div style={{ paddingLeft: 40, paddingRight: 8, paddingBottom: 8 }}>
                          {def === "loading" && (
                            <div style={{ display: "flex", alignItems: "center", gap: 6, fontSize: 12, color: "var(--text-secondary)" }}>
                              <Loader2 size={12} className="spin" /> 加载中…
                            </div>
                          )}
                          {def && typeof def === "object" && "error" in def && (
                            <div style={{ fontSize: 12, color: "var(--danger)" }}>{def.error}</div>
                          )}
                          {def && typeof def === "object" && "columns" in def && (
                            <div style={{ fontSize: 12 }}>
                              {def.comment && (
                                <div style={{ color: "var(--text-secondary)", marginBottom: 4, fontStyle: "italic" }}>
                                  {def.comment}
                                </div>
                              )}
                              {def.columns.map((c) => (
                                <div
                                  key={c.name}
                                  style={{ display: "flex", gap: 6, padding: "2px 0", borderBottom: "1px solid var(--border-subtle)", cursor: "grab" }}
                                  title={`${c.comment ? c.comment + "\n" : ""}可拖到编辑器插入列名，右键复制`}
                                  onContextMenu={(e) => {
                                    e.preventDefault();
                                    setMenu({
                                      x: e.clientX,
                                      y: e.clientY,
                                      items: [
                                        { label: "复制字段名", onClick: () => copyText(c.name, "字段名") },
                                        { label: "复制 表.字段", onClick: () => copyText(`${obj.name}.${c.name}`, "字段引用") },
                                      ],
                                    });
                                  }}
                                  {...makeDragHandlers(
                                    { kind: "column", schema: obj.schema, table: obj.name, column: c.name },
                                    c.name,
                                  )}
                                >
                                  <span style={{ fontWeight: c.is_primary_key ? 600 : 400 }}>
                                    {c.is_primary_key ? "🔑 " : ""}
                                    {c.name}
                                  </span>
                                  <span style={{ color: "var(--text-secondary)", marginLeft: "auto", flexShrink: 0 }}>
                                    {c.data_type}
                                    {!c.nullable ? " NOT NULL" : ""}
                                  </span>
                                </div>
                              ))}
                              {def.columns.some((c) => c.comment) && (
                                <div style={{ marginTop: 4, color: "var(--text-secondary)" }}>
                                  {def.columns
                                    .filter((c) => c.comment)
                                    .map((c) => (
                                      <div key={c.name}>
                                        {c.name}：{c.comment}
                                      </div>
                                    ))}
                                </div>
                              )}
                            </div>
                          )}
                        </div>
                      )}
                    </div>
                  );
                })}
            </div>
          );
        })}
      </div>
      {menu && <ContextMenu x={menu.x} y={menu.y} items={menu.items} onClose={() => setMenu(null)} />}
    </div>
  );
};
