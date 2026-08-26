import React, { useEffect, useState } from "react";
import { Code2, FolderOpen, Server, Laptop } from "lucide-react";
import { useWorkspaceStore } from "../../stores/workspaceStore";
import { RemoteWorkspaceDialog } from "./RemoteWorkspaceDialog";
import { PasswordPromptDialog } from "../ConnectionManager/PasswordPromptDialog";
import { connectionService } from "../../services/connectionService";
import { formatError } from "../../utils/error";
import { useToastStore } from "../shared/Toast";
import { ThemeToggle } from "../shared/ThemeToggle";
import { isAppError } from "../../types/bindings";
import type { ConnectionProfile } from "../../types/bindings";

/** 应用入口（DESIGN.md §3.1.1 / UI_DESIGN.md §3.1）：打开本地文件夹 / 连接远程主机 / 最近工作区。*/
export const WorkspacePicker: React.FC = () => {
  const { recent, loading, error, loadRecent, openLocalFolder, openLocalPath, openRemoteWorkspace, removeFromRecent } =
    useWorkspaceStore();
  const [showRemoteDialog, setShowRemoteDialog] = useState(false);
  const [passwordPrompt, setPasswordPrompt] = useState<{ profile: ConnectionProfile; remotePath: string } | null>(
    null,
  );
  const [savingPassword, setSavingPassword] = useState(false);
  const push = useToastStore((s) => s.push);

  useEffect(() => {
    loadRecent();
  }, [loadRecent]);

  const reopenRecent = async (w: (typeof recent)[number]) => {
    if (w.kind === "local") {
      try {
        await openLocalPath(w.root_path);
      } catch (e) {
        push("error", `打开工作区失败：${formatError(e)}`);
      }
      return;
    }
    if (!w.connection_id) return;
    try {
      await openRemoteWorkspace(w.connection_id, w.root_path);
    } catch (e) {
      // 同一个 bug 的另一个触发点：从"最近工作区"直接重开远程工作区也会走
      // ssh_connect，历史连接一样可能没有真正保存的密码（见 RemoteWorkspaceDialog
      // 里同款注释），这里也要给出补录密码的入口，而不是死路一条的错误提示。
      if (isAppError(e) && e.kind === "Auth" && w.connection_id) {
        try {
          const profiles = await connectionService.list();
          const profile = profiles.find((p) => p.id === w.connection_id);
          if (profile) {
            setPasswordPrompt({ profile, remotePath: w.root_path });
            return;
          }
        } catch {
          // 拉取连接列表也失败的话就退回到下面的通用错误提示
        }
      }
      push("error", `重新连接失败：${formatError(e)}`);
    }
  };

  const handleSavePassword = async (password: string) => {
    if (!passwordPrompt) return;
    setSavingPassword(true);
    try {
      const { profile, remotePath } = passwordPrompt;
      await connectionService.update(profile.id, {
        name: profile.name,
        host: profile.host,
        port: profile.port,
        username: profile.username,
        auth_method: profile.auth_method,
        secret: password,
        group_id: profile.group_id,
        tags: profile.tags,
        jump_host_id: profile.jump_host_id,
        protocol: profile.protocol,
        options: profile.options,
      });
      setPasswordPrompt(null);
      await openRemoteWorkspace(profile.id, remotePath);
    } catch (e) {
      push("error", `保存密码失败：${formatError(e)}`);
    } finally {
      setSavingPassword(false);
    }
  };

  return (
    <div className="workspace-picker-screen">
      <ThemeToggle className="wp-theme-toggle" />
      <div className="wp-logo">
        <Code2 />
        roc_desk
      </div>

      <div className="wp-entries">
        <button className="wp-entry-btn" onClick={() => openLocalFolder()} disabled={loading}>
          <FolderOpen />
          打开本地文件夹
        </button>
        <button className="wp-entry-btn" onClick={() => setShowRemoteDialog(true)}>
          <Server />
          连接远程主机并选择目录
        </button>
      </div>

      {error && <div className="toast error">{error}</div>}

      <div className="wp-recent">
        <div className="wp-recent-title">最近打开的工作区</div>
        {recent.length === 0 ? (
          <div style={{ fontSize: 13, color: "var(--text-secondary)", textAlign: "center" }}>暂无最近工作区</div>
        ) : (
          <div className="wp-recent-list">
            {recent.map((w) => (
              <div key={w.id} className="wp-recent-item" onClick={() => reopenRecent(w)}>
                <span className="wp-kind-icon">
                  {w.kind === "local" ? <Laptop size={16} /> : <Server size={16} />}
                </span>
                <span className="wp-name">{w.display_name}</span>
                <span className="wp-path">{w.root_path}</span>
                <button
                  className="btn ghost sm"
                  onClick={(e) => {
                    e.stopPropagation();
                    removeFromRecent(w.id);
                  }}
                >
                  移除
                </button>
              </div>
            ))}
          </div>
        )}
      </div>

      {showRemoteDialog && <RemoteWorkspaceDialog onClose={() => setShowRemoteDialog(false)} />}
      {passwordPrompt && (
        <PasswordPromptDialog
          open
          connectionName={passwordPrompt.profile.name}
          submitting={savingPassword}
          onCancel={() => setPasswordPrompt(null)}
          onSubmit={handleSavePassword}
        />
      )}
    </div>
  );
};
