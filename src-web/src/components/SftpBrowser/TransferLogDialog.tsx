import React, { useEffect, useState } from "react";
import { X, Search, Trash2, ArrowDownToLine, ArrowUpFromLine } from "lucide-react";
import { transferService } from "../../services/transferService";
import { useToastStore } from "../shared/Toast";
import { ConfirmDialog } from "../shared/ConfirmDialog";
import { formatError } from "../../utils/error";
import type { TransferLogEntry } from "../../types/bindings";

const PAGE_SIZE = 50;

const STATUS_LABEL: Record<TransferLogEntry["status"], string> = {
  completed: "完成",
  cancelled: "已取消",
  failed: "失败",
};
const STATUS_CLASS: Record<TransferLogEntry["status"], string> = {
  completed: "connected",
  cancelled: "connecting",
  failed: "error",
};

function formatTimestamp(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return `${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")} ${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}:${String(d.getSeconds()).padStart(2, "0")}`;
}

interface TransferLogDialogProps {
  onClose: () => void;
}

/**
 * 传输历史查询（用户 2026-09-01 需求："传输日志需要记录，并可在界面上查询追溯"）。
 * SFTP 和 Agent 两种双栏浏览器共用同一个弹窗和同一张后端表——传输记录本身不区分
 * "从哪个浏览器实例发起"，用户想查的是"这台机器/这个路径到底传没传过、传没传
 * 成功"，不需要按打开方式再拆一份。
 */
