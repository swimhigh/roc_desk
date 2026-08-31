/** Windows（Agent 目标）路径工具——AGENT_DESIGN.md：目标机器没有单一根目录 "/"
 * 的概念，用空字符串表示"此电脑"下的盘符列表这一虚拟层级，选中某个盘符
 * （如 "C:\\"）之后才是真正的目录路径。RemoteWorkspaceDialog（工作区挂载向导）
 * 和 AgentBrowser（自由浏览快捷工具）都要用同一套导航逻辑，抽成共享工具避免
 * 两处各写一份、行为慢慢漂移。 */
export const AGENT_ROOT = "";

export function isAgentRoot(path: string): boolean {
  return path === AGENT_ROOT;
}

/** 计算 Windows 路径的上一级：盘符根目录（"C:\\"）的上一级是虚拟的盘符列表
 * （AGENT_ROOT），不是继续往上"越过"盘符本身。 */
export function agentParentPath(path: string): string {
  const trimmed = path.replace(/\\+$/, "");
  const lastSep = trimmed.lastIndexOf("\\");
  if (lastSep <= 2) {
    // "C:" 或 "C:\xxx" 只有一层时，上一级是盘符列表；`lastSep <= 2` 覆盖
    // "C:\xxx"（分隔符正好在下标 2）和找不到分隔符（-1）两种情况。
    return AGENT_ROOT;
  }
  return trimmed.slice(0, lastSep);
}
