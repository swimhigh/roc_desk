import { invoke } from "@tauri-apps/api/core";
import type { SkillMeta } from "../types/bindings";

/** IPC 边界：项目 Skills 查看/导入（`.rock_desk/skills/<name>/SKILL.md`）。 */
export const skillService = {
  list(workspaceId: string): Promise<SkillMeta[]> {
    return invoke("skill_list", { workspaceId });
  },
  /** `localPath` 是本地磁盘上一个含 `SKILL.md` 的目录，由前端文件选择器选中。 */
  import(workspaceId: string, localPath: string): Promise<SkillMeta> {
    return invoke("skill_import", { workspaceId, localPath });
  },
  delete(workspaceId: string, name: string): Promise<void> {
    return invoke("skill_delete", { workspaceId, name });
  },
};
