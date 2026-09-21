const KNOWN_METHODS = new Set(["get", "post", "put", "patch", "delete", "head", "options"]);

/** HTTP 方法 → CSS 类名后缀，供 `.http-method-*`/`.http-method-chip.http-method-*`
 * 系列样式使用（components.css「HTTP 测试工作台」一节）。未知方法（比如用户
 * 手填了别的字符串）统一退到 options 的中性灰，不报错也不留空白类名。 */
export function httpMethodClass(method: string): string {
  const m = method.trim().toLowerCase();
  return `http-method-${KNOWN_METHODS.has(m) ? m : "options"}`;
}
