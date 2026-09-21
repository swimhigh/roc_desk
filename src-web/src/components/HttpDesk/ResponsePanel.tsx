import React, { useMemo, useState } from "react";
import Editor from "@monaco-editor/react";
import { useHttpDeskStore } from "../../stores/httpDeskStore";
import { useThemeStore } from "../../stores/themeStore";

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}

function statusClass(status: number): string {
  if (status >= 200 && status < 300) return "http-status-2xx";
  if (status >= 300 && status < 400) return "http-status-3xx";
  if (status >= 400) return "http-status-4xx";
  return "http-status-0";
}

function detectLanguage(contentType: string, body: string): string {
  const ct = contentType.toLowerCase();
  if (ct.includes("json")) return "json";
  if (ct.includes("xml")) return "xml";
  if (ct.includes("html")) return "html";
  const trimmed = body.trimStart();
  if (trimmed.startsWith("{") || trimmed.startsWith("[")) return "json";
  return "plaintext";
}

/** 响应区：状态码/耗时/大小 + Body/Headers 两个 Tab（docs/HTTP_DESKTOP_PLAN.md
 * §3.3）。Body 用只读 Monaco 展示（按 Content-Type/内容特征猜语言），比纯文本
 * `<pre>` 多了语法高亮和折叠——没有做图片/PDF 二进制预览，属于本轮为了先跑通
 * 主链路而缩的范围。 */
export const ResponsePanel: React.FC = () => {
  const response = useHttpDeskStore((s) => s.response);
  const sendError = useHttpDeskStore((s) => s.sendError);
  const sending = useHttpDeskStore((s) => s.sending);
  const [tab, setTab] = useState<"body" | "headers">("body");
  const monacoTheme = useThemeStore((s) => (s.theme === "dark" ? "roc-dark" : "roc-light"));

  const pretty = useMemo(() => {
    if (!response || !response.body_is_text) return "";
    try {
      return JSON.stringify(JSON.parse(response.body), null, 2);
    } catch {
      return response.body;
    }
  }, [response]);

  const contentType = response?.headers.find(([k]) => k.toLowerCase() === "content-type")?.[1] ?? "";

  if (sending) {
    return (
      <div className="empty-state" style={{ padding: 16 }}>
        请求发送中…
      </div>
    );
  }

  if (sendError) {
    return (
      <div role="alert" style={{ padding: 16, color: "var(--danger)", whiteSpace: "pre-wrap", fontSize: 13 }}>
        {sendError}
      </div>
    );
  }

  if (!response) {
    return (
      <div className="empty-state" style={{ padding: 16 }}>
        点击"发送"查看响应
      </div>
    );
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      <div className="http-response-meta">
        <span className={`http-status-badge ${statusClass(response.status)}`}>
          {response.status} {response.status_text}
        </span>
        <span className="meta-item">{response.duration_ms} ms</span>
        <span className="meta-item">{formatBytes(response.size_bytes)}</span>
        {response.truncated && <span style={{ color: "var(--danger)" }}>响应过大，仅预览前 5MB</span>}
      </div>
      <div className="http-subtabs">
        <div className={`http-subtab ${tab === "body" ? "active" : ""}`} onClick={() => setTab("body")}>
          Body
        </div>
        <div className={`http-subtab ${tab === "headers" ? "active" : ""}`} onClick={() => setTab("headers")}>
          Headers
          {response.headers.length > 0 && <span className="http-subtab-count">{response.headers.length}</span>}
        </div>
      </div>
      <div style={{ flex: 1, minHeight: 0 }}>
        {tab === "body" &&
          (response.body_is_text ? (
            <Editor
              language={detectLanguage(contentType, response.body)}
              theme={monacoTheme}
              value={pretty}
              options={{ readOnly: true, minimap: { enabled: false }, fontSize: 13, automaticLayout: true, scrollBeyondLastLine: false, wordWrap: "on" }}
            />
          ) : (
            <div className="empty-state" style={{ padding: 16 }}>
              二进制响应（{formatBytes(response.size_bytes)}），暂不支持预览
            </div>
          ))}
        {tab === "headers" && (
          <div className="kv-table" style={{ padding: "var(--space-2)", overflowY: "auto", height: "100%" }}>
            {response.headers.map(([k, v], i) => (
              <div className="kv-row" style={{ gridTemplateColumns: "180px 1fr" }} key={i}>
                <span style={{ fontWeight: 600, fontSize: 12.5 }}>{k}</span>
                <span style={{ fontSize: 12.5, wordBreak: "break-all" }}>{v}</span>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
};
