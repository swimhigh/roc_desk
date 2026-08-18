import { invoke } from "@tauri-apps/api/core";
import type { FileContent, FileEntry, WriteOutcome } from "../types/bindings";

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
};
