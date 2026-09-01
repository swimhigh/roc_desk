import { invoke } from "@tauri-apps/api/core";
import type { WorkspaceProfile } from "../types/bindings";

/** IPC 边界：唯一允许调用 invoke() 的地方之一（CODE_DESIGN.md §一分层原则）。 */
export const workspaceService = {
  listRecent(limit = 20): Promise<WorkspaceProfile[]> {
    return invoke("workspace_list_recent", { limit });
  },

  openLocal(path: string): Promise<WorkspaceProfile> {
    return invoke("workspace_open_local", { path });
  },

  openRemote(connectionId: string, remotePath: string): Promise<WorkspaceProfile> {
    return invoke("workspace_open_remote", { connectionId, remotePath });
  },

  removeRecent(id: string): Promise<void> {
    return invoke("workspace_remove_recent", { id });
  },

  updatePath(id: string, newPath: string): Promise<WorkspaceProfile> {
    return invoke("workspace_update_path", { id, newPath });
  },

  close(id: string): Promise<void> {
    return invoke("workspace_close", { id });
  },

  /** SFTP/Agent 双栏浏览器每次导航都调一次，把两边当前目录写回工作区档案——
   * 下次重新打开这个工作区的 SFTP/文件传输时直接定位到这里，不用重新导航。 */
  updateLastSftpPaths(id: string, localPath: string, remotePath: string): Promise<void> {
    return invoke("workspace_update_last_sftp_paths", { id, localPath, remotePath });
  },
};
