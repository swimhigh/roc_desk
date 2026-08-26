import React, { useEffect, useState } from "react";
import { Trash2, Pencil } from "lucide-react";
import { ConfirmDialog } from "../shared/ConfirmDialog";
import { useCodingStore } from "../../stores/codingStore";
import { useToastStore } from "../shared/Toast";
import { formatError } from "../../utils/error";
import type { McpServerInput, McpTransportKind } from "../../types/bindings";

interface McpServerManagerDialogProps {
  onClose: () => void;
}

const emptyForm: McpServerInput = {
  name: "",
  transport: "stdio",
  command: "",
  args: [],
  env: {},
  url: "",
  headers: {},
  auth_token: "",
  enabled: true,
};

/** `KEY=VALUE` / `Key: Value` 每行一条，解析成对象；空行/没有分隔符的行忽略。 */
function parseKvLines(text: string, sep: string): Record<string, string> {
  const out: Record<string, string> = {};
  for (const line of text.split("\n")) {
    const idx = line.indexOf(sep);
    if (idx < 0) continue;
    const key = line.slice(0, idx).trim();
    const value = line.slice(idx + sep.length).trim();
    if (key) out[key] = value;
  }
  return out;
}
const toKvLines = (obj: Record<string, string>, sep: string) => Object.entries(obj).map(([k, v]) => `${k}${sep}${v}`).join("\n");

/**
 * MCP 服务器管理（REQUIREMENTS.md §3.7"未实现：MCP 客户端"补上的部分）：本地
 * stdio 子进程 / 远程 HTTP 两种传输，结构照抄 `ProviderManagerDialog.tsx`。
 * 敏感的 HTTP 鉴权 token 留空表示"沿用已保存的那份"，和 Provider 的 API Key
 * 语义一致（后端从不把已保存的 token 明文吐回前端）。
 */
