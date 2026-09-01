import { invoke } from "@tauri-apps/api/core";
import type { TransferLogEntry } from "../types/bindings";

/** SFTP/Agent 双栏浏览器共用——"停止传输"和"传输历史查询"都是协议无关的
 * 命令（后端按 `request_id` 找取消标记、日志表本身也不分协议存），不需要
 * 分成 sftpService/agentService 各写一份。*/
export const transferService = {
  cancel(requestId: string): Promise<void> {
    return invoke("transfer_cancel", { requestId });
  },
  listLog(limit: number, offset: number, search?: string): Promise<TransferLogEntry[]> {
    return invoke("transfer_log_list", { limit, offset, search: search || null });
  },
  clearLog(): Promise<void> {
    return invoke("transfer_log_clear");
  },
};
