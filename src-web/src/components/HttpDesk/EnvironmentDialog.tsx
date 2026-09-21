import React, { useState } from "react";
import { Plus, Trash2, X } from "lucide-react";
import { useHttpDeskStore } from "../../stores/httpDeskStore";
import { useToastStore } from "../shared/Toast";
import { formatError } from "../../utils/error";
import type { EnvVar } from "../../types/bindings";

function EnvVarTable({ vars, onChange }: { vars: EnvVar[]; onChange: (v: EnvVar[]) => void }) {
  const rows = vars.length === 0 || vars[vars.length - 1].key !== "" ? [...vars, { key: "", value: "", secret: false, enabled: true }] : vars;
  const update = (i: number, patch: Partial<EnvVar>) => {
    const next = rows.map((r, idx) => (idx === i ? { ...r, ...patch } : r));
    onChange(next.filter((r, idx) => !(idx === next.length - 1 && r.key === "" && r.value === "")));
  };
  const remove = (i: number) => onChange(rows.filter((_, idx) => idx !== i).filter((r) => r.key !== "" || r.value !== ""));

  return (
    <div className="kv-table">
      <div className="kv-row-header kv-row-secret">
        <span />
        <span>变量名</span>
        <span>值</span>
        <span style={{ textAlign: "center" }}>敏感</span>
        <span />
      </div>
      {rows.map((row, i) => {
        const empty = row.key === "" && row.value === "";
        return (
          <div className="kv-row kv-row-secret" key={i}>
            <input type="checkbox" checked={row.enabled} onChange={(e) => update(i, { enabled: e.target.checked })} disabled={row.key === ""} />
            <input value={row.key} onChange={(e) => update(i, { key: e.target.value })} placeholder="base_url" />
            <input
              type={row.secret ? "password" : "text"}
              value={row.value}
              onChange={(e) => update(i, { value: e.target.value })}
              placeholder="https://dev.example.com"
            />
            <span style={{ textAlign: "center" }}>
              <input type="checkbox" checked={row.secret} onChange={(e) => update(i, { secret: e.target.checked })} disabled={row.key === ""} />
            </span>
            <button className="kv-row-delete" onClick={() => remove(i)} title="删除" style={{ visibility: empty ? "hidden" : "visible" }}>
              <Trash2 size={13} />
            </button>
          </div>
        );
      })}
    </div>
  );
}

/** 环境/变量管理弹窗——覆盖 docs/HTTP_DESKTOP_PLAN.md §4.5 里"环境"和"全局"两层；
 * "集合"这一层变量走 §3.3 提到的集合设置（本轮没有单独做集合设置弹窗，先放
 * 在这个对话框里通过全局/环境两层覆盖大部分使用场景）。 */
export const EnvironmentDialog: React.FC<{ onClose: () => void }> = ({ onClose }) => {
  const environments = useHttpDeskStore((s) => s.environments);
  const globalVariables = useHttpDeskStore((s) => s.globalVariables);
  const saveEnvironment = useHttpDeskStore((s) => s.saveEnvironment);
  const deleteEnvironment = useHttpDeskStore((s) => s.deleteEnvironment);
  const saveGlobalVariables = useHttpDeskStore((s) => s.saveGlobalVariables);
  const push = useToastStore((s) => s.push);

  const [selectedId, setSelectedId] = useState<string>(environments[0]?.id ?? "__global__");
  const [newName, setNewName] = useState("");

  const selectedEnv = environments.find((e) => e.id === selectedId);

  return (
    <div style={{ position: "fixed", inset: 0, background: "color-mix(in srgb, var(--bg-app) 60%, transparent)", display: "grid", placeItems: "center", zIndex: 20 }}>
      <div className="dialog" style={{ width: 640, maxHeight: "80vh", display: "flex", flexDirection: "column" }}>
        <div style={{ display: "flex", alignItems: "center", padding: 12, borderBottom: "1px solid var(--border-default)" }}>
          <h3 style={{ margin: 0 }}>环境与变量</h3>
          <button className="btn ghost sm" style={{ marginLeft: "auto" }} onClick={onClose}>
            <X size={14} />
          </button>
        </div>
        <div style={{ display: "flex", flex: 1, minHeight: 0 }}>
          <div style={{ width: 170, borderRight: "1px solid var(--border-default)", overflowY: "auto", display: "flex", flexDirection: "column" }}>
            <div style={{ flex: 1, minHeight: 0, overflowY: "auto" }}>
              <div className={`tree-item ${selectedId === "__global__" ? "active" : ""}`} onClick={() => setSelectedId("__global__")}>
                <span className="tree-name">全局变量</span>
              </div>
              {environments.map((env) => (
                <div key={env.id} className={`tree-item ${selectedId === env.id ? "active" : ""}`} onClick={() => setSelectedId(env.id)}>
                  <span className="tree-name">{env.name}</span>
                </div>
              ))}
            </div>
            <div style={{ padding: 8, display: "flex", flexDirection: "column", gap: 6, borderTop: "1px solid var(--border-default)" }}>
              <input
                className="form-input"
                placeholder="新环境名"
                value={newName}
                onChange={(e) => setNewName(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" && newName.trim()) {
                    void saveEnvironment({ id: "", name: newName.trim(), variables: [] })
                      .then(() => setNewName(""))
                      .catch((err) => push("error", `新建环境失败：${formatError(err)}`));
                  }
                }}
              />
              <button
                className="btn ghost sm"
                disabled={!newName.trim()}
                onClick={() => {
                  void saveEnvironment({ id: "", name: newName.trim(), variables: [] })
                    .then(() => setNewName(""))
                    .catch((e) => push("error", `新建环境失败：${formatError(e)}`));
                }}
              >
                <Plus size={12} /> 新建环境
              </button>
            </div>
          </div>
          <div style={{ flex: 1, minWidth: 0, overflowY: "auto", padding: 12 }}>
            {selectedId === "__global__" ? (
              <>
                <EnvVarTable vars={globalVariables} onChange={(vars) => void saveGlobalVariables(vars).catch((e) => push("error", `保存失败：${formatError(e)}`))} />
              </>
            ) : selectedEnv ? (
              <>
                <div style={{ display: "flex", justifyContent: "flex-end", marginBottom: 8 }}>
                  <button
                    className="btn ghost sm"
                    onClick={() => {
                      if (confirm(`删除环境"${selectedEnv.name}"？`)) {
                        void deleteEnvironment(selectedEnv.id)
                          .then(() => setSelectedId("__global__"))
                          .catch((e) => push("error", `删除失败：${formatError(e)}`));
                      }
                    }}
                  >
                    <Trash2 size={12} /> 删除环境
                  </button>
                </div>
                <EnvVarTable
                  vars={selectedEnv.variables}
                  onChange={(vars) => void saveEnvironment({ ...selectedEnv, variables: vars }).catch((e) => push("error", `保存失败：${formatError(e)}`))}
                />
              </>
            ) : (
              <div className="empty-state">选择左侧一个环境</div>
            )}
          </div>
        </div>
      </div>
    </div>
  );
};
