import React from "react";
import { Check } from "lucide-react";

interface ToolCallProgressProps {
  tool: string;
  elapsedMs: number;
  /** 调用已经结束（2026-08-18 之前完成的调用直接从时间线里消失了——用户反馈"编程
   * 助手的思考过程没有展示出来"，根因之一就是这里：调用方原来对完成的条目返回
   * `null`。这里改成保留一条不带动画的完成态，而不是让整段执行过程消失不见）。 */
  done?: boolean;
  /** 这次调用在操作什么（文件路径/搜索词等）。2026-08-18 用户反馈"好像确实是循环
   * 了"——只有工具名完全看不出是不是在反复处理同一个东西，加上这个字段之后
   * 一眼就能确认。 */
  detail?: string | null;
  onOpenFile?: () => void;
}

function toolLabel(tool: string): string {
  const labels: Record<string, string> = {
    read_file: "读取文件", write_file: "创建文件变更", edit_file: "编辑文件",
    list_directory: "查看目录", search_files: "搜索代码", run_command: "执行命令",
  };
  return labels[tool] ?? `执行 ${tool}`;
}

/**
 * 工具调用的耗时反馈（DESIGN.md §3.8.7 性能诚实）。远程目标几乎每次调用都会出现
 * （SSH 往返有延迟）；本地目标如果单次调用 < 300ms，由调用方直接不渲染这个组件，
 * 阈值属于业务决策，组件本身只负责展示。
 */
export const ToolCallProgress: React.FC<ToolCallProgressProps> = ({ tool, elapsedMs, done, detail, onOpenFile }) => (
  <div className="tool-call-progress">
    {done ? <Check style={{ width: 10, height: 10, color: "var(--text-secondary)" }} /> : <span className="spinner" />}
    {done ? `已完成 ${toolLabel(tool)}` : `正在${toolLabel(tool)} · ${(elapsedMs / 1000).toFixed(1)}s`}
    {detail && (onOpenFile ? <button className="tool-file-ref" onClick={onOpenFile}>· {detail}</button> : <span style={{ opacity: 0.7 }}>· {detail}</span>)}
  </div>
);
