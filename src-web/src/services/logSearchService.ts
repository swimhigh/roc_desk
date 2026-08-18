import { invoke } from "@tauri-apps/api/core";
import type { IndexStats, LiveSearchResult, LogQuery, LogSearchResult } from "../types/bindings";

/** IPC 边界（CODE_DESIGN.md §一分层原则）：日志搜索的两种模式——本地 FTS5 索引 / 远程实时 rg-grep。*/
export const logSearchService = {
  searchIndex(query: LogQuery): Promise<LogSearchResult[]> {
    return invoke("log_search_index", { query });
  },
  searchLive(profileId: string, pattern: string, path: string, isRegex: boolean): Promise<LiveSearchResult[]> {
    return invoke("log_search_live", { profileId, pattern, path, isRegex });
  },
  importFile(profileId: string, remotePath: string, hostName: string): Promise<number> {
    return invoke("log_import_file", { profileId, remotePath, hostName });
  },
  importLocalFile(path: string, hostName: string): Promise<number> {
    return invoke("log_import_local_file", { path, hostName });
  },
  indexStats(): Promise<IndexStats> {
    return invoke("log_index_stats");
  },
  indexClear(olderThanDays: number): Promise<number> {
    return invoke("log_index_clear", { olderThanDays });
  },
};
