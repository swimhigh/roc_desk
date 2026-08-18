import React from "react";

interface TargetBadgeProps {
  targetLabel: string; // 例如 "web-01(远程)" 或 "本地"
  isRemote: boolean;
  onChangeTarget?: () => void;
}

/**
 * 自动绑定的目标提示 + "更改"覆盖入口（DESIGN.md §3.8.1）。
 * 默认不是下拉选择器：目标继承自当前工作区，"更改"是刻意做的不显眼例外入口，
 * 防止被当成日常要选的选项（DESIGN.md §3.8.2.1 强调的"防止误以为在改本地文件"落地）。
 */
export const TargetBadge: React.FC<TargetBadgeProps> = ({ targetLabel, isRemote, onChangeTarget }) => (
  <span style={{ display: "inline-flex", alignItems: "center", gap: 8 }}>
    <span className={`target-badge ${isRemote ? "remote" : "local"}`}>
      {isRemote ? "🖥" : "💻"} 目标: {targetLabel}
    </span>
    <button className="target-change-link" onClick={onChangeTarget}>更改▾</button>
  </span>
);
