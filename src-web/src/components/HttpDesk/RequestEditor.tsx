import React, { useState } from "react";
import { useHttpDeskStore } from "../../stores/httpDeskStore";
import { useToastStore } from "../shared/Toast";
import { formatError } from "../../utils/error";
import { KeyValueTable } from "./KeyValueTable";
import type { AuthConfig, RequestBody, RequestDef } from "../../types/bindings";

const METHODS = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];
type SubTab = "params" | "headers" | "body" | "auth";
const SUB_TABS: { id: SubTab; label: string }[] = [
  { id: "params", label: "Params" },
  { id: "headers", label: "Headers" },
  { id: "body", label: "Body" },
  { id: "auth", label: "Auth" },
];

const BODY_TYPES: { value: RequestBody["type"]; label: string }[] = [
  { value: "none", label: "无" },
  { value: "json", label: "JSON" },
  { value: "raw", label: "Raw / XML / Text" },
  { value: "form_url_encoded", label: "x-www-form-urlencoded" },
  { value: "form_data", label: "form-data" },
];

const BodyEditor: React.FC<{ body: RequestBody; onChange: (b: RequestBody) => void }> = ({ body, onChange }) => (
  <div>
    <select
      className="form-select"
      value={body.type}
      onChange={(e) => {
        switch (e.target.value as RequestBody["type"]) {
          case "none":
            onChange({ type: "none" });
            break;
          case "json":
            onChange({ type: "json", content: body.type === "json" ? body.content : "" });
            break;
          case "raw":
            onChange({ type: "raw", content: "", content_type: "text/plain" });
            break;
          case "form_url_encoded":
            onChange({ type: "form_url_encoded", items: [] });
            break;
          case "form_data":
            onChange({ type: "form_data", items: [] });
            break;
        }
      }}
    >
      {BODY_TYPES.map((b) => (
        <option key={b.value} value={b.value}>
          {b.label}
        </option>
      ))}
    </select>
    <div style={{ marginTop: 8 }}>
      {body.type === "json" && (
        <textarea
          className="form-input"
          style={{ width: "100%", minHeight: 220, fontFamily: "monospace", resize: "vertical" }}
          value={body.content}
          onChange={(e) => onChange({ type: "json", content: e.target.value })}
          placeholder='{"key": "value"}'
        />
      )}
      {body.type === "raw" && (
        <>
          <input
            className="form-input"
            style={{ marginBottom: 6, maxWidth: 320 }}
            value={body.content_type}
            onChange={(e) => onChange({ type: "raw", content: body.content, content_type: e.target.value })}
            placeholder="Content-Type"
          />
          <textarea
            className="form-input"
            style={{ width: "100%", minHeight: 200, fontFamily: "monospace", resize: "vertical" }}
            value={body.content}
            onChange={(e) => onChange({ type: "raw", content: e.target.value, content_type: body.content_type })}
          />
        </>
      )}
      {body.type === "form_url_encoded" && (
        <KeyValueTable items={body.items} onChange={(items) => onChange({ type: "form_url_encoded", items })} />
      )}
      {body.type === "form_data" && (
        <KeyValueTable items={body.items} onChange={(items) => onChange({ type: "form_data", items })} />
      )}
      {body.type === "none" && (
        <div className="empty-state" style={{ padding: 16, opacity: 0.6 }}>
          无请求体
        </div>
      )}
    </div>
  </div>
);

const AUTH_TYPES: { value: AuthConfig["type"]; label: string }[] = [
  { value: "none", label: "无认证" },
  { value: "bearer", label: "Bearer Token" },
  { value: "basic", label: "Basic Auth" },
  { value: "api_key", label: "API Key" },
];

