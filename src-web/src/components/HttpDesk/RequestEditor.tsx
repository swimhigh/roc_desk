import React, { useState } from "react";
import Editor from "@monaco-editor/react";
import { useHttpDeskStore } from "../../stores/httpDeskStore";
import { useThemeStore } from "../../stores/themeStore";
import { useToastStore } from "../shared/Toast";
import { formatError } from "../../utils/error";
import { httpMethodClass } from "../../utils/httpMethod";
import { KeyValueTable } from "./KeyValueTable";
import type { AuthConfig, RequestBody, RequestDef } from "../../types/bindings";

const METHODS = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];
type SubTab = "params" | "headers" | "body" | "auth";
const SUB_TABS: { id: SubTab; label: string; count?: (r: RequestDef) => number }[] = [
  { id: "params", label: "Params", count: (r) => r.params.filter((p) => p.key).length },
  { id: "headers", label: "Headers", count: (r) => r.headers.filter((h) => h.key).length },
  { id: "body", label: "Body" },
  { id: "auth", label: "Auth" },
];

const BODY_TYPES: { value: RequestBody["type"]; label: string }[] = [
  { value: "none", label: "无 Body" },
  { value: "json", label: "JSON" },
  { value: "raw", label: "Raw / XML / Text" },
  { value: "form_url_encoded", label: "x-www-form-urlencoded" },
  { value: "form_data", label: "form-data" },
];

function contentTypeLanguage(contentType: string): string {
  const ct = contentType.toLowerCase();
  if (ct.includes("json")) return "json";
  if (ct.includes("xml")) return "xml";
  if (ct.includes("html")) return "html";
  return "plaintext";
}

const MONACO_OPTIONS = {
  minimap: { enabled: false },
  fontSize: 13,
  automaticLayout: true,
  scrollBeyondLastLine: false,
  wordWrap: "on" as const,
  tabSize: 2,
};

const BodyEditor: React.FC<{ body: RequestBody; onChange: (b: RequestBody) => void }> = ({ body, onChange }) => {
  const monacoTheme = useThemeStore((s) => (s.theme === "dark" ? "roc-dark" : "roc-light"));

  return (
    <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
      <div className="http-body-type-row">
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
        {body.type === "raw" && (
          <input
            className="form-input"
            style={{ width: 220 }}
            value={body.content_type}
            onChange={(e) => onChange({ type: "raw", content: body.content, content_type: e.target.value })}
            placeholder="Content-Type"
          />
        )}
      </div>
      <div style={{ flex: 1, minHeight: 0 }}>
        {body.type === "json" && (
          <Editor
            language="json"
            theme={monacoTheme}
            value={body.content}
            onChange={(v) => onChange({ type: "json", content: v ?? "" })}
            options={MONACO_OPTIONS}
          />
        )}
        {body.type === "raw" && (
          <Editor
            language={contentTypeLanguage(body.content_type)}
            theme={monacoTheme}
            value={body.content}
            onChange={(v) => onChange({ type: "raw", content: v ?? "", content_type: body.content_type })}
            options={MONACO_OPTIONS}
          />
        )}
        {body.type === "form_url_encoded" && (
          <div style={{ height: "100%", overflowY: "auto", padding: "var(--space-2)" }}>
            <KeyValueTable items={body.items} onChange={(items) => onChange({ type: "form_url_encoded", items })} />
          </div>
        )}
        {body.type === "form_data" && (
          <div style={{ height: "100%", overflowY: "auto", padding: "var(--space-2)" }}>
            <KeyValueTable items={body.items} onChange={(items) => onChange({ type: "form_data", items })} />
          </div>
        )}
        {body.type === "none" && <div className="empty-state">这个请求没有 Body</div>}
      </div>
    </div>
  );
};

const AUTH_TYPES: { value: AuthConfig["type"]; label: string }[] = [
  { value: "none", label: "无认证" },
  { value: "bearer", label: "Bearer Token" },
  { value: "basic", label: "Basic Auth" },
  { value: "api_key", label: "API Key" },
];

