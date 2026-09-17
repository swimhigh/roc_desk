import React, { useState } from "react";
import { FilePlus2, Folder, FolderPlus, Trash2 } from "lucide-react";
import { useHttpDeskStore } from "../../stores/httpDeskStore";
import { useToastStore } from "../shared/Toast";
import { formatError } from "../../utils/error";
import type { RequestSummary } from "../../types/bindings";

const METHOD_COLORS: Record<string, string> = {
  GET: "#22863a",
  POST: "#b08800",
  PUT: "#0969da",
  PATCH: "#8250df",
  DELETE: "#cf222e",
  HEAD: "#6e7781",
  OPTIONS: "#6e7781",
};

/** 左栏：集合选择 + 当前集合的请求树（docs/HTTP_DESKTOP_PLAN.md §3.3）。文件夹
 * 只读展示（按已有请求的 `folder` 分组），本轮没有做"新建文件夹/拖拽移动请求到
 * 文件夹"的交互——新建请求一律放在集合根目录，folder 分组只用于展示导入/未来
 * 其它渠道产生的、已经带文件夹路径的请求。 */
export const CollectionExplorer: React.FC = () => {
  const collections = useHttpDeskStore((s) => s.collections);
  const activeSlug = useHttpDeskStore((s) => s.activeSlug);
  const requests = useHttpDeskStore((s) => s.requests);
  const selectCollection = useHttpDeskStore((s) => s.selectCollection);
  const createCollection = useHttpDeskStore((s) => s.createCollection);
  const deleteCollection = useHttpDeskStore((s) => s.deleteCollection);
  const createRequest = useHttpDeskStore((s) => s.createRequest);
  const deleteRequest = useHttpDeskStore((s) => s.deleteRequest);
  const openRequestTab = useHttpDeskStore((s) => s.openRequestTab);
  const activeTabId = useHttpDeskStore((s) => s.activeTabId);
  const tabs = useHttpDeskStore((s) => s.tabs);
  const push = useToastStore((s) => s.push);

  const [newCollectionName, setNewCollectionName] = useState("");
  const [showNewCollection, setShowNewCollection] = useState(false);

  const activeRequestId = tabs.find((t) => t.id === activeTabId)?.request_id;

  const groups = new Map<string, RequestSummary[]>();
  for (const r of requests) {
    const key = r.folder.join("/");
    const list = groups.get(key) ?? [];
    list.push(r);
    groups.set(key, list);
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%", minWidth: 0 }}>
      <div style={{ padding: 8, borderBottom: "1px solid var(--border-default)" }}>
        <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
          <select
            className="form-select"
            style={{ flex: 1, minWidth: 0 }}
            value={activeSlug ?? ""}
            onChange={(e) => void selectCollection(e.target.value)}
          >
            {collections.length === 0 && <option value="">暂无集合</option>}
            {collections.map((c) => (
              <option key={c.slug} value={c.slug}>
                {c.name}（{c.request_count}）
              </option>
            ))}
          </select>
          <button className="btn ghost sm" title="新建集合" onClick={() => setShowNewCollection(true)}>
            <FolderPlus size={14} />
          </button>
          {activeSlug && (
            <button
              className="btn ghost sm"
              title="删除当前集合"
              onClick={() => {
                if (confirm(`删除集合"${collections.find((c) => c.slug === activeSlug)?.name}"？此操作不可撤销。`)) {
                  void deleteCollection(activeSlug).catch((e) => push("error", `删除集合失败：${formatError(e)}`));
                }
              }}
            >
              <Trash2 size={14} />
            </button>
          )}
        </div>
        {showNewCollection && (
          <div style={{ display: "flex", gap: 6, marginTop: 6 }}>
            <input
              className="form-input"
              style={{ flex: 1 }}
              autoFocus
              placeholder="集合名称"
              value={newCollectionName}
              onChange={(e) => setNewCollectionName(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && newCollectionName.trim()) {
                  void createCollection(newCollectionName.trim())
                    .then(() => {
                      setNewCollectionName("");
                      setShowNewCollection(false);
                    })
                    .catch((err) => push("error", `新建集合失败：${formatError(err)}`));
                } else if (e.key === "Escape") {
                  setShowNewCollection(false);
                }
              }}
            />
            <button
              className="btn primary sm"
              disabled={!newCollectionName.trim()}
              onClick={() => {
                void createCollection(newCollectionName.trim())
                  .then(() => {
                    setNewCollectionName("");
                    setShowNewCollection(false);
                  })
                  .catch((err) => push("error", `新建集合失败：${formatError(err)}`));
              }}
            >
              创建
            </button>
          </div>
        )}
      </div>

      {activeSlug && (
        <div style={{ padding: "6px 8px" }}>
          <button
            className="btn ghost sm"
            style={{ width: "100%", justifyContent: "flex-start" }}
            onClick={() => {
              const name = prompt("请求名称", "新建请求");
              if (name?.trim()) {
                void createRequest(name.trim()).catch((e) => push("error", `新建请求失败：${formatError(e)}`));
              }
            }}
          >
            <FilePlus2 size={13} /> 新建请求
          </button>
        </div>
      )}

      <div style={{ flex: 1, minHeight: 0, overflowY: "auto", padding: "0 4px 8px" }}>
        {requests.length === 0 && (
          <div className="empty-state" style={{ padding: 16, fontSize: 13, opacity: 0.7 }}>
            {activeSlug ? "还没有请求，点上方新建" : "先新建一个集合"}
          </div>
        )}
        {[...groups.entries()].map(([folderKey, items]) => (
          <div key={folderKey || "__root__"}>
            {folderKey && (
              <div style={{ display: "flex", alignItems: "center", gap: 4, padding: "4px 6px", opacity: 0.65, fontSize: 12 }}>
                <Folder size={12} /> {folderKey}
              </div>
            )}
            {items.map((r) => (
              <div
                key={r.id}
                onClick={() => void openRequestTab(r.id, r.name)}
                className={`home-dashboard-card-item ${activeRequestId === r.id ? "active" : ""}`}
                style={{ cursor: "pointer", paddingLeft: folderKey ? 20 : undefined }}
              >
                <span style={{ color: METHOD_COLORS[r.method] ?? "#6e7781", fontWeight: 700, fontSize: 11, width: 44, flexShrink: 0 }}>
                  {r.method}
                </span>
                <span className="home-dashboard-item-name" title={r.name}>{r.name}</span>
                <button
                  className="btn ghost sm"
                  title="删除请求"
                  style={{ marginLeft: "auto", padding: "2px 4px" }}
                  onClick={(e) => {
                    e.stopPropagation();
                    if (confirm(`删除请求"${r.name}"？`)) {
                      void deleteRequest(r.id).catch((err) => push("error", `删除请求失败：${formatError(err)}`));
                    }
                  }}
                >
                  <Trash2 size={12} />
                </button>
              </div>
            ))}
          </div>
        ))}
      </div>
    </div>
  );
};
