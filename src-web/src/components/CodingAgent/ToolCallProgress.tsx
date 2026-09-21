import React from "react";
import { Check, ChevronDown, ChevronRight } from "lucide-react";

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
  /** 这次调用实际拿到的结果文本（`coding:tool-call-end` 带回来的）——2026-09
   * 用户需求："想点一下时间线里已完成的命令，看看它到底执行出了什么"。只有
   * `done` 且有内容时才会出现可展开的入口，运行中/没有输出内容的调用不受影响。 */
  output?: string | null;
  expanded?: boolean;
  onToggleOutput?: () => void;
}

export function toolLabel(tool: string): string {
  const labels: Record<string, string> = {
    read_file: "读取文件", write_file: "创建文件变更", edit_file: "编辑文件",
    list_directory: "查看目录", search_files: "搜索代码", run_command: "执行命令",
    glob: "按文件名查找", webfetch: "抓取网页", todo_write: "更新任务清单",
    question: "向你提问", skill: "加载技能",
    multi_edit: "批量编辑文件", git_status: "查看 Git 状态", git_diff: "查看 Git 改动",
    git_commit: "Git 提交", run_command_background: "后台启动命令",
    read_background_output: "查看后台输出", stop_background_process: "结束后台进程",
    find_definition: "查找定义", task: "委派子任务",
    // sql::agent 的工具集（和 coding::tools 共用同一个标签映射/同一个组件，
    // 见 SqlAgentPanel.tsx）。
    run_query: "执行 SQL", describe_table: "查看表结构", list_objects: "列出表",
  };
  if (tool.startsWith("mcp__")) {
    const [, server, name] = tool.split("__");
    return `调用 MCP 工具 ${server ?? ""}.${name ?? tool}`;
  }
  return labels[tool] ?? `执行 ${tool}`;
}

/**
 * 工具调用的耗时反馈（DESIGN.md §3.8.7 性能诚实）。远程目标几乎每次调用都会出现
 * （SSH 往返有延迟）；本地目标如果单次调用 < 300ms，由调用方直接不渲染这个组件，
 * 阈值属于业务决策，组件本身只负责展示。
 */
export const ToolCallProgress: React.FC<ToolCallProgressProps> = ({
  tool,
  elapsedMs,
  done,
  detail,
  onOpenFile,
  output,
  expanded,
  onToggleOutput,
}) => {
  const isCommand = tool === "run_command" || tool === "run_query";
  const canExpand = Boolean(done && output && onToggleOutput);
  // Keep very large command/search results from monopolizing the browser main
  // thread. The complete result remains in the timeline state/backend; the
  // expanded preview is intentionally bounded for smooth scrolling.
  const visibleOutput = output && output.length > 12000
    ? `${output.slice(0, 12000)}\n…（结果过长，已折叠 ${output.length - 12000} 个字符）`
    : output;
  return (
    <div>
      <div
        className="tool-call-progress"
        style={canExpand ? { cursor: "pointer" } : undefined}
        onClick={canExpand ? onToggleOutput : undefined}
        title={canExpand ? (expanded ? "点击收起执行结果" : "点击查看执行结果") : undefined}
      >
        {done ? <Check style={{ width: 10, height: 10, color: "var(--text-secondary)" }} /> : <span className="spinner" />}
        {done ? `已完成 ${toolLabel(tool)}` : `正在${toolLabel(tool)} · ${(elapsedMs / 1000).toFixed(1)}s`}
        {detail && (
          onOpenFile ? (
            <button className="tool-file-ref" onClick={(e) => { e.stopPropagation(); onOpenFile(); }}>· {detail}</button>
          ) : isCommand ? (
            <code className="tool-detail-cmd">{detail}</code>
          ) : (
            <span style={{ opacity: 0.7 }}>· {detail}</span>
          )
        )}
        {canExpand && (
          expanded
            ? <ChevronDown style={{ width: 12, height: 12, color: "var(--text-secondary)", marginLeft: "auto", flexShrink: 0 }} />
            : <ChevronRight style={{ width: 12, height: 12, color: "var(--text-secondary)", marginLeft: "auto", flexShrink: 0 }} />
        )}
      </div>
      {expanded && visibleOutput && (
        <pre className="tool-output-pane">{visibleOutput}</pre>
      )}
    </div>
  );
};
