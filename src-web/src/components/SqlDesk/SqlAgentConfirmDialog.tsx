import React from "react";
import { ConfirmDialog } from "../shared/ConfirmDialog";

interface SqlAgentConfirmDialogProps {
  open: boolean;
  sql: string;
  onReject: () => void;
  onAllow: () => void;
}

/** SQL Agent 的 `run_query` 工具遇到 UPDATE/DELETE/DDL 类语句时的二次确认——
 * 和 `CommandConfirmDialog` 是同一种"阻塞等待前端响应"模式，但故意没有做成
 * 那边的"允许并记住"（落地成一条按通配符匹配的权限规则）：SQL 语句不像 Shell
 * 命令那样有"首个词 + `*`"这种自然的模式泛化方式，勉强套用容易让用户以为
 * 记住的是"这一类查询"，实际上大概率只精确匹配这一条语句，没有实用价值。 */
export const SqlAgentConfirmDialog: React.FC<SqlAgentConfirmDialogProps> = ({ open, sql, onReject, onAllow }) => (
  <ConfirmDialog
    open={open}
    severity="warning"
    icon="⚠"
    title="确认执行这条 SQL"
    dismissible
    onDismiss={onReject}
    actions={
      <>
        <button className="btn ghost sm" onClick={onReject}>拒绝</button>
        <button className="btn primary sm" onClick={onAllow}>确认执行</button>
      </>
    }
  >
    <div style={{ fontSize: 12, color: "var(--text-secondary)", marginBottom: 4 }}>AI 打算执行：</div>
    <pre className="cmd-confirm-code" style={{ whiteSpace: "pre-wrap", wordBreak: "break-all" }}>{sql}</pre>
    <p style={{ fontSize: 12, color: "var(--text-secondary)" }}>
      这条语句会修改数据/结构，请确认你了解其影响；拒绝后 AI 不会重复尝试。
    </p>
  </ConfirmDialog>
);
