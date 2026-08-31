import React, { useEffect, useMemo, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { Folder, FolderOpen, Terminal as TerminalIcon, Monitor, HardDrive, FolderCog, Plus } from "lucide-react";
import { useSessionTreeStore } from "../../stores/sessionTreeStore";
import { useRemoteSessionStore } from "../../stores/remoteSessionStore";
import { agentService } from "../../services/agentService";
import { useToastStore } from "../shared/Toast";
import { ContextMenu, type ContextMenuItem } from "../shared/ContextMenu";
import { ConfirmDialog } from "../shared/ConfirmDialog";
import { ConnectionDialog } from "./ConnectionDialog";
import { GroupDialog } from "./GroupDialog";
import { formatError } from "../../utils/error";
import type { ConnectionFormValue } from "../ConnectionManager/ConnectionForm";
import type { ConnectionGroup, ConnectionProfile, ConnectionProfileInput, Protocol } from "../../types/bindings";

type MenuTarget = { kind: "group"; group: ConnectionGroup } | { kind: "connection"; connection: ConnectionProfile } | { kind: "root" };

/**
 * 远程工具模式左侧会话树（DESIGN.md §3.9，参考截图二的 FinalShell 会话列表）：
 * 分组可折叠嵌套，连接按协议区分图标，双击 SSH 打开终端 / RDP 打开远程桌面，
 * 右键给更多动作。树形结构是渲染期从 `sessionTreeStore` 的扁平列表按 parent_id/
 * group_id 现算出来的，不是单独维护一份嵌套 state。
 */
export const SessionTree: React.FC = () => {
  const { groups, connections, loading, expanded, load, toggleExpanded, deleteGroup, deleteConnection, createConnection, updateConnection } =
    useSessionTreeStore();
  const openSshTerminal = useRemoteSessionStore((s) => s.openSshTerminal);
  const openAgentTerminal = useRemoteSessionStore((s) => s.openAgentTerminal);
  const openSftp = useRemoteSessionStore((s) => s.openSftp);
  const openAgentBrowse = useRemoteSessionStore((s) => s.openAgentBrowse);
  const openRdp = useRemoteSessionStore((s) => s.openRdp);
  const push = useToastStore((s) => s.push);

  const [menu, setMenu] = useState<{ x: number; y: number; target: MenuTarget } | null>(null);
  const [addMenu, setAddMenu] = useState<{ x: number; y: number } | null>(null);
  const [groupDialog, setGroupDialog] = useState<{ mode: "create" | "edit"; parentId: string | null; group?: ConnectionGroup } | null>(null);
  const [connectionDialog, setConnectionDialog] = useState<{ protocol: Protocol; groupId: string | null; connection?: ConnectionProfile } | null>(null);
  const [deleteGroupTarget, setDeleteGroupTarget] = useState<ConnectionGroup | null>(null);
  const [deleteConnectionTarget, setDeleteConnectionTarget] = useState<ConnectionProfile | null>(null);

  useEffect(() => {
    load().catch((e) => push("error", `加载会话列表失败：${formatError(e)}`));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 应用版本来自 Cargo.toml（tauri.conf.json 未写 version，Tauri 缺省读它，
  // RELEASING.md 里定的唯一权威来源），运行时用 getVersion() 取，不在前端另存一份
  // 容易和 Cargo.toml 脱节的版本号。
  const [appVersion, setAppVersion] = useState<string | null>(null);
  useEffect(() => {
    getVersion()
      .then(setAppVersion)
      .catch(() => setAppVersion(null));
  }, []);

  const childGroups = useMemo(() => {
    const map = new Map<string | null, ConnectionGroup[]>();
    for (const g of groups) {
      const key = g.parent_id;
      if (!map.has(key)) map.set(key, []);
      map.get(key)!.push(g);
    }
    for (const list of map.values()) list.sort((a, b) => a.name.localeCompare(b.name));
    return map;
  }, [groups]);

  const childConnections = useMemo(() => {
    const map = new Map<string | null, ConnectionProfile[]>();
    for (const c of connections) {
      const key = c.group_id;
      if (!map.has(key)) map.set(key, []);
      map.get(key)!.push(c);
    }
    for (const list of map.values()) list.sort((a, b) => a.name.localeCompare(b.name));
    return map;
  }, [connections]);

  const tabs = useRemoteSessionStore((s) => s.tabs);
  const activeProfileIds = useMemo(
    () => new Set(tabs.filter((t) => (t.kind === "ssh-terminal" || t.kind === "agent-terminal") && !t.disconnected).map((t) => t.profileId)),
    [tabs],
  );

  const openConnection = async (profile: ConnectionProfile) => {
    try {
      if (profile.protocol === "ssh") {
        await openSshTerminal(profile);
      } else if (profile.protocol === "agent") {
        await openAgentTerminal(profile);
      } else {
        openRdp(profile);
      }
    } catch (e) {
      push("error", `打开会话失败：${formatError(e)}`);
    }
  };

  const testAgentConnection = async (profile: ConnectionProfile) => {
    try {
      await agentService.connect(profile.id);
      push("success", `已连接到 ${profile.name}（TLS 握手 + 配对令牌校验通过）`);
    } catch (e) {
      push("error", `连接失败：${formatError(e)}`);
    }
  };

  const connectionMenuItems = (profile: ConnectionProfile): ContextMenuItem[] => {
    const items: ContextMenuItem[] =
      profile.protocol === "ssh"
        ? [
            { label: "打开终端", onClick: () => void openSshTerminal(profile).catch((e) => push("error", `打开终端失败：${formatError(e)}`)) },
            { label: "打开 SFTP", onClick: () => openSftp(profile) },
          ]
        : profile.protocol === "agent"
          ? [
              { label: "打开终端", onClick: () => void openAgentTerminal(profile).catch((e) => push("error", `打开终端失败：${formatError(e)}`)) },
              { label: "文件传输", onClick: () => openAgentBrowse(profile) },
              { label: "测试连接", onClick: () => void testAgentConnection(profile) },
            ]
          : [{ label: "打开 RDP", onClick: () => openRdp(profile) }];
    items.push(
      { label: "编辑", onClick: () => setConnectionDialog({ protocol: profile.protocol, groupId: profile.group_id, connection: profile }), separatorBefore: true },
      { label: "删除", onClick: () => setDeleteConnectionTarget(profile), danger: true },
    );
    return items;
  };

  const groupMenuItems = (group: ConnectionGroup): ContextMenuItem[] => [
    { label: "新建 SSH 会话", onClick: () => setConnectionDialog({ protocol: "ssh", groupId: group.id }) },
    { label: "新建 RDP 会话", onClick: () => setConnectionDialog({ protocol: "rdp", groupId: group.id }) },
    { label: "新建 Agent 会话", onClick: () => setConnectionDialog({ protocol: "agent", groupId: group.id }) },
    { label: "新建子分组", onClick: () => setGroupDialog({ mode: "create", parentId: group.id }) },
    { label: "重命名 / 移动", onClick: () => setGroupDialog({ mode: "edit", parentId: group.parent_id, group }), separatorBefore: true },
    { label: "删除分组", onClick: () => setDeleteGroupTarget(group), danger: true },
  ];

  const rootMenuItems: ContextMenuItem[] = [
    { label: "新建 SSH 会话", onClick: () => setConnectionDialog({ protocol: "ssh", groupId: null }) },
    { label: "新建 RDP 会话", onClick: () => setConnectionDialog({ protocol: "rdp", groupId: null }) },
    { label: "新建 Agent 会话", onClick: () => setConnectionDialog({ protocol: "agent", groupId: null }) },
    { label: "新建分组", onClick: () => setGroupDialog({ mode: "create", parentId: null }) },
  ];

  const submitConnection = async (value: ConnectionFormValue) => {
    const input: ConnectionProfileInput = {
      name: value.name,
      host: value.host,
      port: value.port,
      username: value.username,
      // RDP 表单不展示认证方式选择器（只有一种：密码），这里显式落成 "password"，
      // 不留着表单初始值 "key" 混进去误导以后读这条记录的人。
      auth_method: value.protocol === "rdp" ? "password" : value.authMethod,
      secret: value.secret || null,
      group_id: value.groupId || null,
      tags: [],
      jump_host_id: value.jumpHostId || null,
      protocol: value.protocol,
      options:
        value.protocol === "rdp"
          ? { domain: value.rdpDomain || undefined, width: value.rdpWidth, height: value.rdpHeight, color_depth: value.rdpColorDepth }
          : null,
    };
    try {
      if (connectionDialog?.connection) {
        await updateConnection(connectionDialog.connection.id, input);
      } else {
        await createConnection(input);
      }
      setConnectionDialog(null);
    } catch (e) {
      push("error", `保存会话失败：${formatError(e)}`);
    }
  };

  const renderConnection = (profile: ConnectionProfile, depth: number) => (
    <div
      key={profile.id}
      className="tree-item"
      style={{ paddingLeft: 8 + depth * 16 }}
      onDoubleClick={() => void openConnection(profile)}
      onContextMenu={(e) => {
        e.preventDefault();
        e.stopPropagation();
        setMenu({ x: e.clientX, y: e.clientY, target: { kind: "connection", connection: profile } });
      }}
      title={profile.protocol === "agent" ? `${profile.host}:${profile.port}（Agent）` : `${profile.username}@${profile.host}:${profile.port}`}
    >
      {profile.protocol === "ssh" ? (
        <TerminalIcon className="tree-icon" />
      ) : profile.protocol === "agent" ? (
        <HardDrive className="tree-icon" />
      ) : (
        <Monitor className="tree-icon" />
      )}
      <span className="tree-name">{profile.name}</span>
      {(profile.protocol === "ssh" || profile.protocol === "agent") && (
        <span className={`status-dot tree-status-dot ${activeProfileIds.has(profile.id) ? "connected" : "disconnected"}`} />
      )}
    </div>
  );

  const renderGroup = (group: ConnectionGroup, depth: number): React.ReactNode => {
    const isExpanded = expanded.has(group.id);
    return (
      <React.Fragment key={group.id}>
        <div
          className="tree-item"
          style={{ paddingLeft: 8 + depth * 16 }}
          onClick={() => toggleExpanded(group.id)}
          onContextMenu={(e) => {
            e.preventDefault();
            e.stopPropagation();
            setMenu({ x: e.clientX, y: e.clientY, target: { kind: "group", group } });
          }}
        >
          {isExpanded ? <FolderOpen className="tree-icon is-dir" /> : <Folder className="tree-icon is-dir" />}
          <span className="tree-name">{group.name}</span>
        </div>
        {isExpanded && (
          <>
            {(childGroups.get(group.id) ?? []).map((g) => renderGroup(g, depth + 1))}
            {(childConnections.get(group.id) ?? []).map((c) => renderConnection(c, depth + 1))}
          </>
        )}
      </React.Fragment>
    );
  };

  const rootGroups = childGroups.get(null) ?? [];
  const rootConnections = childConnections.get(null) ?? [];

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      <div className="session-tree-toolbar">
        <span>会话</span>
        <button className="session-tree-add" title="新建" onClick={(e) => setAddMenu({ x: e.clientX, y: e.clientY })}>
          <Plus />
        </button>
      </div>
      <div
        className="project-tree"
        onContextMenu={(e) => {
          if (e.target !== e.currentTarget) return;
          e.preventDefault();
          setMenu({ x: e.clientX, y: e.clientY, target: { kind: "root" } });
        }}
      >
        {loading ? (
          <div style={{ padding: 16, fontSize: 12, color: "var(--text-secondary)" }}>加载中…</div>
        ) : rootGroups.length === 0 && rootConnections.length === 0 ? (
          <div style={{ padding: 16, fontSize: 12, color: "var(--text-secondary)" }}>
            还没有会话，点右上角 <FolderCog style={{ width: 12, height: 12, display: "inline", verticalAlign: -1 }} /> 新建一个
          </div>
        ) : (
          <>
            {rootGroups.map((g) => renderGroup(g, 0))}
            {rootConnections.map((c) => renderConnection(c, 0))}
          </>
        )}
      </div>

      <div className="session-tree-footer" title={`roc_desk v${appVersion ?? "?"}，构建于 ${__APP_BUILD_DATE__}`}>
        roc_desk v{appVersion ?? "…"} · {__APP_BUILD_DATE__}
      </div>

      {addMenu && (
        <ContextMenu x={addMenu.x} y={addMenu.y} items={rootMenuItems} onClose={() => setAddMenu(null)} />
      )}
      {menu && (
        <ContextMenu
          x={menu.x}
          y={menu.y}
          items={
            menu.target.kind === "group"
              ? groupMenuItems(menu.target.group)
              : menu.target.kind === "connection"
                ? connectionMenuItems(menu.target.connection)
                : rootMenuItems
          }
          onClose={() => setMenu(null)}
        />
      )}

      {groupDialog && (
        <GroupDialog
          title={groupDialog.mode === "create" ? "新建分组" : "重命名 / 移动分组"}
          initial={groupDialog.group}
          initialParentId={groupDialog.parentId}
          groups={groups.filter((g) => g.id !== groupDialog.group?.id)}
          onCancel={() => setGroupDialog(null)}
          onSave={async ({ name, parentId }) => {
            try {
              if (groupDialog.mode === "edit" && groupDialog.group) {
                await useSessionTreeStore.getState().renameGroup(groupDialog.group.id, name);
                if (parentId !== groupDialog.group.parent_id) {
                  await useSessionTreeStore.getState().moveGroup(groupDialog.group.id, parentId);
                }
              } else {
                await useSessionTreeStore.getState().createGroup({ name, parent_id: parentId });
              }
              setGroupDialog(null);
            } catch (e) {
              push("error", `保存分组失败：${formatError(e)}`);
            }
          }}
        />
      )}

      {connectionDialog && (
        <ConnectionDialog
          title={
            connectionDialog.connection
              ? "编辑会话"
              : connectionDialog.protocol === "ssh"
                ? "新建 SSH 会话"
                : connectionDialog.protocol === "agent"
                  ? "新建 Agent 会话"
                  : "新建 RDP 会话"
          }
          initial={connectionDialog.connection}
          defaultProtocol={connectionDialog.protocol}
          defaultGroupId={connectionDialog.groupId}
          groups={groups}
          connections={connections}
          onCancel={() => setConnectionDialog(null)}
          onSave={submitConnection}
        />
      )}

      {deleteGroupTarget && (
        <ConfirmDialog
          open
          severity="danger"
          icon="🗑"
          title="删除分组"
          onDismiss={() => setDeleteGroupTarget(null)}
          actions={
            <>
              <button className="btn ghost sm" onClick={() => setDeleteGroupTarget(null)}>取消</button>
              <button
                className="btn danger-strong sm"
                onClick={async () => {
                  const target = deleteGroupTarget;
                  setDeleteGroupTarget(null);
                  try {
                    await deleteGroup(target.id);
                  } catch (e) {
                    push("error", `删除分组失败：${formatError(e)}`);
                  }
                }}
              >
                删除
              </button>
            </>
          }
        >
          <p>
            确定要删除分组 <strong>{deleteGroupTarget.name}</strong> 吗？里面的会话会移到未分组，子分组会上移一级，不会被一起删除。
          </p>
        </ConfirmDialog>
      )}

      {deleteConnectionTarget && (
        <ConfirmDialog
          open
          severity="danger"
          icon="🗑"
          title="删除会话"
          onDismiss={() => setDeleteConnectionTarget(null)}
          actions={
            <>
              <button className="btn ghost sm" onClick={() => setDeleteConnectionTarget(null)}>取消</button>
              <button
                className="btn danger-strong sm"
                onClick={async () => {
                  const target = deleteConnectionTarget;
                  setDeleteConnectionTarget(null);
                  try {
                    await deleteConnection(target.id);
                  } catch (e) {
                    push("error", `删除会话失败：${formatError(e)}`);
                  }
                }}
              >
                删除
              </button>
            </>
          }
        >
          <p>确定要删除会话 <strong>{deleteConnectionTarget.name}</strong> 吗？此操作不可撤销。</p>
        </ConfirmDialog>
      )}
    </div>
  );
};
