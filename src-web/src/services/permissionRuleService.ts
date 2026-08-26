import { invoke } from "@tauri-apps/api/core";
import type { PermissionRule, PermissionRuleInput } from "../types/bindings";

/** IPC 边界：AI 编程助手的权限规则（allow/ask/deny，REQUIREMENTS.md §3.7）。 */
export const permissionRuleService = {
  list(): Promise<PermissionRule[]> {
    return invoke("permission_rule_list");
  },
  create(input: PermissionRuleInput): Promise<PermissionRule> {
    return invoke("permission_rule_create", { input });
  },
  delete(id: string): Promise<void> {
    return invoke("permission_rule_delete", { id });
  },
};
