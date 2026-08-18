import React from "react";

export type ChangeStatus = "pending" | "applied" | "rejected";

export interface DiffLine {
  sign: "+" | "-" | " ";
  content: string;
}

interface FileChangeCardProps {
  path: string;
  status: ChangeStatus;
  diff: DiffLine[];
  onViewDiff?: () => void;
  onAccept?: () => void;
  onReject?: () => void;
  onUndo?: () => void;
}

const BADGE_LABEL: Record<ChangeStatus, string> = {
  pending: "⏳ 待确认",
  applied: "✓ 已应用",
  rejected: "✗ 已拒绝",
};

/** AI 修改的文件变更卡片，嵌入对话流中（DESIGN.md §3.8.3）。三态配色对应 §2.1 语义色。*/
export const FileChangeCard: React.FC<FileChangeCardProps> = ({
  path,
  status,
  diff,
  onViewDiff,
  onAccept,
  onReject,
  onUndo,
}) => (
  <div className={`fcc ${status}`}>
    <div className={`fcc-header ${status}`}>
      <span>📄</span>
      <span className="fcc-path">{path}</span>
      <span className={`fcc-badge ${status}`}>{BADGE_LABEL[status]}</span>
      <div className="fcc-actions">
        <button className="fcc-btn outline" onClick={onViewDiff}>查看 Diff</button>
        {status === "pending" && (
          <>
            <button className="fcc-btn success" onClick={onAccept}>✓ 应用</button>
            <button className="fcc-btn danger" onClick={onReject}>✗ 拒绝</button>
          </>
        )}
        {status === "applied" && (
          <button className="fcc-btn outline" onClick={onUndo}>↩ 撤销</button>
        )}
      </div>
    </div>
    {diff.length > 0 && (
      <div className="fcc-diff">
        {diff.map((line, i) => (
          <div
            key={i}
            className={`diff-line ${line.sign === "+" ? "added" : line.sign === "-" ? "removed" : ""}`}
          >
            <span className="diff-sign">{line.sign}</span>
            <span className="diff-content">{line.content}</span>
          </div>
        ))}
      </div>
    )}
  </div>
);
