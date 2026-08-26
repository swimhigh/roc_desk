import React, { useState } from "react";
import type { Protocol } from "../../types/bindings";

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
  protocol: Protocol;
  // RDP 专属（protocol === 'rdp' 时才有意义），存进 ConnectionProfile.options
  rdpDomain?: string;
  rdpWidth: number;
  rdpHeight: number;
  rdpColorDepth: number;
}

const DEFAULT_PORT: Record<Protocol, number> = { ssh: 22, rdp: 3389 };

interface ConnectionFormProps {
  initial?: Partial<ConnectionFormValue>;
  groups: { id: string; name: string }[];
  jumpHostOptions: { id: string; name: string }[];
  /** 传了这个就不显示协议选择器，永远按这个协议提交——工作区挂载向导只会创建
   * SSH 连接（RDP 没有"目录"这个概念，挂不了工作区）。*/
  fixedProtocol?: Protocol;
  onCancel: () => void;
  onSave: (value: ConnectionFormValue) => void;
}

/**
 * 新建/编辑连接档案（DESIGN.md §3.2.2、§3.9）。用 Dialog 呈现，不跳转新页面；
 * 密码/私钥口令默认 type=password，带显示切换。SSH/RDP 共用这一个表单——切换
 * 协议时把整块认证/RDP 字段换掉，其余（名称/Host/分组）两种协议都要填。
 */
export const ConnectionForm: React.FC<ConnectionFormProps> = ({
  initial,
  groups,
  jumpHostOptions,
  fixedProtocol,
  onCancel,
  onSave,
}) => {
  const [value, setValue] = useState<ConnectionFormValue>({
    name: initial?.name ?? "",
    host: initial?.host ?? "",
    port: initial?.port ?? DEFAULT_PORT[fixedProtocol ?? initial?.protocol ?? "ssh"],
    username: initial?.username ?? "",
    authMethod: initial?.authMethod ?? "key",
    secret: initial?.secret ?? "",
    groupId: initial?.groupId,
    jumpHostId: initial?.jumpHostId,
    protocol: fixedProtocol ?? initial?.protocol ?? "ssh",
    rdpDomain: initial?.rdpDomain,
    rdpWidth: initial?.rdpWidth ?? 1280,
    rdpHeight: initial?.rdpHeight ?? 800,
    rdpColorDepth: initial?.rdpColorDepth ?? 32,
  });
  const [secretVisible, setSecretVisible] = useState(false);

  const set = <K extends keyof ConnectionFormValue>(key: K, v: ConnectionFormValue[K]) =>
    setValue((s) => ({ ...s, [key]: v }));

  const setProtocol = (protocol: Protocol) =>
    setValue((s) => ({
      ...s,
      protocol,
      // 端口还停在上一个协议的默认值时顺手换掉，用户已经手改过的端口不碰。
      port: s.port === DEFAULT_PORT[s.protocol] ? DEFAULT_PORT[protocol] : s.port,
    }));

  return (
    <div className="form">
      {!fixedProtocol && (
        <div className="form-row">
          <label className="form-label">协议</label>
          <select className="form-select" value={value.protocol} onChange={(e) => setProtocol(e.target.value as Protocol)}>
            <option value="ssh">SSH（终端 / SFTP）</option>
            <option value="rdp">RDP（远程桌面）</option>
          </select>
        </div>
      )}
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
            onChange={(e) => set("port", Number(e.target.value) || DEFAULT_PORT[value.protocol])}
          />
        </div>
      </div>
      <div className="form-row">
        <label className="form-label">用户名</label>
        <input className="form-input" value={value.username} onChange={(e) => set("username", e.target.value)} />
      </div>
      {value.protocol === "ssh" ? (
        <>
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
        </>
      ) : (
        <>
          <div className="form-row split">
            <div className="form-row">
              <label className="form-label">密码</label>
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
            <div className="form-row">
              <label className="form-label">域（可选）</label>
              <input className="form-input" value={value.rdpDomain ?? ""} onChange={(e) => set("rdpDomain", e.target.value)} />
            </div>
          </div>
          <div className="form-row split">
            <div className="form-row">
              <label className="form-label">分辨率</label>
              <select
                className="form-select"
                value={`${value.rdpWidth}x${value.rdpHeight}`}
                onChange={(e) => {
                  const [w, h] = e.target.value.split("x").map(Number);
                  setValue((s) => ({ ...s, rdpWidth: w, rdpHeight: h }));
                }}
              >
                <option value="1280x800">1280 × 800</option>
                <option value="1366x768">1366 × 768</option>
                <option value="1600x900">1600 × 900</option>
                <option value="1920x1080">1920 × 1080</option>
              </select>
            </div>
            <div className="form-row">
              <label className="form-label">色深</label>
              <select
                className="form-select"
                value={value.rdpColorDepth}
                onChange={(e) => set("rdpColorDepth", Number(e.target.value))}
              >
                <option value={16}>16 位</option>
                <option value={24}>24 位</option>
                <option value={32}>32 位</option>
              </select>
            </div>
          </div>
        </>
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
        {value.protocol === "ssh" && (
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
        )}
      </div>
      <div className="form-actions">
        <button className="btn ghost sm" onClick={onCancel}>取消</button>
        <button className="btn primary sm" onClick={() => onSave(value)}>保存</button>
      </div>
    </div>
  );
};