export const TransferLogDialog: React.FC<TransferLogDialogProps> = ({ onClose }) => {
  const [entries, setEntries] = useState<TransferLogEntry[]>([]);
  const [search, setSearch] = useState("");
  const [loading, setLoading] = useState(false);
  const [hasMore, setHasMore] = useState(true);
  const [confirmClear, setConfirmClear] = useState(false);
  const push = useToastStore((s) => s.push);

  const load = async (reset: boolean) => {
    setLoading(true);
    try {
      const offset = reset ? 0 : entries.length;
      const page = await transferService.listLog(PAGE_SIZE, offset, search.trim() || undefined);
      setEntries((prev) => (reset ? page : [...prev, ...page]));
      setHasMore(page.length === PAGE_SIZE);
    } catch (e) {
      push("error", `加载传输日志失败：${formatError(e)}`);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    void load(true);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const handleSearch = (e: React.FormEvent) => {
    e.preventDefault();
    void load(true);
  };

  const handleClear = async () => {
    setConfirmClear(false);
    try {
      await transferService.clearLog();
      setEntries([]);
      setHasMore(false);
      push("success", "已清空传输日志");
    } catch (e) {
      push("error", `清空失败：${formatError(e)}`);
    }
  };

  return (
    <div className="dialog-overlay" role="dialog" aria-modal="true" onClick={(e) => e.target === e.currentTarget && onClose()}>
      <div className="dialog" style={{ minWidth: 640, width: "80vw", maxWidth: 960, maxHeight: "80vh", display: "flex", flexDirection: "column" }}>
        <div className="dialog-title-bar info" style={{ display: "flex", alignItems: "center" }}>
          <span>📜</span>
          <span style={{ flex: 1 }}>传输日志</span>
          <button className="btn ghost sm" onClick={onClose} title="关闭">
            <X style={{ width: 14, height: 14 }} />
          </button>
        </div>
        <div className="dialog-body" style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column", overflow: "hidden" }}>
          <form onSubmit={handleSearch} style={{ display: "flex", gap: 8, marginBottom: 8, flexShrink: 0 }}>
            <div className="form-input-group" style={{ flex: 1 }}>
              <Search style={{ width: 14, height: 14, flexShrink: 0, color: "var(--text-secondary)" }} />
              <input
                className="form-input"
                placeholder="按本地/远程路径或连接名称搜索"
                value={search}
                onChange={(e) => setSearch(e.target.value)}
              />
            </div>
            <button className="btn ghost sm" type="submit" disabled={loading}>
              搜索
            </button>
            <button className="btn ghost sm" type="button" onClick={() => setConfirmClear(true)}>
              <Trash2 style={{ width: 12, height: 12 }} /> 清空
            </button>
          </form>

          <div style={{ flex: 1, minHeight: 0, overflow: "auto", border: "1px solid var(--border-default)", borderRadius: "var(--radius-sm)" }}>
            <table style={{ width: "100%", borderCollapse: "collapse", fontSize: 12 }}>
              <thead>
                <tr style={{ position: "sticky", top: 0, background: "var(--bg-surface)", textAlign: "left" }}>
                  <th style={{ padding: "6px 8px" }}>时间</th>
                  <th style={{ padding: "6px 8px" }}>方向</th>
                  <th style={{ padding: "6px 8px" }}>连接</th>
                  <th style={{ padding: "6px 8px" }}>本地路径</th>
                  <th style={{ padding: "6px 8px" }}>远程路径</th>
                  <th style={{ padding: "6px 8px" }}>文件数</th>
                  <th style={{ padding: "6px 8px" }}>状态</th>
                </tr>
              </thead>
              <tbody>
                {entries.length === 0 && !loading ? (
                  <tr>
                    <td colSpan={7} style={{ padding: 16, textAlign: "center", color: "var(--text-secondary)" }}>
                      没有传输记录
                    </td>
                  </tr>
                ) : (
                  entries.map((entry) => (
                    <tr key={entry.id} style={{ borderTop: "1px solid var(--border-subtle)" }} title={entry.error_message ?? undefined}>
                      <td style={{ padding: "6px 8px", whiteSpace: "nowrap", color: "var(--text-secondary)" }}>{formatTimestamp(entry.finished_at)}</td>
                      <td style={{ padding: "6px 8px" }} title={entry.direction === "upload" ? "上传" : "下载"}>
                        {entry.direction === "upload" ? (
                          <ArrowUpFromLine style={{ width: 12, height: 12, display: "inline", verticalAlign: -1 }} />
                        ) : (
                          <ArrowDownToLine style={{ width: 12, height: 12, display: "inline", verticalAlign: -1 }} />
                        )}{" "}
                        {entry.protocol === "sftp" ? "SFTP" : "Agent"}
                      </td>
                      <td style={{ padding: "6px 8px", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", maxWidth: 120 }}>
                        {entry.profile_name}
                      </td>
                      <td style={{ padding: "6px 8px", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", maxWidth: 200, fontFamily: "var(--font-mono)" }}>
                        {entry.local_path}
                      </td>
                      <td style={{ padding: "6px 8px", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", maxWidth: 200, fontFamily: "var(--font-mono)" }}>
                        {entry.remote_path}
                      </td>
                      <td style={{ padding: "6px 8px" }}>{entry.file_count}</td>
                      <td style={{ padding: "6px 8px" }}>
                        <span className={`status-dot ${STATUS_CLASS[entry.status]}`} style={{ marginRight: 4 }} />
                        {STATUS_LABEL[entry.status]}
                      </td>
                    </tr>
                  ))
                )}
              </tbody>
            </table>
          </div>

          {hasMore && (
            <button className="btn ghost sm" style={{ marginTop: 8, alignSelf: "center" }} disabled={loading} onClick={() => void load(false)}>
              {loading ? "加载中…" : "加载更多"}
            </button>
          )}
        </div>
      </div>

      {confirmClear && (
        <ConfirmDialog
          open
          severity="danger"
          icon="🗑"
          title="清空传输日志"
          onDismiss={() => setConfirmClear(false)}
          actions={
            <>
              <button className="btn ghost sm" onClick={() => setConfirmClear(false)}>
                取消
              </button>
              <button className="btn danger-strong sm" onClick={handleClear}>
                清空
              </button>
            </>
          }
        >
          <p>确定要清空全部传输日志吗？此操作不可撤销。</p>
        </ConfirmDialog>
      )}
    </div>
  );
};
