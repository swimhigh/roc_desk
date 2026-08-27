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
};
