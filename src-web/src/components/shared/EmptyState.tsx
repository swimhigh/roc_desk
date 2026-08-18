import React from "react";

interface EmptyStateProps {
  icon: React.ReactNode;
  text: string;
  actionLabel?: string;
  onAction?: () => void;
}

/** 空列表用图标+一句话说明+主要操作，不用空表格干等（UI_DESIGN.md §十一）。*/
export const EmptyState: React.FC<EmptyStateProps> = ({ icon, text, actionLabel, onAction }) => (
  <div className="empty-state">
    <div className="es-icon">{icon}</div>
    <div className="es-text">{text}</div>
    {actionLabel && (
      <button className="es-btn" onClick={onAction}>
        {actionLabel}
      </button>
    )}
  </div>
);
