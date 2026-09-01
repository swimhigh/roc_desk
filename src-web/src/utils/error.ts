import { isAppError } from "../types/bindings";

/** 和 Rust 侧 `fsops::TRANSFER_CANCELLED_MESSAGE` 是同一个哨兵字符串——SFTP/Agent
 * 双栏浏览器点"停止"之后，`copy_between`/`download_recursive`/`upload_recursive`
 * 会用这条消息中断传输，前端拿到这个特定错误要展示成"已取消"而不是"传输失败"，
 * 不能让用户误以为是网络/权限问题。 */
const TRANSFER_CANCELLED_MESSAGE = "传输已取消";

export function isCancelledTransferError(e: unknown): boolean {
  return isAppError(e) && e.message === TRANSFER_CANCELLED_MESSAGE;
}

const KIND_LABEL: Record<string, string> = {
  Connection: "连接失败",
  Auth: "认证失败",
  HostKeyRejected: "主机指纹校验被拒绝",
  PermissionDenied: "权限不足",
  NotFound: "未找到",
  Database: "数据库错误",
  Conflict: "冲突",
  Internal: "内部错误",
};

/**
 * Tauri `invoke()` 的 rejection 是后端 `AppError` 序列化后的裸对象
 * `{ kind, message }`，不是 JS `Error` 实例——直接 `String(e)` 会得到
 * "[object Object]"。所有 catch 分支都要走这里，不要各自 `String(e)`。
 */
export function formatError(e: unknown): string {
  if (isAppError(e)) {
    const label = KIND_LABEL[e.kind];
    return label ? `${label}：${e.message}` : e.message;
  }
  if (e instanceof Error) return e.message;
  if (typeof e === "string") return e;
  try {
    return JSON.stringify(e);
  } catch {
    return String(e);
  }
}
