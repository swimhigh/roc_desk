import React, { useState } from "react";
import { Loader2, Pencil, Trash2, X } from "lucide-react";

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
  onOpen: (id: string) => void | Promise<unknown>;
  onDelete: (id: string) => void;
  onRename: (id: string, title: string) => void;
  onClose: () => void;
  /** 正在打开中的历史记录 id——2026-10 起"点开一条历史"每次都要去工作区目录
   * 现读内容（不再有本地全量缓存），远程工作区可能有明显延迟，这里用它在
   * 对应行上显示"正在打开"，而不是让弹窗看起来卡住没反应。只有编程助手历史
   * 传这个（SQL 助手历史量通常小得多，还没有这个延迟问题）。 */
  openingId?: string | null;
}

/** 会话历史列表弹窗——`coding`（编程助手）和 `sql::agent`（SQL 助手）共用同一个
 * 组件，只是标题/空状态文案和摘要行里要不要展示"模式"不一样（见
 * `SqlAgentPanel.tsx` 的调用点）。 */
export const CodingHistoryDialog: React.FC<Props> = ({ title, histories, emptyText, onOpen, onDelete, onRename, onClose, openingId }) => {
  const [editingId, setEditingId] = useState<string | null>(null);
  const [renameText, setRenameText] = useState("");
  const submit = (id: string) => {
    if (renameText.trim()) onRename(id, renameText.trim());
    setEditingId(null);
  };
  const opening = Boolean(openingId);
  return <div className="coding-history-overlay" onClick={onClose}>
    <div className="coding-history-dialog" onClick={(event) => event.stopPropagation()}>
      <div className="coding-history-title">
        <span>{title}</span>
        <button className="btn ghost sm" onClick={onClose}><X /></button>
      </div>
      {histories.length === 0 ? <div className="coding-history-empty">{emptyText}</div> : <div className="coding-history-list">
        {histories.map((item) => <div className="coding-history-row" key={item.id}>
          {editingId === item.id ? <input className="form-input coding-history-rename" autoFocus value={renameText} onChange={(event) => setRenameText(event.target.value)} onBlur={() => submit(item.id)} onKeyDown={(event) => { if (event.key === "Enter") submit(item.id); if (event.key === "Escape") setEditingId(null); }} /> : <button className="coding-history-open" disabled={opening} onClick={() => onOpen(item.id)}><strong>{item.title}</strong><span>{item.provider_label} · {item.model || "未知模型"}{item.mode ? ` · ${item.mode}` : ""}</span>{openingId === item.id ? <span className="coding-history-opening"><Loader2 className="spin" /> 正在打开…</span> : <time>{new Date(item.updated_at).toLocaleString()}</time>}</button>}
          <button className="btn ghost sm" title="重命名" disabled={opening} onMouseDown={(event) => event.preventDefault()} onClick={() => { setEditingId(item.id); setRenameText(item.title); }}><Pencil /></button>
          <button className="btn ghost sm" title="删除历史" disabled={opening} onClick={() => onDelete(item.id)}><Trash2 /></button>
        </div>)}
      </div>}
    </div>
  </div>;
};
