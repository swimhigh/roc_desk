import { invoke } from "@tauri-apps/api/core";
import type { BinaryInfo, FileContent, FileEntry, JarInfo, WriteOutcome } from "../types/bindings";

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
  /** 图片/PDF/Word/Excel 预览：返回 base64（不含 data: 前缀），前端自己按扩展名分流。*/
  readBinaryPreview(profileId: string, path: string): Promise<string> {
    return invoke("sftp_read_binary_preview", { profileId, path });
  },

  /** "用系统默认程序打开"——一律先下载到本地临时目录再拉起系统程序。*/
  openExternally(profileId: string, path: string): Promise<void> {
    return invoke("sftp_open_externally", { profileId, path });
  },

  /** 旧版二进制 Office 文档用本机 LibreOffice 临时转成 PDF 预览，语义和
   * `fsService.convertLegacyOfficeToPdf` 一致。*/
  convertLegacyOfficeToPdf(profileId: string, path: string): Promise<string> {
    return invoke("sftp_convert_legacy_office_to_pdf", { profileId, path });
  },

  /** EXE/DLL/SO 等可执行文件的基本信息 + 依赖库列表（2026-08-28 需求）。*/
  inspectBinary(profileId: string, path: string): Promise<BinaryInfo> {
    return invoke("sftp_inspect_binary", { profileId, path });
  },

  /** 没有已知可执行文件扩展名的文件，打开前嗅探开头几个字节判断是不是 ELF/PE/
   * Mach-O（2026-08-28 用户反馈：Linux 远程主机上的可执行文件习惯上不带扩展名）。*/
  peekIsBinary(profileId: string, path: string): Promise<boolean> {
    return invoke("sftp_peek_is_binary", { profileId, path });
  },

  /** JAR 包的基本信息（manifest/Main-Class/Class-Path）+ 内部条目列表（2026-08-28 需求）。*/
  inspectJar(profileId: string, path: string): Promise<JarInfo> {
    return invoke("sftp_inspect_jar", { profileId, path });
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
