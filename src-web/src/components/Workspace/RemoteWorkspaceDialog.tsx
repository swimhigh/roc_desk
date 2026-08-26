import React, { useEffect, useState } from "react";
import { connectionService } from "../../services/connectionService";
import { connectionGroupService } from "../../services/connectionGroupService";
import { sftpService } from "../../services/sftpService";
import { useWorkspaceStore } from "../../stores/workspaceStore";
import { formatError } from "../../utils/error";
import { useToastStore } from "../shared/Toast";
import { ConnectionForm, type ConnectionFormValue } from "../ConnectionManager/ConnectionForm";
import { PasswordPromptDialog } from "../ConnectionManager/PasswordPromptDialog";
import { isAppError } from "../../types/bindings";
import type { ConnectionGroup, ConnectionProfile, FileEntry } from "../../types/bindings";

type Step = "pick" | "new" | "browse";

interface RemoteWorkspaceDialogProps {
  onClose: () => void;
}

/**
 * "连接远程主机并选择目录"入口的三步流程（DESIGN.md §3.1.1）：
 * 选/建连接档案 → 轻量远程目录浏览器（只做单选目录，不含上传下载） → 确认打开工作区。
 */
export const RemoteWorkspaceDialog: React.FC<RemoteWorkspaceDialogProps> = ({ onClose }) => {
  const [step, setStep] = useState<Step>("pick");
  const [profiles, setProfiles] = useState<ConnectionProfile[]>([]);
  const [groups, setGroups] = useState<ConnectionGroup[]>([]);
  const [activeConnectionId, setActiveConnectionId] = useState<string | null>(null);
  const [cwd, setCwd] = useState("/");
  const [entries, setEntries] = useState<FileEntry[]>([]);
  const [browsing, setBrowsing] = useState(false);
  const [passwordPromptFor, setPasswordPromptFor] = useState<ConnectionProfile | null>(null);
  const [savingPassword, setSavingPassword] = useState(false);
  const push = useToastStore((s) => s.push);
  const openRemoteWorkspace = useWorkspaceStore((s) => s.openRemoteWorkspace);

  useEffect(() => {
    connectionService.list().then(setProfiles).catch((e) => push("error", formatError(e)));
    connectionGroupService.list().then(setGroups).catch((e) => push("error", formatError(e)));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 只允许明确的“取消”按钮或 Esc 关闭；点击遮罩不会丢弃已填写的连接信息。
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !passwordPromptFor && !savingPassword) onClose();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onClose, passwordPromptFor, savingPassword]);

  const startBrowsing = async (connectionId: string, path: string) => {
    setActiveConnectionId(connectionId);
    setStep("browse");
    setBrowsing(true);
    try {
      const dirs = await sftpService.listDir(connectionId, path);
      setCwd(path);
      setEntries(dirs.filter((e) => e.is_dir));
    } catch (e) {
      // Auth 类错误大多是"这个连接没有可用的已保存密码"（真实 bug：keyring 之前
      // 没启用 windows-native，历史连接的密码全是假保存），弹出补录密码的入口，
      // 而不是让用户卡在一个没有任何补救办法的错误提示上。
      if (isAppError(e) && e.kind === "Auth") {
        const profile = profiles.find((p) => p.id === connectionId);
        if (profile) {
          setPasswordPromptFor(profile);
          setStep("pick");
          return;
        }
      }
      push("error", `浏览失败：${formatError(e)}`);
      setStep("pick");
    } finally {
      setBrowsing(false);
    }
  };

  const handleSavePassword = async (password: string) => {
    if (!passwordPromptFor) return;
    setSavingPassword(true);
    try {
      await connectionService.update(passwordPromptFor.id, {
        name: passwordPromptFor.name,
        host: passwordPromptFor.host,
        port: passwordPromptFor.port,
        username: passwordPromptFor.username,
        auth_method: passwordPromptFor.auth_method,
        secret: password,
        group_id: passwordPromptFor.group_id,
        tags: passwordPromptFor.tags,
        jump_host_id: passwordPromptFor.jump_host_id,
        protocol: passwordPromptFor.protocol,
        options: passwordPromptFor.options,
      });
      const connectionId = passwordPromptFor.id;
      setPasswordPromptFor(null);
      await startBrowsing(connectionId, "/");
    } catch (e) {
      push("error", `保存密码失败：${formatError(e)}`);
    } finally {
      setSavingPassword(false);
    }
  };

  const handleCreateConnection = async (value: ConnectionFormValue) => {
    try {
      const created = await connectionService.create({
        name: value.name,
        host: value.host,
        port: value.port,
        username: value.username,
        auth_method: value.authMethod,
        secret: value.secret || null,
        group_id: value.groupId ?? null,
        tags: [],
        jump_host_id: value.jumpHostId ?? null,
        protocol: "ssh",
        options: null,
      });
      setProfiles((p) => [...p, created]);
      await startBrowsing(created.id, created.username.startsWith("root") ? "/root" : `/home/${created.username}`);
    } catch (e) {
      push("error", `创建连接失败：${formatError(e)}`);
    }
  };

  const handleConfirm = async () => {
    if (!activeConnectionId) return;
    try {
      await openRemoteWorkspace(activeConnectionId, cwd);
      onClose();
    } catch (e) {
      push("error", `打开工作区失败：${formatError(e)}`);
    }
  };

  return (
    <div className="dialog-overlay" role="dialog" aria-modal="true">
      <div className="dialog" style={{ minWidth: 480, maxWidth: 560 }}>
        <div className="dialog-title-bar info">
          <span>🖥</span>
          <span>连接远程主机并选择目录</span>
        </div>
        <div className="dialog-body">
          {step === "pick" && (
            <div>
              {profiles.length === 0 ? (
                <p style={{ fontSize: 13, color: "var(--text-secondary)", marginBottom: 12 }}>
                  还没有已保存的连接。
                </p>
              ) : (
                <div style={{ display: "flex", flexDirection: "column", gap: 4, marginBottom: 12 }}>
                  {profiles.map((p) => (
                    <div
                      key={p.id}
                      className="file-row"
                      style={{ gridTemplateColumns: "1fr auto", cursor: "pointer" }}
                      onClick={() => startBrowsing(p.id, "/")}
                    >
                      <span>{p.name} — {p.username}@{p.host}:{p.port}</span>
                      <span style={{ color: "var(--text-secondary)", fontSize: 12 }}>连接 →</span>
                    </div>
                  ))}
                </div>
              )}
              <button className="btn ghost sm" onClick={() => setStep("new")}>+ 新建连接</button>
            </div>
          )}

          {step === "new" && (
            <ConnectionForm
              groups={groups.map((g) => ({ id: g.id, name: g.name }))}
              jumpHostOptions={profiles.filter((p) => p.protocol === "ssh").map((p) => ({ id: p.id, name: p.name }))}
              fixedProtocol="ssh"
              onCancel={() => setStep("pick")}
              onSave={handleCreateConnection}
            />
          )}

          {step === "browse" && (
            <div>
              <div style={{ fontSize: 12, color: "var(--text-secondary)", marginBottom: 8 }}>
                当前路径：<span style={{ fontFamily: "var(--font-mono)" }}>{cwd}</span>
              </div>
              {browsing ? (
                <div style={{ padding: 16, fontSize: 12, color: "var(--text-secondary)" }}>加载中…</div>
              ) : (
                <div style={{ maxHeight: 260, overflowY: "auto", border: "1px solid var(--border-default)", borderRadius: 4 }}>
                  <div
                    className="file-row"
                    style={{ gridTemplateColumns: "1fr" }}
                    onClick={() => activeConnectionId && startBrowsing(activeConnectionId, cwd.split("/").slice(0, -1).join("/") || "/")}
                  >
                    <span>📁 ..</span>
                  </div>
                  {entries.map((e) => (
                    <div
                      key={e.path}
                      className="file-row"
                      style={{ gridTemplateColumns: "1fr" }}
                      onClick={() => activeConnectionId && startBrowsing(activeConnectionId, e.path)}
                    >
                      <span>📁 {e.name}</span>
                    </div>
                  ))}
                </div>
              )}
            </div>
          )}
        </div>
        <div className="dialog-actions">
          <button className="btn ghost sm" onClick={onClose}>取消</button>
          {step === "browse" && (
            <button className="btn primary sm" onClick={handleConfirm}>选定此目录作为工作区</button>
          )}
        </div>
      </div>
      {passwordPromptFor && (
        <PasswordPromptDialog
          open
          connectionName={passwordPromptFor.name}
          submitting={savingPassword}
          onCancel={() => setPasswordPromptFor(null)}
          onSubmit={handleSavePassword}
        />
      )}
    </div>
  );
};
