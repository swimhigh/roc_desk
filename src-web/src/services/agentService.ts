import { invoke } from "@tauri-apps/api/core";
import type { FileContent, FileEntry, WriteOutcome } from "../types/bindings";

/** 远程 Windows Agent（AGENT_DESIGN.md）连接的命令封装，和 sshService.ts 是同一种模式。 */
export const agentService = {
  connect(profileId: string): Promise<string> {
    return invoke("agent_connect", { profileId });
  },
  disconnect(profileId: string): Promise<void> {
    return invoke("agent_disconnect", { profileId });
  },
  confirmCert(requestId: string, trust: boolean): Promise<void> {
    return invoke("agent_confirm_cert", { requestId, trust });
  },
  /** 连接设置对话框"测试连接"按钮：直接测表单里当前填的值，不需要先保存连接、
   * 不做证书指纹持久化——纯粹验证这几个字段填得对不对，成功时返回一句可以直接
   * toast 出来的结果文本。 */
  testConnection(host: string, port: number, token: string): Promise<string> {
    return invoke("agent_test_connection", { host, port, token });
  },
  listDir(profileId: string, path: string): Promise<FileEntry[]> {
    return invoke("agent_list_dir", { profileId, path });
  },
  /** Windows 盘符列表（"此电脑"下的 C:\、D:\ ...），工作区挂载向导浏览到根时用它
   * 代替 `listDir`——Agent 目标没有单一根目录 "/" 的概念。 */
  listRoots(profileId: string): Promise<string[]> {
    return invoke("agent_list_roots", { profileId });
  },
  /** 交互式终端（AGENT_DESIGN.md §四.4 Phase 2），和 sshService 的四个终端方法
   * 一一对应，`TerminalView` 复用同一套渲染逻辑。 */
  openShell(profileId: string, rows: number, cols: number, cwd?: string): Promise<string> {
    return invoke("agent_open_shell", { profileId, rows, cols, cwd: cwd ?? null });
  },
  write(profileId: string, channelId: string, data: Uint8Array): Promise<void> {
    return invoke("agent_write", { profileId, channelId, data: Array.from(data) });
  },
  resize(profileId: string, channelId: string, rows: number, cols: number): Promise<void> {
    return invoke("agent_resize", { profileId, channelId, rows, cols });
  },
  closeChannel(profileId: string, channelId: string): Promise<void> {
    return invoke("agent_close_channel", { profileId, channelId });
  },

  // --- 自由浏览快捷工具（AGENT_DESIGN.md §四.3），和 sftpService.ts 是同一种用途，
  // 分开一套方法是因为底层协议不同（Agent vs SFTP）。 ---
  readFile(profileId: string, path: string): Promise<FileContent> {
    return invoke("agent_read_file", { profileId, path });
  },
  writeFile(profileId: string, path: string, content: string, expectedMtime: number | null): Promise<WriteOutcome> {
    return invoke("agent_write_file", { profileId, path, content, expectedMtime });
  },
  delete(profileId: string, path: string, isDir: boolean): Promise<void> {
    return invoke("agent_delete", { profileId, path, isDir });
  },
  rename(profileId: string, from: string, to: string): Promise<void> {
    return invoke("agent_rename", { profileId, from, to });
  },
  download(profileId: string, remotePath: string, localPath: string): Promise<void> {
    return invoke("agent_download", { profileId, remotePath, localPath });
  },
  upload(profileId: string, localPath: string, remotePath: string): Promise<void> {
    return invoke("agent_upload", { profileId, localPath, remotePath });
  },
  downloadEntry(profileId: string, remotePath: string, isDir: boolean, localDir: string, requestId: string): Promise<void> {
    return invoke("agent_download_entry", { profileId, remotePath, isDir, localDir, requestId });
  },
  uploadEntry(profileId: string, localPath: string, isDir: boolean, remoteDir: string, requestId: string): Promise<void> {
    return invoke("agent_upload_entry", { profileId, localPath, isDir, remoteDir, requestId });
  },
};
