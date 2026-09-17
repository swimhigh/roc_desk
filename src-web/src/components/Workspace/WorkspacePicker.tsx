import React, { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { Code2, FolderOpen, Home, ListPlus, Server, Laptop, Pencil, FilePlus2 } from "lucide-react";
import { useWorkspaceStore } from "../../stores/workspaceStore";
import { useTerminalStore } from "../../stores/terminalStore";
import { useModeStore, type WorkMode } from "../../stores/modeStore";
import { openExternalPaths } from "../../utils/openExternalPaths";
import { workspaceService } from "../../services/workspaceService";
import { RemoteWorkspaceDialog } from "./RemoteWorkspaceDialog";
import { PasswordPromptDialog } from "../ConnectionManager/PasswordPromptDialog";
import { connectionService } from "../../services/connectionService";
import { formatError } from "../../utils/error";
import { useToastStore } from "../shared/Toast";
import { ThemeToggle } from "../shared/ThemeToggle";
import { isAppError } from "../../types/bindings";
import type { ConnectionProfile, WorkspaceProfile } from "../../types/bindings";

const MODULE_LABEL: Partial<Record<WorkMode, string>> = {
  workspace: "工作区",
  http: "HTTP 测试工作台",
};

/** "从已有工作区中选择"子弹窗——列出系统里所有打开过的工作区（不管之前是哪个
 * 模块打开的），选一个就关联到当前模块并打开（2026-09 需求：模块之间不再共用
 * 同一份"最近工作区"，但同一个目录可以被多个模块各自"添加"）。 */
const SelectExistingWorkspaceDialog: React.FC<{
  module: WorkMode;
  excludeIds: string[];
  onClose: () => void;
  onSelected: (w: WorkspaceProfile) => void;
}> = ({ module, excludeIds, onClose, onSelected }) => {
  const [all, setAll] = useState<WorkspaceProfile[]>([]);
  const [loading, setLoading] = useState(true);
  const push = useToastStore((s) => s.push);

  useEffect(() => {
    workspaceService
      .listRecent()
      .then(setAll)
      .catch((e) => push("error", `加载工作区列表失败：${formatError(e)}`))
      .finally(() => setLoading(false));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const candidates = all.filter((w) => !excludeIds.includes(w.id));

  return (
    <div className="dialog-overlay" role="dialog" aria-modal="true">
      <div className="dialog" style={{ minWidth: 480, maxWidth: 560 }}>
        <div className="dialog-title-bar info">
          <ListPlus size={16} />
          <span>从已有工作区中选择</span>
        </div>
        <div className="dialog-body">
          {loading ? (
            <div style={{ fontSize: 13, color: "var(--text-secondary)" }}>加载中…</div>
          ) : candidates.length === 0 ? (
            <div style={{ fontSize: 13, color: "var(--text-secondary)" }}>
              没有可选的工作区了——所有已打开过的工作区都已经在{MODULE_LABEL[module] ?? module}里。
            </div>
          ) : (
            <div style={{ display: "flex", flexDirection: "column", gap: 4, maxHeight: 360, overflowY: "auto" }}>
              {candidates.map((w) => (
                <div key={w.id} className="file-row" style={{ gridTemplateColumns: "auto 1fr auto", cursor: "pointer" }} onClick={() => onSelected(w)}>
                  {w.kind === "local" ? <Laptop size={14} /> : <Server size={14} />}
                  <span>
                    {w.display_name} <span style={{ color: "var(--text-secondary)", fontSize: 12 }}>{w.root_path}</span>
                  </span>
                </div>
              ))}
            </div>
          )}
        </div>
        <div className="dialog-actions">
          <button className="btn ghost sm" onClick={onClose}>
            取消
          </button>
        </div>
      </div>
    </div>
  );
};

/** 应用入口（DESIGN.md §3.1.1 / UI_DESIGN.md §3.1）：打开本地文件夹 / 连接远程主机 /
 * 从已有工作区中选择 / 最近添加的工作区。
 *
 * 2026-09 需求：不再是"系统里所有打开过的工作区都会出现在这里"——每个模块
 * （`module` prop：`"workspace"` 工作区桌面 / `"http"` HTTP 测试工作台）各自
 * 维护一份显式"添加"过的子集（`workspace_module_links`，见后端仓库文档），新建/
 * 选择都要经过这个组件走一遍关联流程，不会因为在另一个模块打开过就自动出现。 */
export const WorkspacePicker: React.FC<{ module: WorkMode }> = ({ module }) => {
  const { loading, openLocalPath, openRemoteWorkspace, updatePath } = useWorkspaceStore();
  /** 有界保活的 LRU 常驻工作区集合（terminalStore.ts）——和 App.tsx 顶部"切换
   * 工作区"下拉菜单同一个用途，这里标在首页的"最近打开的工作区"列表上（2026-09-01
   * 用户需求：首页也要能看到在线状态）：在集合里说明这个工作区的终端 Channel/
   * xterm 实例还活着，切过去是原样恢复；不在集合里则是全新终端。 */
  const residentWorkspaceIds = useTerminalStore((s) => s.residentOrder);
  const [moduleWorkspaces, setModuleWorkspaces] = useState<WorkspaceProfile[]>([]);
  const [moduleLoading, setModuleLoading] = useState(true);
  const [showRemoteDialog, setShowRemoteDialog] = useState(false);
  const [editRemote, setEditRemote] = useState<WorkspaceProfile | null>(null);
  const [showSelectExisting, setShowSelectExisting] = useState(false);
  const [passwordPrompt, setPasswordPrompt] = useState<{ profile: ConnectionProfile; remotePath: string } | null>(
    null,
  );
  const [savingPassword, setSavingPassword] = useState(false);
  const push = useToastStore((s) => s.push);
  const goHome = useModeStore((s) => s.goHome);

  const refreshModuleList = async () => {
    setModuleLoading(true);
    try {
      setModuleWorkspaces(await workspaceService.listForModule(module));
    } catch (e) {
      push("error", `加载工作区列表失败：${formatError(e)}`);
    } finally {
      setModuleLoading(false);
    }
  };

  useEffect(() => {
    void refreshModuleList();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [module]);

  /** 新建/选择成功打开一个工作区之后，把它关联到当前模块——统一走这里，读
   * `useWorkspaceStore.getState().current` 而不是要求调用方各自传一份 profile，
   * 避免不同入口（本地文件夹/远程对话框/选择已有）传出不一致的数据。 */
  const linkCurrentToModule = async () => {
    const id = useWorkspaceStore.getState().current?.id;
    if (!id) return;
    try {
      await workspaceService.addModuleLink(id, module);
    } finally {
      await refreshModuleList();
    }
  };

  /** 目录配错了不用"移除再重新打开"——本地直接弹原生目录选择器改路径；远程复用
   * "连接远程主机并选择目录"里的目录浏览步骤，只是确认时改路径而不是新建工作区
   * （见 RemoteWorkspaceDialog 的 editWorkspace 分支）。 */
  const handleEdit = async (w: WorkspaceProfile) => {
    if (w.kind === "remote") {
      if (!w.connection_id) return;
      setEditRemote(w);
      return;
    }
    const selected = await open({ directory: true, multiple: false, defaultPath: w.root_path });
    if (!selected || Array.isArray(selected)) return;
    try {
      await updatePath(w.id, selected);
      push("success", "工作区目录已更新");
    } catch (e) {
      push("error", `更新工作区目录失败：${formatError(e)}`);
    }
  };

  const reopenRecent = async (w: WorkspaceProfile) => {
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

  const handleOpenLocalFolder = async () => {
    const selected = await open({ directory: true, multiple: false });
    if (!selected || Array.isArray(selected)) return;
    try {
      await openLocalPath(selected);
      await linkCurrentToModule();
    } catch (e) {
      push("error", `打开文件夹失败：${formatError(e)}`);
    }
  };

  const handleSelectExisting = async (w: WorkspaceProfile) => {
    setShowSelectExisting(false);
    await reopenRecent(w);
    await linkCurrentToModule();
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
      await linkCurrentToModule();
    } catch (e) {
      push("error", `保存密码失败：${formatError(e)}`);
    } finally {
      setSavingPassword(false);
    }
  };

  return (
    <div className="workspace-picker-screen">
      <button
        className="quick-tool-btn"
        style={{ position: "absolute", top: "var(--space-4)", left: "var(--space-4)" }}
        title="返回首页"
        onClick={() => void goHome().catch((e) => push("error", `返回首页失败：${formatError(e)}`))}
      >
        <Home />
      </button>
      <ThemeToggle className="wp-theme-toggle" />
      <div className="wp-logo">
        <Code2 />
        roc_desk
      </div>

      <div className="wp-entries">
        <button className="wp-entry-btn" onClick={() => void handleOpenLocalFolder()} disabled={loading}>
          <FolderOpen />
          打开本地文件夹
        </button>
        <button className="wp-entry-btn" onClick={() => setShowRemoteDialog(true)}>
          <Server />
          连接远程主机并选择目录
        </button>
        <button className="wp-entry-btn" onClick={() => setShowSelectExisting(true)}>
          <ListPlus />
          从已有工作区中选择
        </button>
        {/* 不建工作区，直接打开单个文件看/改（2026-09-03 需求，像 VSCode/Notepad 一样）——
            只对"工作区"模块有意义，HTTP 测试工作台不需要这个入口。 */}
        {module === "workspace" && (
          <button
            className="wp-entry-btn"
            onClick={async () => {
              const selected = await open({ directory: false, multiple: true });
              if (!selected) return;
              await openExternalPaths(Array.isArray(selected) ? selected : [selected]);
            }}
          >
            <FilePlus2 />
            打开文件
          </button>
        )}
      </div>

      <div className="wp-recent">
        <div className="wp-recent-title">{module === "http" ? "已添加的工作区" : "最近打开的工作区"}</div>
        {moduleLoading ? null : moduleWorkspaces.length === 0 ? (
          <div style={{ fontSize: 13, color: "var(--text-secondary)", textAlign: "center" }}>
            {module === "http" ? "还没有工作区，先新建或从已有工作区中选择一个" : "暂无最近工作区"}
          </div>
        ) : (
          <div className="wp-recent-list">
            {moduleWorkspaces.map((w) => (
              <div key={w.id} className="wp-recent-item" onClick={() => reopenRecent(w)}>
                <span
                  className={`status-dot ${residentWorkspaceIds.includes(w.id) ? "connected" : "disconnected"}`}
                  title={residentWorkspaceIds.includes(w.id) ? "终端会话保持中，切过去原样恢复" : "没有保持中的终端会话"}
                />
                <span className="wp-kind-icon">
                  {w.kind === "local" ? <Laptop size={16} /> : <Server size={16} />}
                </span>
                <span className="wp-name">{w.display_name}</span>
                <span className="wp-path">{w.root_path}</span>
                <button
                  className="btn ghost sm"
                  title="修改工作区目录"
                  onClick={(e) => {
                    e.stopPropagation();
                    handleEdit(w);
                  }}
                >
                  <Pencil size={13} /> 编辑
                </button>
                <button
                  className="btn ghost sm"
                  title={`从${MODULE_LABEL[module] ?? module}移除（不会删除工作区本身）`}
                  onClick={(e) => {
                    e.stopPropagation();
                    void workspaceService
                      .removeModuleLink(w.id, module)
                      .then(refreshModuleList)
                      .catch((err) => push("error", `移除失败：${formatError(err)}`));
                  }}
                >
                  移除
                </button>
              </div>
            ))}
          </div>
        )}
      </div>

      {showRemoteDialog && (
        <RemoteWorkspaceDialog onClose={() => setShowRemoteDialog(false)} onOpened={() => void linkCurrentToModule()} />
      )}
      {editRemote && editRemote.connection_id && (
        <RemoteWorkspaceDialog
          onClose={() => setEditRemote(null)}
          editWorkspace={{ id: editRemote.id, connectionId: editRemote.connection_id, initialPath: editRemote.root_path }}
        />
      )}
      {showSelectExisting && (
        <SelectExistingWorkspaceDialog
          module={module}
          excludeIds={moduleWorkspaces.map((w) => w.id)}
          onClose={() => setShowSelectExisting(false)}
          onSelected={(w) => void handleSelectExisting(w)}
        />
      )}
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
