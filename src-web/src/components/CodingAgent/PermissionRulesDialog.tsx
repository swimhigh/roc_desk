import React, { useEffect, useState } from "react";
import { Trash2 } from "lucide-react";
import { ConfirmDialog } from "../shared/ConfirmDialog";
import { useCodingStore } from "../../stores/codingStore";
import { useToastStore } from "../shared/Toast";
import { formatError } from "../../utils/error";
import type { PermissionDecision, PermissionRuleInput } from "../../types/bindings";

interface PermissionRulesDialogProps {
  onClose: () => void;
}

const emptyForm: PermissionRuleInput = { tool: "run_command", pattern: "", decision: "allow" };

const toolLabel: Record<string, string> = {
  run_command: "执行命令",
  webfetch: "抓取网页",
  mcp: "MCP 工具调用",
};

const decisionLabel: Record<PermissionDecision, string> = { allow: "允许", ask: "询问", deny: "拒绝" };

/**
 * 权限规则管理（REQUIREMENTS.md §3.7 权限引擎升级）：allow/ask/deny 三态 + 按
 * 模式匹配，结构照抄 `ProviderManagerDialog.tsx`（列表 + 复用表单 + 增删）——
 * 规则只有"新增/删除"，没有"编辑"（改了等于新建一条，删旧的更清楚）。
 */
export const PermissionRulesDialog: React.FC<PermissionRulesDialogProps> = ({ onClose }) => {
  const { permissionRules, loadPermissionRules, createPermissionRule, deletePermissionRule } = useCodingStore();
  const [form, setForm] = useState<PermissionRuleInput>(emptyForm);
  const [saving, setSaving] = useState(false);
  const push = useToastStore((s) => s.push);

  useEffect(() => {
    loadPermissionRules();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const set = <K extends keyof PermissionRuleInput>(key: K, v: PermissionRuleInput[K]) => setForm((f) => ({ ...f, [key]: v }));

  const handleSave = async () => {
    if (!form.pattern.trim()) return;
    setSaving(true);
    try {
      await createPermissionRule(form);
      setForm({ ...emptyForm, tool: form.tool });
      push("success", "已添加规则");
    } catch (e) {
      push("error", `保存失败：${formatError(e)}`);
    } finally {
      setSaving(false);
    }
  };

  return (
    <ConfirmDialog open severity="info" icon="🔐" title="权限规则管理" onDismiss={onClose} actions={<button className="btn ghost sm" onClick={onClose}>关闭</button>}>
      <p style={{ fontSize: 12, color: "var(--text-secondary)", marginBottom: 10 }}>
        控制 AI 编程助手执行命令 / 抓取网页 / 调用 MCP 工具时是否需要弹窗确认。规则按创建时间排序，后创建的优先命中；
        黑名单命中的高危命令永远被拦截，不受这里的规则影响。
      </p>
      <div style={{ maxHeight: 220, overflowY: "auto", marginBottom: 12 }}>
        {permissionRules.length === 0 ? (
          <p style={{ fontSize: 13, color: "var(--text-secondary)" }}>还没有自定义规则，未匹配的调用按各工具默认策略处理。</p>
        ) : (
          permissionRules.map((rule) => (
            <div key={rule.id} className="file-row" style={{ gridTemplateColumns: "auto 1fr auto auto" }}>
              <span style={{ color: "var(--text-secondary)", fontSize: 11 }}>{toolLabel[rule.tool] ?? rule.tool}</span>
              <code style={{ fontSize: 12 }}>{rule.pattern}</code>
              <span
                style={{
                  fontSize: 11,
                  color: rule.decision === "deny" ? "var(--danger)" : rule.decision === "allow" ? "var(--accent)" : "var(--text-secondary)",
                }}
              >
                {decisionLabel[rule.decision]}
              </span>
              <button className="btn ghost sm" onClick={() => deletePermissionRule(rule.id)} title="删除">
                <Trash2 style={{ width: 14, height: 14 }} />
              </button>
            </div>
          ))
        )}
      </div>

      <div className="form">
        <div className="form-row">
          <label className="form-label">工具</label>
          <select className="form-select" value={form.tool} onChange={(e) => set("tool", e.target.value)}>
            <option value="run_command">执行命令</option>
            <option value="webfetch">抓取网页</option>
            <option value="mcp">MCP 工具调用</option>
          </select>
        </div>
        <div className="form-row">
          <label className="form-label">模式</label>
          <input
            className="form-input"
            value={form.pattern}
            onChange={(e) => set("pattern", e.target.value)}
            placeholder={form.tool === "run_command" ? "如：git * / npm install" : form.tool === "webfetch" ? "如：https://example.com/*" : "如：filesystem:* / filesystem:read_file"}
          />
        </div>
        <div className="form-row">
          <label className="form-label">动作</label>
          <select className="form-select" value={form.decision} onChange={(e) => set("decision", e.target.value as PermissionDecision)}>
            <option value="allow">允许</option>
            <option value="ask">询问</option>
            <option value="deny">拒绝</option>
          </select>
        </div>
        <div className="form-actions">
          <button className="btn primary sm" onClick={handleSave} disabled={saving || !form.pattern.trim()}>
            {saving ? "保存中…" : "+ 添加规则"}
          </button>
        </div>
      </div>
    </ConfirmDialog>
  );
};
