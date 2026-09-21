import React from "react";
import { FilePlus2 } from "lucide-react";
import { useSqlStore } from "../../stores/sqlStore";
import { ObjectExplorer } from "./ObjectExplorer";
import { SqlEditorTabs } from "./SqlEditorTabs";
import { ResultPanel } from "./ResultPanel/ResultPanel";
import { SqlAgentPanel } from "./SqlAgentPanel";
import { TableDataDialog } from "./TableDataDialog";
import { TransferDialog } from "./TransferDialog";
import type { DataSourceProfile, ObjectRef } from "../../types/bindings";
import { sqlService } from "../../services/sqlService";

interface SqlWorkspaceProps {
  dataSource: DataSourceProfile;
}

/** SQL 工作区三栏布局（docs/SQL_DESKTOP_PLAN.md §3.3）：对象浏览器 | SQL
 * 标签页编辑器 + 结果区 | AI 工具。列宽固定，不做拖拽调整——控制这一版的
 * 实现规模，后续需要再加。 */
export const SqlWorkspace: React.FC<SqlWorkspaceProps> = ({ dataSource }) => {
  const activeTabId = useSqlStore((s) => s.activeTabId);
  const runSql = useSqlStore((s) => s.runSql);
  const cancelQuery = useSqlStore((s) => s.cancelQuery);

  const [layout, setLayout] = React.useState(() => {
    // AI 工具默认收起（2026-09 用户反馈：要和"工作区"模式一样，默认收起、
    // 要用时再点开——`App.tsx` 里 `aiToolsOpen` 的初始值就是 `false`）。
    const defaults = { ai: false, tree: true, left: 280, right: 300, result: 40 };
    try { return { ...defaults, ...JSON.parse(localStorage.getItem("sql-layout") ?? "{}") }; } catch { return defaults; }
  });
  const [maxResult, setMaxResult] = React.useState(false);
  React.useEffect(() => { localStorage.setItem("sql-layout", JSON.stringify(layout)); }, [layout]);
  const resize = (key: "left" | "right" | "result") => (event: React.PointerEvent<HTMLDivElement>) => {
    event.currentTarget.setPointerCapture(event.pointerId);
    const startX = event.clientX, startY = event.clientY, initial = layout[key];
    event.currentTarget.onpointermove = (e) => setLayout((l: typeof layout) => ({ ...l, [key]: key === "result" ? Math.max(20, Math.min(80, initial - (e.clientY - startY) / window.innerHeight * 100)) : Math.max(key === "right" ? 320 : 180, Math.min(key === "right" ? 760 : 480, initial + (e.clientX - startX) * (key === "right" ? -1 : 1))) }));
    event.currentTarget.onpointerup = (e) => { const el = e.currentTarget as HTMLDivElement; el.onpointermove = null; el.releasePointerCapture(e.pointerId); };
  };
  const [viewDataObject, setViewDataObject] = React.useState<ObjectRef | null>(null);
  const [transfer, setTransfer] = React.useState<{ mode: "export" | "import"; object: ObjectRef } | null>(null);

  const openSqlInNewTab = async (sql: string, title: string) => {
    const s = useSqlStore.getState();
    await s.createTab(title);
    const id = useSqlStore.getState().activeTabId;
    if (id) s.setTabContent(id, sql);
  };

  const handleInsertSelect = (object: ObjectRef) => {
    void (async () => {
      const snippet = await sqlService.previewTemplate(dataSource.id, object);
      const s = useSqlStore.getState();
      if (s.currentDataSourceId !== dataSource.id) return;
      await s.createTab(object.name);
      const id = useSqlStore.getState().activeTabId;
      if (id) {
        s.setTabContent(id, snippet);
        s.requestSelectAll(id);
      }
    })().catch((e) => useSqlStore.setState({ error: String(e) }));
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", width: "100%", minWidth: 0 }}>
      <div style={{ display: "flex", alignItems: "center", gap: 8, padding: 4 }}>
        <button className="btn ghost sm" onClick={() => setLayout({ ...layout, tree: !layout.tree })}>对象树</button>
        <button className="btn ghost sm" title="新建一个空白 SQL 查询标签页" onClick={() => void useSqlStore.getState().createTab(`查询 ${useSqlStore.getState().tabs.length + 1}`)}>
          <FilePlus2 size={13} /> 新建 SQL 编辑器
        </button>
        <button className={`btn ghost sm ${layout.ai ? "active" : ""}`} style={{ marginLeft: "auto" }} onClick={() => setLayout({ ...layout, ai: !layout.ai })}>AI 工具</button>
      </div>
      <div style={{ display: "flex", flex: 1, minHeight: 0 }}>
      {layout.tree && <><div style={{ width: layout.left, flexShrink: 0, borderRight: "1px solid var(--border-default)", overflow: "hidden" }}>
        <ObjectExplorer
          dataSourceId={dataSource.id}
          onInsertSelect={handleInsertSelect}
          onViewData={(object) => setViewDataObject(object)}
          onExport={(object) => setTransfer({ mode: "export", object })}
          onImport={(object) => setTransfer({ mode: "import", object })}
        />
      </div><div title="拖动调整对象树宽度" onPointerDown={resize("left")} style={{ width: 5, cursor: "col-resize", touchAction: "none" }} /></>}
      <div style={{ flex: 1, minWidth: 0, display: "flex", flexDirection: "column", overflow: "hidden" }}>
        <div style={{ flex: 1, minHeight: 0, display: maxResult ? "none" : undefined }}>
          <SqlEditorTabs
            onRun={(sql) => activeTabId && void runSql(activeTabId, sql, false)}
            onCancel={() => activeTabId && void cancelQuery(activeTabId)}
          />
        </div>
        {!maxResult && <div title="拖动调整结果区高度" onPointerDown={resize("result")} style={{ height: 5, cursor: "row-resize", touchAction: "none" }} />}
        <div style={{ height: maxResult ? "100%" : `${layout.result}%`, minHeight: 100, borderTop: "1px solid var(--border-default)" }}>
          <ResultPanel maximized={maxResult} onToggleMaximized={() => setMaxResult(!maxResult)} />
        </div>
      </div>
      {layout.ai && <><div title="拖动调整 AI 宽度" onPointerDown={resize("right")} style={{ width: 5, cursor: "col-resize", touchAction: "none" }} /><div style={{ width: layout.right, minWidth: 320, maxWidth: 760, flexShrink: 0, borderLeft: "1px solid var(--border-default)", overflow: "hidden", display: "flex", flexDirection: "column" }}>
        <SqlAgentPanel dataSourceId={dataSource.id} />
      </div></>}
      </div>
      {viewDataObject && (
        <TableDataDialog
          dataSourceId={dataSource.id}
          object={viewDataObject}
          readonly={dataSource.readonly}
          onClose={() => setViewDataObject(null)}
          onOpenAlterSql={(sql) => {
            setViewDataObject(null);
            void openSqlInNewTab(sql, `修改 ${viewDataObject.name}`);
          }}
        />
      )}
      {transfer && (
        <TransferDialog
          dataSourceId={dataSource.id}
          mode={transfer.mode}
          object={transfer.object}
          onClose={() => setTransfer(null)}
        />
      )}
    </div>
  );
};
