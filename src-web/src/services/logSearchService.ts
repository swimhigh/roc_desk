import { invoke } from "@tauri-apps/api/core";
import type { IndexStats, LiveSearchResult, LogImportOutcome, LogQuery, LogSearchResult } from "../types/bindings";

/** IPC 边界（CODE_DESIGN.md §一分层原则）：日志搜索的两种模式——本地 FTS5 索引 / 远程实时 rg-grep。*/
export const logSearchService = {
  searchIndex(query: LogQuery): Promise<LogSearchResult[]> {
    return invoke("log_search_index", { query });
  },
  searchLive(profileId: string, pattern: string, path: string, isRegex: boolean): Promise<LiveSearchResult[]> {
    return invoke("log_search_live", { profileId, pattern, path, isRegex });
  },
  /** `paths` 可以混着传文件和目录；`recursive` 为 false 时传目录会报错（后端不会
   * 帮忙猜"导入这个目录下哪个文件"）。`requestId` 用来在 `log:import-progress`
   * 事件里过滤出这一次导入自己的进度。 */
  importRemotePaths(profileId: string, paths: string[], recursive: boolean, hostName: string, requestId: string): Promise<LogImportOutcome> {
    return invoke("log_import_remote_paths", { profileId, paths, recursive, hostName, requestId });
  },
  importLocalPaths(paths: string[], recursive: boolean, hostName: string, requestId: string): Promise<LogImportOutcome> {
    return invoke("log_import_local_paths", { paths, recursive, hostName, requestId });
  },
  indexStats(): Promise<IndexStats> {
    return invoke("log_index_stats");
  },
  indexClear(olderThanDays: number): Promise<number> {
    return invoke("log_index_clear", { olderThanDays });
  },
};
