import { invoke } from "@tauri-apps/api/core";

export const sshService = {
  connect(profileId: string): Promise<string> {
    return invoke("ssh_connect", { profileId });
  },
  openShell(profileId: string, rows: number, cols: number, cwd?: string): Promise<string> {
    return invoke("ssh_open_shell", { profileId, rows, cols, cwd: cwd ?? null });
  },
  write(profileId: string, channelId: string, data: Uint8Array): Promise<void> {
    return invoke("ssh_write", { profileId, channelId, data: Array.from(data) });
  },
  resize(profileId: string, channelId: string, rows: number, cols: number): Promise<void> {
    return invoke("ssh_resize", { profileId, channelId, rows, cols });
  },
  disconnect(profileId: string): Promise<void> {
    return invoke("ssh_disconnect", { profileId });
  },
  closeChannel(profileId: string, channelId: string): Promise<void> {
    return invoke("ssh_close_channel", { profileId, channelId });
  },
  confirmHostKey(requestId: string, trust: boolean): Promise<void> {
    return invoke("ssh_confirm_host_key", { requestId, trust });
  },
};
