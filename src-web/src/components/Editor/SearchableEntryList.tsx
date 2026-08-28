import React from "react";
import { groupKeyOf, symbolColorClass, topGroupStats } from "../../utils/symbolHighlight";

interface SearchableEntryListProps {
  /** 完整条目列表（导出符号名 / JAR 内部路径），不做截断——用户要的就是"能看到全部"。 */
  items: string[];
  /** 后端安全上限截断时的提示（正常软件基本不会触发，见 fsops 里 `*_HARD_CAP` 的注释）。 */
  hardCapped: boolean;
  placeholder: string;
  emptyText: string;
}

/**
 * 导出符号（BinaryInfoPanel）/ JAR 条目（JarInfoPanel）共用的搜索 + 按前缀分组
 * 上色 + 统计列表（2026-08-28 用户反馈："要能查看所有的导出符号，这对开发人员
 * 很重要"，并要求"多颜色显示（参考终端字体颜色）...能搜索、统计"）。
 *
 * 不做虚拟滚动——几千条 `<div>` 在现代浏览器里渲染/搜索仍然流畅，真正的大体量
 * （几万条）已经被后端的硬上限挡住，没必要为了这个引入虚拟列表依赖。
 */
export const SearchableEntryList: React.FC<SearchableEntryListProps> = ({ items, hardCapped, placeholder, emptyText }) => {
  const [query, setQuery] = React.useState("");

  const groupStats = React.useMemo(() => topGroupStats(items, 10), [items]);

  const filtered = React.useMemo(() => {
    if (!query.trim()) return items;
    const q = query.toLowerCase();
    return items.filter((name) => name.toLowerCase().includes(q));
  }, [items, query]);

  if (items.length === 0) {
    return <div className="sym-list-empty">{emptyText}</div>;
  }

  return (
    <div>
      <div style={{ display: "flex", alignItems: "center", gap: "var(--space-2)", marginBottom: 4 }}>
        <input
          className="sym-list-search"
          type="text"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder={placeholder}
        />
        <span className="sym-list-count">
          {query.trim() ? `匹配 ${filtered.length} / 共 ${items.length}` : `共 ${items.length}`}
          {hardCapped ? "（已按上限截断）" : ""}
        </span>
      </div>

      {groupStats.length > 1 && (
        <div className="sym-list-groups">
          {groupStats.map((g) => (
            <span
              key={g.key}
              className={`sym-list-group-badge ${query === g.key ? "active" : ""}`}
              title={`按前缀 "${g.key}" 筛选（点击已选中的再点一次取消）`}
              onClick={() => setQuery((prev) => (prev === g.key ? "" : g.key))}
            >
              <span className={symbolColorClass(g.key + "_")}>{g.key}</span> × {g.count}
            </span>
          ))}
        </div>
      )}

      <div className="sym-list-rows">
        {filtered.length === 0 ? (
          <div className="sym-list-empty">没有匹配 "{query}" 的条目</div>
        ) : (
          filtered.map((name, i) => (
            <div key={name + i} className="sym-list-row">
              <span className={symbolColorClass(name)}>{name}</span>
            </div>
          ))
        )}
      </div>
    </div>
  );
};

// groupKeyOf 目前只在 symbolHighlight.ts 内部使用，这里 re-export 是为了让
// BinaryInfoPanel/JarInfoPanel 不需要额外 import 就能在展示"依赖库"这类不经过
// 这个列表组件的地方也复用同一套分组着色逻辑（比如按依赖库名分组上色）。
export { groupKeyOf, symbolColorClass };
