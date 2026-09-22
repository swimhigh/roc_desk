import React, { useState } from "react";
import { open as openFileDialog, save as saveFileDialog } from "@tauri-apps/plugin-dialog";
import { FilePlus2, Folder, FolderPlus, Trash2, FileJson, FileCode2, Download } from "lucide-react";
import { useHttpDeskStore } from "../../stores/httpDeskStore";
import { useToastStore } from "../shared/Toast";
import { formatError } from "../../utils/error";
import { httpMethodClass } from "../../utils/httpMethod";
import { localFileService } from "../../services/fsService";
import type { RequestSummary } from "../../types/bindings";

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
  const importPostmanCollection = useHttpDeskStore((s) => s.importPostmanCollection);
  const importOpenApiSpec = useHttpDeskStore((s) => s.importOpenApiSpec);
  const exportCurrentCollection = useHttpDeskStore((s) => s.exportCurrentCollection);
  const push = useToastStore((s) => s.push);

  const [newCollectionName, setNewCollectionName] = useState("");
  const [showNewCollection, setShowNewCollection] = useState(false);
  const [newRequestName, setNewRequestName] = useState("");
  const [showNewRequest, setShowNewRequest] = useState(false);
  const [importing, setImporting] = useState(false);

  const activeCollectionName = collections.find((c) => c.slug === activeSlug)?.name ?? "collection";

  const handleImportPostman = async () => {
    const path = await openFileDialog({
      multiple: false,
      filters: [{ name: "Postman Collection (.json)", extensions: ["json"] }],
    });
    if (!path || Array.isArray(path)) return;
    setImporting(true);
    try {
      const content = await localFileService.readFile(path);
      await importPostmanCollection(content.text);
      push("success", "Postman 集合导入成功");
    } catch (e) {
      push("error", `导入失败：${formatError(e)}`);
    } finally {
      setImporting(false);
    }
  };

  const handleImportOpenApi = async () => {
    const path = await openFileDialog({
      multiple: false,
      filters: [{ name: "OpenAPI (.json/.yaml/.yml)", extensions: ["json", "yaml", "yml"] }],
    });
    if (!path || Array.isArray(path)) return;
    setImporting(true);
    try {
      const content = await localFileService.readFile(path);
      await importOpenApiSpec(content.text);
      push("success", "OpenAPI 规范导入成功");
    } catch (e) {
      push("error", `导入失败：${formatError(e)}`);
    } finally {
      setImporting(false);
    }
  };

  const handleExportPostman = async () => {
    try {
      const json = await exportCurrentCollection();
      const path = await saveFileDialog({
        defaultPath: `${activeCollectionName}.postman_collection.json`,
        filters: [{ name: "Postman Collection (.json)", extensions: ["json"] }],
      });
      if (!path) return;
      await localFileService.writeFile(path, json, null);
      push("success", `已导出到 ${path}`);
    } catch (e) {
      push("error", `导出失败：${formatError(e)}`);
    }
  };

  const activeRequestId = tabs.find((t) => t.id === activeTabId)?.request_id;

  const groups = new Map<string, RequestSummary[]>();
  for (const r of requests) {
    const key = r.folder.join("/");
    const list = groups.get(key) ?? [];
    list.push(r);
    groups.set(key, list);
  }

  const submitNewRequest = () => {
    const name = newRequestName.trim();
    if (!name) return;
    void createRequest(name)
      .then(() => {
        setNewRequestName("");
        setShowNewRequest(false);
      })
      .catch((e) => push("error", `新建请求失败：${formatError(e)}`));
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%", minWidth: 0 }}>
      <div className="http-collection-toolbar">
        <select className="form-select" value={activeSlug ?? ""} onChange={(e) => void selectCollection(e.target.value)}>
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
        <button className="btn ghost sm" title="从 Postman Collection (.json) 导入为新集合" disabled={importing} onClick={() => void handleImportPostman()}>
          <FileJson size={14} />
        </button>
        <button className="btn ghost sm" title="从 OpenAPI 规范 (.json/.yaml) 导入为新集合" disabled={importing} onClick={() => void handleImportOpenApi()}>
          <FileCode2 size={14} />
        </button>
        {activeSlug && (
          <button className="btn ghost sm" title="导出当前集合为 Postman Collection" onClick={() => void handleExportPostman()}>
            <Download size={14} />
          </button>
        )}
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
        <div style={{ display: "flex", gap: 6, padding: "0 var(--space-2) var(--space-2)" }}>
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

      {activeSlug && (
        <div style={{ padding: "var(--space-2)", borderBottom: "1px solid var(--border-default)" }}>
          {showNewRequest ? (
            <div style={{ display: "flex", gap: 6 }}>
              <input
                className="form-input"
                style={{ flex: 1 }}
                autoFocus
                placeholder="请求名称"
                value={newRequestName}
                onChange={(e) => setNewRequestName(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") submitNewRequest();
                  else if (e.key === "Escape") setShowNewRequest(false);
                }}
                onBlur={() => !newRequestName.trim() && setShowNewRequest(false)}
              />
              <button className="btn primary sm" disabled={!newRequestName.trim()} onClick={submitNewRequest}>
                创建
              </button>
            </div>
          ) : (
            <button
              className="btn ghost sm"
              style={{ width: "100%", justifyContent: "flex-start" }}
              onClick={() => {
                setNewRequestName("新建请求");
                setShowNewRequest(true);
              }}
            >
              <FilePlus2 size={13} /> 新建请求
            </button>
          )}
        </div>
      )}

      <div className="http-tree-scroll">
        {requests.length === 0 && (
          <div className="empty-state" style={{ padding: 16, fontSize: 13 }}>
            {activeSlug ? "还没有请求，点上方新建" : "先新建一个集合"}
          </div>
        )}
        {[...groups.entries()].map(([folderKey, items]) => (
          <div key={folderKey || "__root__"}>
            {folderKey && (
              <div className="http-tree-folder">
                <Folder size={12} /> {folderKey}
              </div>
            )}
            {items.map((r) => (
              <div
                key={r.id}
                onClick={() => void openRequestTab(r.id, r.name)}
                className={`http-tree-item ${activeRequestId === r.id ? "active" : ""}`}
                style={{ paddingLeft: folderKey ? 28 : undefined }}
              >
                <span className={`http-method-chip ${httpMethodClass(r.method)}`}>{r.method}</span>
                <span className="http-tree-name" title={r.name}>
                  {r.name}
                </span>
                <button
                  className="http-tree-delete"
                  title="删除请求"
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
