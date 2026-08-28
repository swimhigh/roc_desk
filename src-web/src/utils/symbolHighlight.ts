/**
 * 导出符号 / JAR 条目列表的"按公共前缀分组上色"（2026-08-28 用户反馈"参考终端字体
 * 颜色，多颜色显示"）——不追求真正的符号语法高亮（C++ 修饰名解码是另一个量级的工作，
 * 也不是用户提的诉求），只是给成百上千条名字按它们共享的前缀/命名空间稳定分配一种
 * 颜色，同一个模块的符号在列表里连成一片颜色，扫描时一眼能看出"这批是同一个东西"。
 * 颜色来自 theme.css 的 --sym-1..6（Dracula 调色板，和终端配色同源）。
 */

const SYMBOL_COLOR_COUNT = 6;

/** 提取一个名字的"分组 key"：优先用第一个下划线/斜杠/点分隔符之前的部分（DLL 导出
 * 常见的 `Module_FuncName`、JAR 条目路径的 `com/foo/Bar.class` 都适用），
 * 找不到就退化成开头的驼峰前缀或前 8 个字符。 */
export function groupKeyOf(name: string): string {
  const stripped = name.replace(/^_+/, "").replace(/@\d+$/, "");
  const sepMatch = stripped.match(/^[^_./\\]+/);
  if (sepMatch && sepMatch[0].length >= 2 && sepMatch[0].length <= 24 && sepMatch[0].length < stripped.length) {
    return sepMatch[0];
  }
  const camel = stripped.match(/^[A-Z][a-z0-9]*/);
  if (camel && camel[0].length >= 3) return camel[0];
  return stripped.slice(0, Math.min(stripped.length, 8)) || name;
}

function hashString(s: string): number {
  let h = 0;
  for (let i = 0; i < s.length; i++) h = (h * 31 + s.charCodeAt(i)) | 0;
  return Math.abs(h);
}

export function symbolColorClass(name: string): string {
  return `sym-color-${hashString(groupKeyOf(name)) % SYMBOL_COLOR_COUNT}`;
}

export interface GroupStat {
  key: string;
  count: number;
}

/** 统计（用户原话"要能...统计"）：按分组 key 聚合出现次数，取数量最多的
 * `limit` 组——列表条目上千时，这几个分组往往就代表了这个库/包里最主要的几个
 * 模块，点一下对应的徽标可以直接把搜索框设成这个前缀，起到快速筛选的作用。 */
export function topGroupStats(names: string[], limit = 10): GroupStat[] {
  const counts = new Map<string, number>();
  for (const name of names) {
    const key = groupKeyOf(name);
    counts.set(key, (counts.get(key) ?? 0) + 1);
  }
  return Array.from(counts.entries())
    .map(([key, count]) => ({ key, count }))
    .sort((a, b) => b.count - a.count || a.key.localeCompare(b.key))
    .slice(0, limit);
}
