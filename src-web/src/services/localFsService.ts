import { invoke } from "@tauri-apps/api/core";
import type { FileEntry } from "../types/bindings";

/** SFTP 双栏浏览器的本地一侧（DESIGN.md §3.3），不受工作区边界限制，和 sftpService 对称。*/
export const localFsService = {
  listDir(path: string): Promise<FileEntry[]> {
    return invoke("local_list_dir", { path });
  },
  homeDir(): Promise<string> {
    return invoke("local_home_dir");
  },
};
