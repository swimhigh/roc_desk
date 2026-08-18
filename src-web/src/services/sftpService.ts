import { invoke } from "@tauri-apps/api/core";
import type { FileContent, FileEntry, WriteOutcome } from "../types/bindings";

/**
 * SFTP 自由浏览快捷工具（DESIGN.md §3.3）。与 fsService 的区别：这里按 `profileId`
 * （而不是 `workspaceId`）寻址，后端不做工作区边界检查，可以看工作区之外的路径。
 */
export const sftpService = {
  listDir(profileId: string, path: string): Promise<FileEntry[]> {
    return invoke("sftp_list_dir", { profileId, path });
  },
  readFile(profileId: string, path: string): Promise<FileContent> {
    return invoke("sftp_read_file", { profileId, path });
  },
  writeFile(profileId: string, path: string, content: string, expectedMtime: number | null): Promise<WriteOutcome> {
    return invoke("sftp_write_file", { profileId, path, content, expectedMtime });
  },
  download(profileId: string, remotePath: string, localPath: string): Promise<void> {
    return invoke("sftp_download", { profileId, remotePath, localPath });
  },
  upload(profileId: string, localPath: string, remotePath: string): Promise<void> {
    return invoke("sftp_upload", { profileId, localPath, remotePath });
  },
  /** 双栏浏览器用：目标名沿用原名，落在 localDir 下面；目录会递归传输，过程中按
   * requestId 发 sftp:transfer-progress 事件（粗粒度，按完成的文件数，不是字节百分比）。*/
  downloadEntry(profileId: string, remotePath: string, isDir: boolean, localDir: string, requestId: string): Promise<void> {
    return invoke("sftp_download_entry", { profileId, remotePath, isDir, localDir, requestId });
  },
  uploadEntry(profileId: string, localPath: string, isDir: boolean, remoteDir: string, requestId: string): Promise<void> {
    return invoke("sftp_upload_entry", { profileId, localPath, isDir, remoteDir, requestId });
  },
  delete(profileId: string, path: string, isDir: boolean): Promise<void> {
    return invoke("sftp_delete", { profileId, path, isDir });
  },
  rename(profileId: string, from: string, to: string): Promise<void> {
    return invoke("sftp_rename", { profileId, from, to });
  },
};
