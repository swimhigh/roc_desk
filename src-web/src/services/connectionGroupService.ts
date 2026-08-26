import { invoke } from "@tauri-apps/api/core";
import type { ConnectionGroup, ConnectionGroupInput } from "../types/bindings";

export const connectionGroupService = {
  list(): Promise<ConnectionGroup[]> {
    return invoke("connection_group_list");
  },
  create(input: ConnectionGroupInput): Promise<ConnectionGroup> {
    return invoke("connection_group_create", { input });
  },
  /** 重命名 + 移动到另一个父分组共用这一个方法（parent_id 不变就是纯重命名）。 */
  update(id: string, input: ConnectionGroupInput): Promise<ConnectionGroup> {
    return invoke("connection_group_update", { id, input });
  },
  delete(id: string): Promise<void> {
    return invoke("connection_group_delete", { id });
  },
};
