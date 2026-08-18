import React from "react";

export type ConnectionStatus = "connected" | "connecting" | "disconnected" | "error";

interface StatusDotProps {
  status: ConnectionStatus;
  title?: string;
}

/**
 * 四态状态圆点（docs/prototypes/devhub-dialogs.html §6），在 Tab 栏、Explorer 工作区名、
 * 连接树、状态栏中复用，颜色语义与 UI_DESIGN.md §2.1 一致，不在各处各画一套。
 */
export const StatusDot: React.FC<StatusDotProps> = ({ status, title }) => (
  <span className={`status-dot ${status}`} title={title} />
);
