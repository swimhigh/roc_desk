import React from "react";
import { ConnectionForm, type ConnectionFormValue } from "../ConnectionManager/ConnectionForm";
import type { ConnectionGroup, ConnectionProfile, Protocol } from "../../types/bindings";

interface ConnectionDialogProps {
  title: string;
  initial?: ConnectionProfile;
  defaultProtocol?: Protocol;
  /** 新建模式下（没有 `initial`）分组下拉的初始值——从某个分组的右键菜单点"新建
   * SSH/RDP 会话"，这里就是那个分组的 id。*/
  defaultGroupId?: string | null;
  groups: ConnectionGroup[];
  connections: ConnectionProfile[];
  onCancel: () => void;
  onSave: (value: ConnectionFormValue) => void;
}

/** 新建/编辑一个会话（SSH 或 RDP）的弹窗外壳，ConnectionForm 本身没有 Dialog 样式的
 * chrome（RemoteWorkspaceDialog 里是内嵌在向导的某一步，这里要单独弹出）。 */
export const ConnectionDialog: React.FC<ConnectionDialogProps> = ({
  title,
  initial,
  defaultProtocol,
  defaultGroupId,
  groups,
  connections,
  onCancel,
  onSave,
}) => {
  return (
    <div className="dialog-overlay" role="dialog" aria-modal="true">
      <div className="dialog" style={{ minWidth: 420, maxWidth: 480 }}>
        <div className="dialog-title-bar info">
          <span>🖥</span>
          <span>{title}</span>
        </div>
        <div className="dialog-body">
          <ConnectionForm
            initial={
              initial
                ? {
                    name: initial.name,
                    host: initial.host,
                    port: initial.port,
                    username: initial.username,
                    authMethod: initial.auth_method,
                    secret: "",
                    groupId: initial.group_id ?? undefined,
                    jumpHostId: initial.jump_host_id ?? undefined,
                    protocol: initial.protocol,
                    rdpDomain: initial.options?.domain,
                    rdpWidth: initial.options?.width,
                    rdpHeight: initial.options?.height,
                    rdpColorDepth: initial.options?.color_depth,
                  }
                : defaultGroupId
                  ? { groupId: defaultGroupId }
                  : undefined
            }
            groups={groups.map((g) => ({ id: g.id, name: g.name }))}
            jumpHostOptions={connections.filter((c) => c.protocol === "ssh" && c.id !== initial?.id).map((c) => ({ id: c.id, name: c.name }))}
            fixedProtocol={initial ? initial.protocol : defaultProtocol}
            onCancel={onCancel}
            onSave={onSave}
          />
        </div>
      </div>
    </div>
  );
};
