import React, { useState } from "react";
import { Pencil, Trash2, X } from "lucide-react";

/** `CodingHistorySummary`/`SqlAgentHistorySummary` 的公共形状——`mode` 是可选的
 * 因为 SQL Agent 没有 Plan/Build 模式概念（见 `sql::agent::session` 文档）。 */
interface HistorySummaryLike {
  id: string;
  title: string;
  provider_label: string;
  model: string;
  mode?: string;
  updated_at: string;
}

interface Props {
  title: string;
  histories: HistorySummaryLike[];
  emptyText: string;
  onOpen: (id: string) => void;
  onDelete: (id: string) => void;
  onRename: (id: string, title: string) => void;
  onClose: () => void;
}

/** 会话历史列表弹窗——`coding`（编程助手）和 `sql::agent`（SQL 助手）共用同一个
 * 组件，只是标题/空状态文案和摘要行里要不要展示"模式"不一样（见
 * `SqlAgentPanel.tsx` 的调用点）。 */
export const CodingHistoryDialog: React.FC<Props> = ({ title, histories, emptyText, onOpen, onDelete, onRename, onClose }) => {
  const [editingId, setEditingId] = useState<string | null>(null);
  const [renameText, setRenameText] = useState("");
  const submit = (id: string) => {
    if (renameText.trim()) onRename(id, renameText.trim());
    setEditingId(null);
  };
  return <div className="coding-history-overlay" onClick={onClose}>
    <div className="coding-history-dialog" onClick={(event) => event.stopPropagation()}>
      <div className="coding-history-title"><span>{title}</span><button className="btn ghost sm" onClick={onClose}><X /></button></div>
      {histories.length === 0 ? <div className="coding-history-empty">{emptyText}</div> : <div className="coding-history-list">
        {histories.map((item) => <div className="coding-history-row" key={item.id}>
          {editingId === item.id ? <input className="form-input coding-history-rename" autoFocus value={renameText} onChange={(event) => setRenameText(event.target.value)} onBlur={() => submit(item.id)} onKeyDown={(event) => { if (event.key === "Enter") submit(item.id); if (event.key === "Escape") setEditingId(null); }} /> : <button className="coding-history-open" onClick={() => onOpen(item.id)}><strong>{item.title}</strong><span>{item.provider_label} · {item.model || "未知模型"}{item.mode ? ` · ${item.mode}` : ""}</span><time>{new Date(item.updated_at).toLocaleString()}</time></button>}
          <button className="btn ghost sm" title="重命名" onMouseDown={(event) => event.preventDefault()} onClick={() => { setEditingId(item.id); setRenameText(item.title); }}><Pencil /></button>
          <button className="btn ghost sm" title="删除历史" onClick={() => onDelete(item.id)}><Trash2 /></button>
        </div>)}
      </div>}
    </div>
  </div>;
};
