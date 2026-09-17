import React, { useState } from "react";
import { useHttpDeskStore } from "../../stores/httpDeskStore";

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}

function statusColor(status: number): string {
  if (status >= 200 && status < 300) return "#22863a";
  if (status >= 300 && status < 400) return "#0969da";
  if (status >= 400) return "#cf222e";
  return "#6e7781";
}

/** 响应区：状态码/耗时/大小 + Body/Headers 两个 Tab（docs/HTTP_DESKTOP_PLAN.md
 * §3.3）。Body 目前只有"原始文本"一种展示模式——没有做 JSON 折叠/格式化，也没有
 * 图片/PDF 预览，属于本轮为了先跑通主链路而缩的范围。 */
export const ResponsePanel: React.FC = () => {
  const response = useHttpDeskStore((s) => s.response);
  const sendError = useHttpDeskStore((s) => s.sendError);
  const sending = useHttpDeskStore((s) => s.sending);
  const [tab, setTab] = useState<"body" | "headers">("body");

  if (sending) {
    return (
      <div className="empty-state" style={{ padding: 16 }}>
        请求发送中…
      </div>
    );
  }

  if (sendError) {
    return (
      <div role="alert" style={{ padding: 16, color: "var(--danger)", whiteSpace: "pre-wrap" }}>
        {sendError}
      </div>
    );
  }

  if (!response) {
    return (
      <div className="empty-state" style={{ padding: 16, opacity: 0.6 }}>
        点击"发送"查看响应
      </div>
    );
  }

  let pretty = response.body;
  if (response.body_is_text) {
    try {
      pretty = JSON.stringify(JSON.parse(response.body), null, 2);
    } catch {
      pretty = response.body;
    }
  }

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      <div style={{ display: "flex", alignItems: "center", gap: 12, padding: 8, borderBottom: "1px solid var(--border-default)", fontSize: 13 }}>
        <span style={{ fontWeight: 700, color: statusColor(response.status) }}>
          {response.status} {response.status_text}
        </span>
        <span style={{ opacity: 0.75 }}>{response.duration_ms} ms</span>
        <span style={{ opacity: 0.75 }}>{formatBytes(response.size_bytes)}</span>
        {response.truncated && <span style={{ color: "var(--danger)" }}>响应过大，仅预览前 5MB</span>}
      </div>
      <div style={{ display: "flex", gap: 4, padding: "4px 8px", borderBottom: "1px solid var(--border-default)" }}>
        <button className={`btn ghost sm ${tab === "body" ? "active" : ""}`} onClick={() => setTab("body")}>
          Body
        </button>
        <button className={`btn ghost sm ${tab === "headers" ? "active" : ""}`} onClick={() => setTab("headers")}>
          Headers（{response.headers.length}）
        </button>
      </div>
      <div style={{ flex: 1, minHeight: 0, overflow: "auto", padding: 8 }}>
        {tab === "body" &&
          (response.body_is_text ? (
            <pre style={{ margin: 0, whiteSpace: "pre-wrap", wordBreak: "break-all", fontFamily: "monospace", fontSize: 12 }}>
              {pretty}
            </pre>
          ) : (
            <div className="empty-state" style={{ padding: 16, opacity: 0.6 }}>
              二进制响应（{formatBytes(response.size_bytes)}），暂不支持预览
            </div>
          ))}
        {tab === "headers" && (
          <table style={{ width: "100%", borderCollapse: "collapse", fontSize: 12 }}>
            <tbody>
              {response.headers.map(([k, v], i) => (
                <tr key={i}>
                  <td style={{ padding: "2px 8px 2px 0", fontWeight: 600, verticalAlign: "top", whiteSpace: "nowrap" }}>{k}</td>
                  <td style={{ padding: "2px 0", wordBreak: "break-all" }}>{v}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>
    </div>
  );
};