const AuthEditor: React.FC<{ auth: AuthConfig; onChange: (a: AuthConfig) => void }> = ({ auth, onChange }) => (
  <div style={{ padding: "var(--space-3) var(--space-2)" }}>
    <div className="form-row" style={{ maxWidth: 320, marginBottom: "var(--space-3)" }}>
      <span className="form-label">认证方式</span>
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
    </div>
    <div style={{ display: "flex", flexDirection: "column", gap: "var(--space-3)", maxWidth: 380 }}>
      {auth.type === "bearer" && (
        <div className="form-row">
          <span className="form-label">Token</span>
          <input
            className="form-input"
            placeholder="支持 {{var}} 变量插值"
            value={auth.token}
            onChange={(e) => onChange({ type: "bearer", token: e.target.value })}
          />
        </div>
      )}
      {auth.type === "basic" && (
        <>
          <div className="form-row">
            <span className="form-label">用户名</span>
            <input className="form-input" value={auth.username} onChange={(e) => onChange({ type: "basic", username: e.target.value, password: auth.password })} />
          </div>
          <div className="form-row">
            <span className="form-label">密码</span>
            <input
              className="form-input"
              type="password"
              value={auth.password}
              onChange={(e) => onChange({ type: "basic", username: auth.username, password: e.target.value })}
            />
          </div>
        </>
      )}
      {auth.type === "api_key" && (
        <>
          <div className="form-row">
            <span className="form-label">Key 名称</span>
            <input className="form-input" value={auth.key} onChange={(e) => onChange({ type: "api_key", key: e.target.value, value: auth.value, add_to: auth.add_to })} />
          </div>
          <div className="form-row">
            <span className="form-label">Value</span>
            <input
              className="form-input"
              placeholder="支持 {{var}} 变量插值"
              value={auth.value}
              onChange={(e) => onChange({ type: "api_key", key: auth.key, value: e.target.value, add_to: auth.add_to })}
            />
          </div>
          <div className="form-row">
            <span className="form-label">添加位置</span>
            <select
              className="form-select"
              value={auth.add_to}
              onChange={(e) => onChange({ type: "api_key", key: auth.key, value: auth.value, add_to: e.target.value as "header" | "query" })}
            >
              <option value="header">Header</option>
              <option value="query">Query 参数</option>
            </select>
          </div>
        </>
      )}
      {auth.type === "none" && <div className="empty-state">不添加任何认证信息</div>}
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
      <div style={{ display: "flex", gap: "var(--space-2)", padding: "var(--space-2)", borderBottom: "1px solid var(--border-default)" }}>
        <select
          className={`http-method-select ${httpMethodClass(draft.method)}`}
          value={draft.method}
          onChange={(e) => patch({ method: e.target.value })}
        >
          {METHODS.map((m) => (
            <option key={m} value={m}>
              {m}
            </option>
          ))}
        </select>
        <input
          className="form-input"
          style={{ flex: 1, minWidth: 0, fontFamily: "monospace" }}
          placeholder="https://api.example.com/users?id={{id}}"
          value={draft.url}
          onChange={(e) => patch({ url: e.target.value })}
          onKeyDown={(e) => {
            if (e.key === "Enter" && draft.url && !sending) void sendDraft(requestId);
          }}
        />
        <button className="btn primary sm" disabled={sending || !draft.url} onClick={() => void sendDraft(requestId)}>
          {sending ? "发送中…" : "发送"}
        </button>
        <button
          className="btn ghost sm"
          disabled={!dirty}
          onClick={() => void saveDraft(requestId).catch((e) => push("error", `保存失败：${formatError(e)}`))}
        >
          保存{dirty ? " •" : ""}
        </button>
      </div>
      <div className="http-subtabs">
        {SUB_TABS.map((t) => {
          const count = t.count?.(draft) ?? 0;
          return (
            <div key={t.id} className={`http-subtab ${subTab === t.id ? "active" : ""}`} onClick={() => setSubTab(t.id)}>
              {t.label}
              {count > 0 && <span className="http-subtab-count">{count}</span>}
            </div>
          );
        })}
      </div>
      {subTab === "params" && (
        <div style={{ flex: 1, minHeight: 0, overflowY: "auto", padding: "var(--space-2)" }}>
          <KeyValueTable items={draft.params} onChange={(items) => patch({ params: items })} />
        </div>
      )}
      {subTab === "headers" && (
        <div style={{ flex: 1, minHeight: 0, overflowY: "auto", padding: "var(--space-2)" }}>
          <KeyValueTable items={draft.headers} onChange={(items) => patch({ headers: items })} keyPlaceholder="Header" />
        </div>
      )}
      {subTab === "body" && <BodyEditor body={draft.body} onChange={(body) => patch({ body })} />}
      {subTab === "auth" && (
        <div style={{ flex: 1, minHeight: 0, overflowY: "auto" }}>
          <AuthEditor auth={draft.auth} onChange={(auth) => patch({ auth })} />
        </div>
      )}
    </div>
  );
};
