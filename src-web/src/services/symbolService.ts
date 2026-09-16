import { invoke } from "@tauri-apps/api/core";
import type { SymbolLocation } from "../types/bindings";

/** 符号索引（"转到定义/声明"，2026-09-16 需求）：后端是自己写的轻量正则符号
 * 扫描器（不是真正的 ctags/LSP），只覆盖工作区场景——游离文件没有"项目"边界，
 * 不接这个功能。 */
export const symbolService = {
  /** 打开工作区时调一次，全量扫描重建索引。返回索引到的符号总数，目前前端没有
   * 用来展示，失败也不影响正常编辑。 */
  buildIndex(workspaceId: string): Promise<number> {
    return invoke("symbols_build_index", { workspaceId });
  },
  goToDefinition(workspaceId: string, symbol: string): Promise<SymbolLocation[]> {
    return invoke("symbols_go_to_definition", { workspaceId, symbol });
  },
  /** 文件保存成功后调用，增量重建这一个文件的条目，不用重新扫全工作区。 */
  reindexFile(workspaceId: string, path: string, content: string): Promise<void> {
    return invoke("symbols_reindex_file", { workspaceId, path, content });
  },
};
