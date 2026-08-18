import React from "react";

interface SkeletonTextProps {
  lines?: number;
}

/** 与最终内容同形状的骨架屏，不用通用 Spinner（UI_DESIGN.md §十一）。*/
export const SkeletonText: React.FC<SkeletonTextProps> = ({ lines = 3 }) => {
  const widths: Array<"long" | "medium" | "short"> = ["long", "medium", "short"];
  return (
    <div>
      {Array.from({ length: lines }).map((_, i) => (
        <div key={i} className={`skeleton-line ${widths[i % widths.length]}`} />
      ))}
    </div>
  );
};
