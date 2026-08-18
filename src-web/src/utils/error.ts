import { isAppError } from "../types/bindings";

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
