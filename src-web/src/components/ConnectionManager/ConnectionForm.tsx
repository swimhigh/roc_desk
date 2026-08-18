import React, { useState } from "react";

export type AuthMethod = "password" | "key" | "agent";

export interface ConnectionFormValue {
  name: string;
  host: string;
  port: number;
  username: string;
  authMethod: AuthMethod;
  secret: string; // 密码或私钥口令，提交后由后端写入系统密钥链，不落库明文
  groupId?: string;
  jumpHostId?: string;
}

interface ConnectionFormProps {
  initial?: Partial<ConnectionFormValue>;
  groups: { id: string; name: string }[];
  jumpHostOptions: { id: string; name: string }[];
  onCancel: () => void;
  onSave: (value: ConnectionFormValue) => void;
}

/**
 * 新建/编辑连接档案（DESIGN.md §3.2.2）。用 Dialog 呈现，不跳转新页面；
 * 密码/私钥口令默认 type=password，带显示切换。
 */
export const ConnectionForm: React.FC<ConnectionFormProps> = ({
  initial,
  groups,
  jumpHostOptions,
  onCancel,
  onSave,
}) => {
  const [value, setValue] = useState<ConnectionFormValue>({
    name: initial?.name ?? "",
    host: initial?.host ?? "",
    port: initial?.port ?? 22,
    username: initial?.username ?? "",
    authMethod: initial?.authMethod ?? "key",
    secret: initial?.secret ?? "",
    groupId: initial?.groupId,
    jumpHostId: initial?.jumpHostId,
  });
  const [secretVisible, setSecretVisible] = useState(false);

  const set = <K extends keyof ConnectionFormValue>(key: K, v: ConnectionFormValue[K]) =>
    setValue((s) => ({ ...s, [key]: v }));

  return (
    <div className="form">
      <div className="form-row">
        <label className="form-label">名称</label>
        <input
          className="form-input"
          value={value.name}
          placeholder="连接名称"
          onChange={(e) => set("name", e.target.value)}
        />
      </div>
      <div className="form-row split">
        <div className="form-row">
          <label className="form-label">Host</label>
          <input className="form-input" value={value.host} onChange={(e) => set("host", e.target.value)} />
        </div>
        <div className="form-row" style={{ maxWidth: 80 }}>
          <label className="form-label">Port</label>
          <input
            className="form-input"
            value={value.port}
            onChange={(e) => set("port", Number(e.target.value) || 22)}
          />
        </div>
      </div>
      <div className="form-row">
        <label className="form-label">用户名</label>
        <input className="form-input" value={value.username} onChange={(e) => set("username", e.target.value)} />
      </div>
      <div className="form-row">
        <label className="form-label">认证方式</label>
        <select
          className="form-select"
          value={value.authMethod}
          onChange={(e) => set("authMethod", e.target.value as AuthMethod)}
        >
          <option value="password">密码</option>
          <option value="key">密钥</option>
          <option value="agent">SSH Agent</option>
        </select>
      </div>
      {value.authMethod !== "agent" && (
        <div className="form-row">
          <label className="form-label">{value.authMethod === "key" ? "私钥口令" : "密码"}</label>
          <div className="form-input-group">
            <input
              className="form-input"
              type={secretVisible ? "text" : "password"}
              value={value.secret}
              onChange={(e) => set("secret", e.target.value)}
            />
            <button className="btn ghost sm" onClick={() => setSecretVisible((v) => !v)}>
              {secretVisible ? "隐藏" : "显示"}
            </button>
          </div>
        </div>
      )}
      <div className="form-row split">
        <div className="form-row">
          <label className="form-label">分组</label>
          <select className="form-select" value={value.groupId ?? ""} onChange={(e) => set("groupId", e.target.value)}>
            <option value="">— 未分组 —</option>
            {groups.map((g) => (
              <option key={g.id} value={g.id}>{g.name}</option>
            ))}
          </select>
        </div>
        <div className="form-row">
          <label className="form-label">跳板机（可选）</label>
          <select
            className="form-select"
            value={value.jumpHostId ?? ""}
            onChange={(e) => set("jumpHostId", e.target.value)}
          >
            <option value="">— 无 —</option>
            {jumpHostOptions.map((h) => (
              <option key={h.id} value={h.id}>{h.name}</option>
            ))}
          </select>
        </div>
      </div>
      <div className="form-actions">
        <button className="btn ghost sm" onClick={onCancel}>取消</button>
        <button className="btn primary sm" onClick={() => onSave(value)}>保存</button>
      </div>
    </div>
  );
};
