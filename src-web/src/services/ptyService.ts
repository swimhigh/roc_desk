import { invoke } from "@tauri-apps/api/core";

/** 本地终端 IPC 封装（DESIGN.md §3.2 本地工作区分支），和 sshService 的形状对齐。*/
export const ptyService = {
  open(cwd: string, rows: number, cols: number): Promise<string> {
    return invoke("pty_open", { cwd, rows, cols });
  },
  write(channelId: string, data: Uint8Array): Promise<void> {
    return invoke("pty_write", { channelId, data: Array.from(data) });
  },
  resize(channelId: string, rows: number, cols: number): Promise<void> {
    return invoke("pty_resize", { channelId, rows, cols });
  },
  close(channelId: string): Promise<void> {
    return invoke("pty_close", { channelId });
  },
};