const AuthEditor: React.FC<{ auth: AuthConfig; onChange: (a: AuthConfig) => void }> = ({ auth, onChange }) => (
  <div>
    <select
      className="form-select"
      value={auth.type}
      onChange={(e) => {
        switch (e.target.value as AuthConfig["type"]) {
          case "none":
            onChange({ type: "none" });
            break;
          case "bearer":
            onChange({ type: "bearer", token: "" });
            break;
          case "basic":
            onChange({ type: "basic", username: "", password: "" });
            break;
          case "api_key":
            onChange({ type: "api_key", key: "", value: "", add_to: "header" });
            break;
        }
      }}
    >
      {AUTH_TYPES.map((a) => (
        <option key={a.value} value={a.value}>
          {a.label}
        </option>
      ))}
    </select>
    <div style={{ marginTop: 8, display: "flex", flexDirection: "column", gap: 6, maxWidth: 380 }}>
      {auth.type === "bearer" && (
        <input
          className="form-input"
          placeholder="Token（可用 {{var}}）"
          value={auth.token}
          onChange={(e) => onChange({ type: "bearer", token: e.target.value })}
        />
      )}
      {auth.type === "basic" && (
        <>
          <input
            className="form-input"
            placeholder="用户名"
            value={auth.username}
            onChange={(e) => onChange({ type: "basic", username: e.target.value, password: auth.password })}
          />
          <input
            className="form-input"
            type="password"
            placeholder="密码"
            value={auth.password}
            onChange={(e) => onChange({ type: "basic", username: auth.username, password: e.target.value })}
          />
        </>
      )}
      {auth.type === "api_key" && (
        <>
          <input
            className="form-input"
            placeholder="Key 名称"
            value={auth.key}
            onChange={(e) => onChange({ type: "api_key", key: e.target.value, value: auth.value, add_to: auth.add_to })}
          />
          <input
            className="form-input"
            placeholder="Value（可用 {{var}}）"
            value={auth.value}
            onChange={(e) => onChange({ type: "api_key", key: auth.key, value: e.target.value, add_to: auth.add_to })}
          />
          <select
            className="form-select"
            value={auth.add_to}
            onChange={(e) =>
              onChange({ type: "api_key", key: auth.key, value: auth.value, add_to: e.target.value as "header" | "query" })
            }
          >
            <option value="header">放在 Header</option>
            <option value="query">放在 Query 参数</option>
          </select>
        </>
      )}
      {auth.type === "none" && (
        <div className="empty-state" style={{ padding: 16, opacity: 0.6 }}>
          不添加任何认证信息
        </div>
      )}
    </div>
  </div>
);

/** 中栏：URL 栏 + Params/Headers/Body/Auth 多 Tab 编辑（docs/HTTP_DESKTOP_PLAN.md
 * §3.3）。前置/后置脚本 Tab 没有实现——`RequestDef` 目前没有 script 字段，见
 * http_desk/mod.rs 顶部范围说明。 */
export const RequestEditor: React.FC<{ requestId: string }> = ({ requestId }) => {
  const draft = useHttpDeskStore((s) => s.drafts[requestId]);
  const updateDraft = useHttpDeskStore((s) => s.updateDraft);
  const saveDraft = useHttpDeskStore((s) => s.saveDraft);
  const sendDraft = useHttpDeskStore((s) => s.sendDraft);
  const dirty = useHttpDeskStore((s) => s.dirty[requestId]);
  const sending = useHttpDeskStore((s) => s.sending);
  const [subTab, setSubTab] = useState<SubTab>("params");
  const push = useToastStore((s) => s.push);

  if (!draft) return null;

  const patch = (p: Partial<RequestDef>) => updateDraft(requestId, p);

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%", minWidth: 0 }}>
      <div style={{ display: "flex", gap: 6, padding: 8, borderBottom: "1px solid var(--border-default)" }}>
        <select className="form-select" style={{ width: 110 }} value={draft.method} onChange={(e) => patch({ method: e.target.value })}>
          {METHODS.map((m) => (
            <option key={m} value={m}>
              {m}
            </option>
          ))}
        </select>
        <input
          className="form-input"
          style={{ flex: 1, minWidth: 0 }}
          placeholder="https://api.example.com/users?id={{id}}"
          value={draft.url}
          onChange={(e) => patch({ url: e.target.value })}
        />
        <button className="btn primary sm" disabled={sending || !draft.url} onClick={() => void sendDraft(requestId)}>
          {sending ? "发送中…" : "发送"}
        </button>
        <button
          className="btn ghost sm"
          disabled={!dirty}
          onClick={() => void saveDraft(requestId).catch((e) => push("error", `保存失败：${formatError(e)}`))}
        >
          保存{dirty ? " *" : ""}
        </button>
      </div>
      <div style={{ display: "flex", gap: 4, padding: "4px 8px", borderBottom: "1px solid var(--border-default)" }}>
        {SUB_TABS.map((t) => (
          <button
            key={t.id}
            className={`btn ghost sm ${subTab === t.id ? "active" : ""}`}
            onClick={() => setSubTab(t.id)}
          >
            {t.label}
          </button>
        ))}
      </div>
      <div style={{ flex: 1, minHeight: 0, overflowY: "auto", padding: 8 }}>
        {subTab === "params" && <KeyValueTable items={draft.params} onChange={(items) => patch({ params: items })} />}
        {subTab === "headers" && (
          <KeyValueTable items={draft.headers} onChange={(items) => patch({ headers: items })} keyPlaceholder="Header" />
        )}
        {subTab === "body" && <BodyEditor body={draft.body} onChange={(body) => patch({ body })} />}
        {subTab === "auth" && <AuthEditor auth={draft.auth} onChange={(auth) => patch({ auth })} />}
      </div>
    </div>
  );
};
