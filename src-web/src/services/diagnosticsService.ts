import { invoke } from "@tauri-apps/api/core";

/**
 * 前端未捕获异常上报后端日志（main.tsx 的全局 error/unhandledrejection 监听、
 * ErrorBoundary 组件是调用方）。故意不 await、不向上抛错——上报本身失败也不能
 * 再抛一次异常把调用方拖下水，那就成了"报错处理器自己也在报错"的死循环。
 */
export function logFrontendError(message: string, stack?: string, source?: string): void {
  invoke("log_frontend_error", { message, stack, source }).catch(() => {});
}