export const McpServerManagerDialog: React.FC<McpServerManagerDialogProps> = ({ onClose }) => {
  const { mcpServers, loadMcpServers, createMcpServer, updateMcpServer, deleteMcpServer } = useCodingStore();
  const [form, setForm] = useState<McpServerInput>(emptyForm);
  const [argsText, setArgsText] = useState("");
  const [envText, setEnvText] = useState("");
  const [headersText, setHeadersText] = useState("");
  const [editingId, setEditingId] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const push = useToastStore((s) => s.push);

  useEffect(() => {
    loadMcpServers();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const set = <K extends keyof McpServerInput>(key: K, v: McpServerInput[K]) => setForm((f) => ({ ...f, [key]: v }));

  const startEdit = (id: string) => {
    const server = mcpServers.find((s) => s.id === id);
    if (!server) return;
    setEditingId(id);
    setForm({
      name: server.name,
      transport: server.transport,
      command: server.command ?? "",
      args: server.args,
      env: server.env,
      url: server.url ?? "",
      headers: server.headers,
      auth_token: "",
      enabled: server.enabled,
    });
    setArgsText(server.args.join("\n"));
    setEnvText(toKvLines(server.env, "="));
    setHeadersText(toKvLines(server.headers, ": "));
  };

  const cancelEdit = () => {
    setEditingId(null);
    setForm(emptyForm);
    setArgsText("");
    setEnvText("");
    setHeadersText("");
  };

  const handleSave = async () => {
    if (!form.name.trim()) return;
    if (form.transport === "stdio" && !form.command?.trim()) return;
    if (form.transport === "http" && !form.url?.trim()) return;
    setSaving(true);
    try {
      const payload: McpServerInput = {
        ...form,
        args: argsText.split("\n").map((s) => s.trim()).filter(Boolean),
        env: parseKvLines(envText, "="),
        headers: parseKvLines(headersText, ": "),
        auth_token: form.auth_token || null,
      };
      if (editingId) {
        await updateMcpServer(editingId, payload);
        push("success", "已更新 MCP 服务器");
      } else {
        await createMcpServer(payload);
        push("success", "已添加 MCP 服务器");
      }
      cancelEdit();
    } catch (e) {
      push("error", `保存失败：${formatError(e)}`);
    } finally {
      setSaving(false);
    }
  };

  const canSave = form.name.trim() && (form.transport === "stdio" ? Boolean(form.command?.trim()) : Boolean(form.url?.trim()));

  return (
    <ConfirmDialog open severity="info" icon="🔌" title="MCP 服务器管理" onDismiss={onClose} actions={<button className="btn ghost sm" onClick={onClose}>关闭</button>}>
      <p style={{ fontSize: 12, color: "var(--text-secondary)", marginBottom: 10 }}>
        MCP 服务器的工具会合并进 AI 编程助手 Build 模式下可调用的工具列表；HTTP 传输仅支持单次 JSON 响应，不支持要求 SSE 长连接的服务器。
      </p>
      <div style={{ maxHeight: 180, overflowY: "auto", marginBottom: 12 }}>
        {mcpServers.length === 0 ? (
          <p style={{ fontSize: 13, color: "var(--text-secondary)" }}>还没有配置 MCP 服务器。</p>
        ) : (
          mcpServers.map((server) => (
            <div key={server.id} className="file-row" style={{ gridTemplateColumns: "1fr auto auto" }}>
              <span>
                {server.name}
                <span style={{ color: "var(--text-secondary)", fontSize: 11, marginLeft: 6 }}>
                  {server.transport === "stdio" ? "本地 stdio" : "远程 HTTP"} · {server.enabled ? "已启用" : "已停用"}
                </span>
              </span>
              <button className="btn ghost sm" onClick={() => startEdit(server.id)} title="编辑">
                <Pencil style={{ width: 14, height: 14 }} />
              </button>
              <button className="btn ghost sm" onClick={() => deleteMcpServer(server.id)} title="删除">
                <Trash2 style={{ width: 14, height: 14 }} />
              </button>
            </div>
          ))
        )}
      </div>

      <div className="form">
        {editingId && <div style={{ fontSize: 12, color: "var(--accent)", marginBottom: 4 }}>正在编辑，取消可回到新建</div>}
        <div className="form-row">
          <label className="form-label">名称</label>
          <input className="form-input" value={form.name} onChange={(e) => set("name", e.target.value)} placeholder="如：filesystem" />
        </div>
        <div className="form-row">
          <label className="form-label">传输方式</label>
          <select className="form-select" value={form.transport} onChange={(e) => set("transport", e.target.value as McpTransportKind)}>
            <option value="stdio">本地 stdio（起子进程）</option>
            <option value="http">远程 HTTP</option>
          </select>
        </div>
        {form.transport === "stdio" ? (
          <>
            <div className="form-row">
              <label className="form-label">可执行命令</label>
              <input className="form-input" value={form.command ?? ""} onChange={(e) => set("command", e.target.value)} placeholder="如：npx" />
            </div>
            <div className="form-row">
              <label className="form-label">参数（每行一个）</label>
              <textarea className="form-input" rows={2} value={argsText} onChange={(e) => setArgsText(e.target.value)} placeholder={"-y\n@modelcontextprotocol/server-everything"} />
            </div>
            <div className="form-row">
              <label className="form-label">环境变量（KEY=VALUE，每行一个）</label>
              <textarea className="form-input" rows={2} value={envText} onChange={(e) => setEnvText(e.target.value)} />
            </div>
          </>
        ) : (
          <>
            <div className="form-row">
              <label className="form-label">端点 URL</label>
              <input className="form-input" value={form.url ?? ""} onChange={(e) => set("url", e.target.value)} placeholder="https://example.com/mcp" />
            </div>
            <div className="form-row">
              <label className="form-label">请求头（Key: Value，每行一个）</label>
              <textarea className="form-input" rows={2} value={headersText} onChange={(e) => setHeadersText(e.target.value)} />
            </div>
            <div className="form-row">
              <label className="form-label">鉴权 Token（Bearer）</label>
              <input
                className="form-input"
                type="password"
                value={form.auth_token ?? ""}
                onChange={(e) => set("auth_token", e.target.value)}
                placeholder={editingId ? "留空则沿用已保存的 token" : "可留空"}
              />
            </div>
          </>
        )}
        <div className="form-row" style={{ flexDirection: "row", alignItems: "center", gap: 6 }}>
          <input type="checkbox" checked={form.enabled} onChange={(e) => set("enabled", e.target.checked)} />
          <label className="form-label" style={{ margin: 0 }}>启用（未启用不会出现在模型可调用的工具里）</label>
        </div>
        <div className="form-actions">
          {editingId && (
            <button className="btn ghost sm" onClick={cancelEdit} disabled={saving}>取消编辑</button>
          )}
          <button className="btn primary sm" onClick={handleSave} disabled={saving || !canSave}>
            {saving ? "保存中…" : editingId ? "保存修改" : "+ 添加"}
          </button>
        </div>
      </div>
    </ConfirmDialog>
  );
};
