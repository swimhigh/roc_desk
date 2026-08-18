import React, { useState } from "react";
import { Trash2, Pencil } from "lucide-react";
import { ConfirmDialog } from "../shared/ConfirmDialog";
import { useAiChatStore } from "../../stores/aiChatStore";
import { useToastStore } from "../shared/Toast";
import { formatError } from "../../utils/error";
import type { AiProviderInput } from "../../types/bindings";

interface ProviderManagerDialogProps {
  onClose: () => void;
}

const emptyForm: AiProviderInput = { name: "", api_base: "", api_key: "", model: "", is_local: false };

/**
 * AI Provider 管理（DESIGN.md §3.6）：豆包/OpenAI 兼容/DeepSeek/通义千问/本地 Ollama
 * 都走同一套 OpenAI 兼容协议，区别只在 api_base/model；API Key 走系统密钥链
 * （和 ConnectionForm 的密码字段是同一套模式）。
 *
 * 编辑态复用同一个表单：点"编辑"把已有 Provider 的字段（API Key 除外——后端从不
 * 把已保存的密钥吐回前端，这是密钥链的设计使然）填进表单，API Key 输入框留空
 * 表示"沿用原来保存的那份"，和 `AiProviderManager::update` 的语义完全对应
 * （空字符串不会覆盖已有的 credential_ref）。
 */
export const ProviderManagerDialog: React.FC<ProviderManagerDialogProps> = ({ onClose }) => {
  const { providers, createProvider, updateProvider, deleteProvider } = useAiChatStore();
  const [form, setForm] = useState<AiProviderInput>(emptyForm);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const push = useToastStore((s) => s.push);

  const set = <K extends keyof AiProviderInput>(key: K, v: AiProviderInput[K]) =>
    setForm((f) => ({ ...f, [key]: v }));

  const startEdit = (id: string) => {
    const p = providers.find((x) => x.id === id);
    if (!p) return;
    setEditingId(id);
    setForm({ name: p.name, api_base: p.api_base, api_key: "", model: p.model, is_local: p.is_local });
  };

  const cancelEdit = () => {
    setEditingId(null);
    setForm(emptyForm);
  };

  const handleSave = async () => {
    if (!form.name.trim() || !form.api_base.trim() || !form.model.trim()) return;
    setSaving(true);
    try {
      const payload = { ...form, api_key: form.api_key || null };
      if (editingId) {
        await updateProvider(editingId, payload);
        push("success", "已更新 Provider");
      } else {
        await createProvider(payload);
        push("success", "已添加 Provider");
      }
      setEditingId(null);
      setForm(emptyForm);
    } catch (e) {
      push("error", `保存失败：${formatError(e)}`);
    } finally {
      setSaving(false);
    }
  };

  return (
    <ConfirmDialog open severity="info" icon="🤖" title="AI Provider 管理" onDismiss={onClose} actions={<button className="btn ghost sm" onClick={onClose}>关闭</button>}>
      <div style={{ maxHeight: 200, overflowY: "auto", marginBottom: 12 }}>
        {providers.length === 0 ? (
          <p style={{ fontSize: 13, color: "var(--text-secondary)" }}>还没有配置 Provider。</p>
        ) : (
          providers.map((p) => (
            <div key={p.id} className="file-row" style={{ gridTemplateColumns: "1fr auto auto" }}>
              <span>
                {p.name}
                <span style={{ color: "var(--text-secondary)", fontSize: 11, marginLeft: 6 }}>
                  {p.is_local ? "本地" : "云端"} · {p.model}
                </span>
              </span>
              <button className="btn ghost sm" onClick={() => startEdit(p.id)} title="编辑">
                <Pencil style={{ width: 14, height: 14 }} />
              </button>
              <button className="btn ghost sm" onClick={() => deleteProvider(p.id)} title="删除">
                <Trash2 style={{ width: 14, height: 14 }} />
              </button>
            </div>
          ))
        )}
      </div>

      <div className="form">
        {editingId && (
          <div style={{ fontSize: 12, color: "var(--accent)", marginBottom: 4 }}>正在编辑，取消可回到新建</div>
        )}
        <div className="form-row">
          <label className="form-label">名称</label>
          <input className="form-input" value={form.name} onChange={(e) => set("name", e.target.value)} placeholder="如：DeepSeek" />
        </div>
        <div className="form-row">
          <label className="form-label">API Base</label>
          <input
            className="form-input"
            value={form.api_base}
            onChange={(e) => set("api_base", e.target.value)}
            placeholder="https://api.deepseek.com/v1"
          />
        </div>
        <div className="form-row">
          <label className="form-label">模型</label>
          <input className="form-input" value={form.model} onChange={(e) => set("model", e.target.value)} placeholder="deepseek-chat" />
        </div>
        <div className="form-row">
          <label className="form-label">API Key</label>
          <input
            className="form-input"
            type="password"
            value={form.api_key ?? ""}
            onChange={(e) => set("api_key", e.target.value)}
            placeholder={editingId ? "留空则沿用已保存的密钥" : "本地 Ollama 可留空"}
          />
        </div>
        <div className="form-row" style={{ flexDirection: "row", alignItems: "center", gap: 6 }}>
          <input type="checkbox" checked={form.is_local} onChange={(e) => set("is_local", e.target.checked)} />
          <label className="form-label" style={{ margin: 0 }}>
            本地模型（不受数据出境脱敏策略约束）
          </label>
        </div>
        <div className="form-actions">
          {editingId && (
            <button className="btn ghost sm" onClick={cancelEdit} disabled={saving}>
              取消
            </button>
          )}
          <button className="btn primary sm" onClick={handleSave} disabled={saving}>
            {saving ? "保存中…" : editingId ? "保存修改" : "+ 添加"}
          </button>
        </div>
      </div>
    </ConfirmDialog>
  );
};
