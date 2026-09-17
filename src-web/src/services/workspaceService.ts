import { invoke } from "@tauri-apps/api/core";
import type { WorkspaceProfile } from "../types/bindings";

/** IPC 边界：唯一允许调用 invoke() 的地方之一（CODE_DESIGN.md §一分层原则）。 */
export const workspaceService = {
  // 工作区选择页和首页都使用带滚动条的最近列表；100 条足以覆盖日常历史，
  // 同时避免一次性渲染无限多的旧记录。
  listRecent(limit = 100): Promise<WorkspaceProfile[]> {
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

  /** 某个工作模块（"workspace"/"http"）首页卡片/选择页展示的"添加过的工作区"
   * 列表——默认是空的，要显式 `addModuleLink` 才会出现（2026-09 需求：不同
   * 模块不该共用同一份"最近工作区"，各自维护自己的子集）。 */
  listForModule(module: string, limit = 100): Promise<WorkspaceProfile[]> {
    return invoke("workspace_list_for_module", { module, limit });
  },

  addModuleLink(id: string, module: string): Promise<void> {
    return invoke("workspace_add_module_link", { id, module });
  },

  /** 只解除这个工作区和该模块的关联，不删除工作区本身——同一个工作区可能还
   * 关联着其它模块。彻底忘记一个工作区仍然用 `removeRecent`。 */
  removeModuleLink(id: string, module: string): Promise<void> {
    return invoke("workspace_remove_module_link", { id, module });
  },
};
