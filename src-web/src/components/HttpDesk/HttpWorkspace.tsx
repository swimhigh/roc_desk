import React, { useEffect, useState } from "react";
import { History, Import, Settings2, X } from "lucide-react";
import { useHttpDeskStore } from "../../stores/httpDeskStore";
import { useToastStore } from "../shared/Toast";
import { formatError } from "../../utils/error";
import { httpMethodClass } from "../../utils/httpMethod";
import { CollectionExplorer } from "./CollectionExplorer";
import { RequestEditor } from "./RequestEditor";
import { ResponsePanel } from "./ResponsePanel";
import { EnvironmentDialog } from "./EnvironmentDialog";

/** HTTP 工作区布局（docs/HTTP_DESKTOP_PLAN.md §3.3）：左栏请求树 | 环境选择器 +
 * 打开的请求标签页 + 请求编辑区/响应区上下分栏 | 可选的历史侧栏。没有做 AI 工具栏
 * （右栏）——本轮没有接入 AI，见 http_desk/mod.rs 顶部范围说明；也没有做
 * 上下分栏的拖拽调整，响应区固定占 40% 高度，先把主链路做对，细节交互留待后续。 */
export const HttpWorkspace: React.FC<{ workspaceId: string }> = ({ workspaceId: _workspaceId }) => {
  const tabs = useHttpDeskStore((s) => s.tabs);
  const activeTabId = useHttpDeskStore((s) => s.activeTabId);
  const setActiveTab = useHttpDeskStore((s) => s.setActiveTab);
  const closeTab = useHttpDeskStore((s) => s.closeTab);
  const environments = useHttpDeskStore((s) => s.environments);
  const activeEnvironmentId = useHttpDeskStore((s) => s.activeEnvironmentId);
  const setActiveEnvironment = useHttpDeskStore((s) => s.setActiveEnvironment);
  const importCurl = useHttpDeskStore((s) => s.importCurl);
  const history = useHttpDeskStore((s) => s.history);
  const loadHistory = useHttpDeskStore((s) => s.loadHistory);
  const push = useToastStore((s) => s.push);

  const [showEnvDialog, setShowEnvDialog] = useState(false);
  const [showHistory, setShowHistory] = useState(false);
  const [showImport, setShowImport] = useState(false);
  const [curlText, setCurlText] = useState("");
  const [importing, setImporting] = useState(false);

  useEffect(() => {
    if (showHistory) void loadHistory();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [showHistory]);

  const activeTab = tabs.find((t) => t.id === activeTabId);
  const dirtyMap = useHttpDeskStore((s) => s.dirty);
  const drafts = useHttpDeskStore((s) => s.drafts);

  return (
    <div style={{ display: "flex", width: "100%", minWidth: 0 }}>
      <div style={{ width: 260, flexShrink: 0, borderRight: "1px solid var(--border-default)", overflow: "hidden" }}>
        <CollectionExplorer />
      </div>
      <div style={{ flex: 1, minWidth: 0, display: "flex", flexDirection: "column" }}>
        <div className="http-toolbar">
          <select
            className="form-select"
            style={{ width: 170, flex: "0 0 auto" }}
            value={activeEnvironmentId ?? ""}
            onChange={(e) => setActiveEnvironment(e.target.value || null)}
            title="当前环境——决定 {{变量}} 怎么解析"
          >
            <option value="">无环境</option>
            {environments.map((env) => (
              <option key={env.id} value={env.id}>
                {env.name}
              </option>
            ))}
          </select>
          <button className="btn ghost sm" onClick={() => setShowEnvDialog(true)}>
            <Settings2 size={13} /> 环境管理
          </button>
          <button className="btn ghost sm" onClick={() => setShowImport(true)}>
            <Import size={13} /> 导入 curl
          </button>
          <button className={`btn ghost sm ${showHistory ? "active" : ""}`} style={{ marginLeft: "auto" }} onClick={() => setShowHistory(!showHistory)}>
            <History size={13} /> 历史
          </button>
        </div>
        <div className="editor-tabs">
          {tabs.map((t) => {
            const isDirty = dirtyMap[t.request_id];
            const method = drafts[t.request_id]?.method;
            return (
              <div key={t.id} className={`editor-tab ${t.id === activeTabId ? "active" : ""}`} onClick={() => setActiveTab(t.id)}>
                {method && <span className={httpMethodClass(method)} style={{ fontSize: 11, fontWeight: 700 }}>{method}</span>}
                <span>{t.title}</span>
                {isDirty && <span className="dirty-dot" title="有未保存的改动" />}
                <span
                  className="editor-tab-close"
                  onClick={(e) => {
                    e.stopPropagation();
                    void closeTab(t.id);
                  }}
                >
                  <X />
                </span>
              </div>
            );
          })}
          {tabs.length === 0 && <div style={{ padding: "0 var(--space-3)", color: "var(--text-secondary)", fontSize: 12.5, display: "flex", alignItems: "center" }}>暂无打开的请求，从左侧新建或选择一个</div>}
        </div>
        <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
          {activeTab ? (
            <>
              <div style={{ flex: 1, minHeight: 0 }}>
                <RequestEditor requestId={activeTab.request_id} />
              </div>
              <div style={{ height: "40%", flexShrink: 0, borderTop: "1px solid var(--border-default)" }}>
                <ResponsePanel />
              </div>
            </>
          ) : (
            <div className="empty-state" style={{ padding: 24 }}>
              从左侧选择或新建一个请求
            </div>
          )}
        </div>
      </div>
      {showHistory && (
        <div style={{ width: 300, flexShrink: 0, borderLeft: "1px solid var(--border-default)", display: "flex", flexDirection: "column" }}>
          <div style={{ padding: "var(--space-2)", fontWeight: 600, fontSize: 13, borderBottom: "1px solid var(--border-default)" }}>请求历史</div>
          <div style={{ flex: 1, minHeight: 0, overflowY: "auto" }}>
            {history.map((h) => (
              <div key={h.id} className="http-tree-item" style={{ height: 40, cursor: "default" }}>
                <span className={`http-method-chip ${httpMethodClass(h.method)}`}>{h.method}</span>
                <span style={{ display: "flex", flexDirection: "column", minWidth: 0, gap: 2 }}>
                  <span className="http-tree-name" title={h.url} style={{ fontSize: 12.5 }}>
                    {h.url}
                  </span>
                  <span style={{ fontSize: 11, color: "var(--text-secondary)" }}>
                    {h.status_code ?? "请求失败"} · {h.duration_ms ?? "-"} ms
                  </span>
                </span>
              </div>
            ))}
            {history.length === 0 && (
              <div className="empty-state" style={{ padding: 16 }}>
                暂无历史
              </div>
            )}
          </div>
        </div>
      )}
      {showEnvDialog && <EnvironmentDialog onClose={() => setShowEnvDialog(false)} />}
      {showImport && (
        <div
          style={{
            position: "fixed",
            inset: 0,
            background: "color-mix(in srgb, var(--bg-app) 60%, transparent)",
            display: "grid",
            placeItems: "center",
            zIndex: 20,
          }}
        >
          <div className="dialog" style={{ width: 520, padding: 16 }}>
            <h3 style={{ marginTop: 0 }}>从 curl 命令导入</h3>
            <textarea
              className="form-input"
              style={{ width: "100%", minHeight: 140, fontFamily: "monospace", resize: "vertical" }}
              value={curlText}
              onChange={(e) => setCurlText(e.target.value)}
              placeholder="curl https://api.example.com/users -H 'Authorization: Bearer xxx'"
            />
            <div className="dialog-actions">
              <button className="btn ghost sm" onClick={() => setShowImport(false)}>
                取消
              </button>
              <button
                className="btn primary sm"
                disabled={!curlText.trim() || importing}
                onClick={() => {
                  setImporting(true);
                  void importCurl(curlText)
                    .then(() => {
                      setCurlText("");
                      setShowImport(false);
                    })
                    .catch((e) => push("error", `导入失败：${formatError(e)}`))
                    .finally(() => setImporting(false));
                }}
              >
                {importing ? "导入中…" : "导入"}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
};
