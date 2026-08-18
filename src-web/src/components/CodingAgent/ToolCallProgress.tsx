import React from "react";

interface ToolCallProgressProps {
  tool: string;
  elapsedMs: number;
}

/**
 * 工具调用中的耗时反馈（DESIGN.md §3.8.7 性能诚实）。远程目标几乎每次调用都会出现
 * （SSH 往返有延迟）；本地目标如果单次调用 < 300ms，由调用方直接不渲染这个组件，
 * 阈值属于业务决策，组件本身只负责展示。
 */
export const ToolCallProgress: React.FC<ToolCallProgressProps> = ({ tool, elapsedMs }) => (
  <div className="tool-call-progress">
    <span className="spinner" />
    正在执行 {tool} · {(elapsedMs / 1000).toFixed(1)}s
  </div>
);
