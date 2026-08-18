import { invoke } from "@tauri-apps/api/core";
import type { FileContent, FileEntry, ReplaceSummary, SearchMode, SearchOptions, WriteOutcome } from "../types/bindings";

export const fsService = {
  listDir(workspaceId: string, path: string): Promise<FileEntry[]> {
    return invoke("fs_list_dir", { workspaceId, path });
  },

  readFile(workspaceId: string, path: string): Promise<FileContent> {
    return invoke("fs_read_file", { workspaceId, path });
  },

  writeFile(
    workspaceId: string,
    path: string,
    content: string,
    expectedMtime: number | null,
  ): Promise<WriteOutcome> {
    return invoke("fs_write_file", {
      workspaceId,
      path,
      content,
      expectedMtime,
    });
  },

  /** "Reopen with Encoding"（参考 VS Code）：忽略自动探测，强制按指定编码重新读取。*/
  readFileWithEncoding(workspaceId: string, path: string, encodingLabel: string): Promise<FileContent> {
    return invoke("fs_read_file_with_encoding", { workspaceId, path, encodingLabel });
  },

  /** "Save with Encoding"（参考 VS Code）：按指定编码写盘，而不是固定 UTF-8。*/
  writeFileWithEncoding(
    workspaceId: string,
    path: string,
    content: string,
    encodingLabel: string,
    expectedMtime: number | null,
  ): Promise<WriteOutcome> {
    return invoke("fs_write_file_with_encoding", { workspaceId, path, content, encodingLabel, expectedMtime });
  },

  supportedEncodings(): Promise<string[]> {
    return invoke("fs_supported_encodings");
  },

  deleteFile(workspaceId: string, path: string, isDir: boolean): Promise<void> {
    return invoke("fs_delete", { workspaceId, path, isDir });
  },

  rename(workspaceId: string, from: string, to: string): Promise<void> {
    return invoke("fs_rename", { workspaceId, from, to });
  },

  /** 剪切+粘贴的移动就是换个目标路径调 rename；复制目前只支持文件，见后端 `FileOps::copy` 注释。*/
  copy(workspaceId: string, from: string, to: string, isDir: boolean): Promise<void> {
    return invoke("fs_copy", { workspaceId, from, to, isDir });
  },

  /** 左侧目录树的全文搜索（参考 VS Code 搜索面板）：不直接返回结果，结果通过
   * `search:file-result`/`search:done`/`search:error` 事件流式推送（2026-08-18
   * 需求："能否一个一个目录搜，搜到一部分先展示一部分"），这里只是触发一次搜索。
   * `scopePath` 为空时搜整个工作区，传了就只搜这个子目录（右键"在此文件夹中搜索"）。*/
  searchStream(
    workspaceId: string,
    requestId: string,
    scopePath: string | null,
    query: string,
    mode: SearchMode,
    options: SearchOptions,
  ): Promise<void> {
    return invoke("fs_search_stream", { workspaceId, requestId, scopePath, query, mode, options });
  },

  /** 查找并替换全部——paths 是搜索结果里的文件路径，不重新在后端搜一遍。 */
  replace(
    workspaceId: string,
    paths: string[],
    query: string,
    replacement: string,
    options: SearchOptions,
  ): Promise<ReplaceSummary> {
    return invoke("fs_replace", { workspaceId, paths, query, replacement, options });
  },
};
