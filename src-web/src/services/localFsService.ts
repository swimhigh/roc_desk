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
  /** 资源管理器模块的盘符切换（Windows 下 C:/D:/... ）。 */
  listDrives(): Promise<string[]> {
    return invoke("local_list_drives");
  },
  /** 从外部拖真实文件进来时用——判断是文件还是目录，决定走单文件还是整目录上传。*/
  isDir(path: string): Promise<boolean> {
    return invoke("local_is_dir", { path });
  },

  /** 资源管理器模块（Total Commander 式本地双栏，`docs/HOME_MODES_DESIGN.md` §3.2/§6
   * Phase 4）的文件操作——和上面几个一样不需要工作区，用户就是要在任意本地目录
   * 之间复制/移动/删除。 */
  deletePath(path: string, isDir: boolean): Promise<void> {
    return invoke("local_delete", { path, isDir });
  },

  rename(from: string, to: string): Promise<void> {
    return invoke("local_rename", { from, to });
  },

  copy(from: string, to: string, isDir: boolean): Promise<void> {
    return invoke("local_copy", { from, to, isDir });
  },

  /** 同盘先走原子 `rename`，跨盘自动退化成"复制后删除源"，见后端 `local_move` 注释。*/
  move(from: string, to: string, isDir: boolean): Promise<void> {
    return invoke("local_move", { from, to, isDir });
  },

  createDir(path: string): Promise<void> {
    return invoke("local_create_dir", { path });
  },
};
