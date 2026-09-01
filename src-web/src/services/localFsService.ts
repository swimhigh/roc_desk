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
  /** 从外部拖真实文件进来时用——判断是文件还是目录，决定走单文件还是整目录上传。*/
  isDir(path: string): Promise<boolean> {
    return invoke("local_is_dir", { path });
  },
};
