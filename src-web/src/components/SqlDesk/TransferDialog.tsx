import React, { useEffect, useRef, useState } from "react";
import { open as openFileDialog, save as saveFileDialog } from "@tauri-apps/plugin-dialog";
import { X } from "lucide-react";
import { sqlService } from "../../services/sqlService";
import { useToastStore } from "../shared/Toast";
import { formatError } from "../../utils/error";
import type { ObjectRef, TransferFormat } from "../../types/bindings";

interface TransferDialogProps {
  dataSourceId: string;
  mode: "export" | "import";
  object: ObjectRef;
  onClose: () => void;
}

/** 导出/导入弹窗（docs/SQL_DESKTOP_PLAN.md，2026-09 用户反馈：右键表可以
 * 导出导入数据，导入前提示先备份，导出要支持大数据量断点续传）。真正的
 * 分批查询/写盘/断点续传逻辑在后端 `sql::transfer`，这里只是发起任务 +
 * 轮询进度，和查询执行的轮询是同一种模式（`sqlStore.pollUntilDone`）。 */
export const TransferDialog: React.FC<TransferDialogProps> = ({ dataSourceId, mode, object, onClose }) => {
  const push = useToastStore((s) => s.push);
  const [format, setFormat] = useState<TransferFormat>("csv");
  const [filePath, setFilePath] = useState("");
  const [hasHeader, setHasHeader] = useState(true);
  const [backupConfirmed, setBackupConfirmed] = useState(false);
  const [jobId, setJobId] = useState<string | null>(null);
  const [progress, setProgress] = useState<{ rowsDone: number; done: boolean; cancelled: boolean; error: string | null } | null>(null);
  const [resumeAvailable, setResumeAvailable] = useState(false);
  const pollTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);

  const running = jobId != null && progress != null && !progress.done && !progress.cancelled && !progress.error;

  const pickPath = async () => {
    try {
      if (mode === "export") {
        const path = await saveFileDialog({
          defaultPath: `${object.name}.${format}`,
          filters: [{ name: format.toUpperCase(), extensions: [format] }],
        });
        if (path) {
          setFilePath(path);
          setResumeAvailable(false);
        }
      } else {
        const path = await openFileDialog({ multiple: false, filters: [{ name: "CSV", extensions: ["csv"] }] });
        if (typeof path === "string") setFilePath(path);
      }
    } catch (e) {
      push("error", `选择文件失败：${formatError(e)}`);
    }
  };

  const poll = (id: string) => {
    const fn = mode === "export" ? sqlService.exportPoll : sqlService.importPoll;
    fn(id)
      .then((p) => {
        setProgress({ rowsDone: p.rows_done, done: p.done, cancelled: p.cancelled, error: p.error });
        if (!p.done && !p.cancelled && !p.error) {
          pollTimer.current = setTimeout(() => poll(id), 500);
        } else if (p.error) {
          push("error", `${mode === "export" ? "导出" : "导入"}失败：${p.error}`);
        } else if (p.done) {
          push("success", `${mode === "export" ? "导出" : "导入"}完成，共 ${p.rows_done} 行`);
        }
      })
      .catch((e) => push("error", `查询进度失败：${formatError(e)}`));
  };

  useEffect(() => () => clearTimeout(pollTimer.current), []);

  const start = async (resume: boolean) => {
    if (!filePath) return;
    try {
      const id =
        mode === "export"
          ? await sqlService.exportStart(dataSourceId, object, format, filePath, resume)
          : await sqlService.importStart(dataSourceId, object, filePath, hasHeader, resume);
      setJobId(id);
      setProgress({ rowsDone: 0, done: false, cancelled: false, error: null });
      poll(id);
    } catch (e) {
      push("error", `启动失败：${formatError(e)}`);
    }
  };

  const cancel = async () => {
    if (!jobId) return;
    clearTimeout(pollTimer.current);
    try {
      await (mode === "export" ? sqlService.exportCancel(jobId) : sqlService.importCancel(jobId));
      setResumeAvailable(true);
      push("success", "已请求取消，已写入的部分不会丢失，可以稍后继续");
    } catch (e) {
      push("error", `取消失败：${formatError(e)}`);
    }
  };

  return (
    <div className="dialog-overlay" onClick={running ? undefined : onClose}>
      <div className="dialog" style={{ width: 480 }} onClick={(e) => e.stopPropagation()}>
        <div className="dialog-title-bar">
          <span>{mode === "export" ? "导出数据" : "导入数据"}：{object.schema}.{object.name}</span>
          {!running && <button className="icon-btn" onClick={onClose}><X size={16} /></button>}
        </div>
        <div className="dialog-body">
          <div className="form">
            {mode === "import" && (
              <div
                style={{
                  padding: 10,
                  borderRadius: 6,
                  background: "color-mix(in srgb, var(--danger) 10%, transparent)",
                  color: "var(--danger)",
                  fontSize: 12,
                }}
              >
                导入会直接向表里写入数据，可能产生重复或覆盖，
                <b>强烈建议先备份该表或整个数据库</b>再继续。
                <div style={{ marginTop: 6, display: "flex", alignItems: "center", gap: 6 }}>
                  <input type="checkbox" checked={backupConfirmed} onChange={(e) => setBackupConfirmed(e.target.checked)} />
                  <label style={{ margin: 0, color: "var(--text-primary)" }}>我已经备份，确认继续导入</label>
                </div>
              </div>
            )}

            {mode === "export" && (
              <div className="form-row">
                <label className="form-label">格式</label>
                <select className="form-select" value={format} onChange={(e) => { setFormat(e.target.value as TransferFormat); setFilePath(""); }} disabled={running}>
                  <option value="csv">CSV（支持断点续传）</option>
                  <option value="json">JSON</option>
                </select>
              </div>
            )}

            <div className="form-row">
              <label className="form-label">{mode === "export" ? "保存到" : "选择 CSV 文件"}</label>
              <div className="form-input-group">
                <input className="form-input" readOnly value={filePath} placeholder="点击右侧按钮选择" />
                <button className="btn ghost sm" onClick={() => void pickPath()} disabled={running}>选择…</button>
              </div>
            </div>

            {mode === "import" && (
              <div className="form-row" style={{ flexDirection: "row", alignItems: "center", gap: 6 }}>
                <input type="checkbox" checked={hasHeader} onChange={(e) => setHasHeader(e.target.checked)} disabled={running} />
                <label className="form-label" style={{ margin: 0 }}>CSV 第一行是表头（列名需要和表字段一致）</label>
              </div>
            )}
            {mode === "import" && !hasHeader && (
              <p style={{ fontSize: 11, color: "var(--text-secondary)" }}>
                没有表头时，CSV 各列顺序必须和表结构里的字段顺序完全一致。
              </p>
            )}

            {progress && (
              <div style={{ fontSize: 13 }}>
                {progress.error ? (
                  <span style={{ color: "var(--danger)" }}>失败：{progress.error}</span>
                ) : progress.cancelled ? (
                  <span>已取消，已处理 {progress.rowsDone} 行（可以点"继续上次进度"接着做）</span>
                ) : progress.done ? (
                  <span style={{ color: "var(--success)" }}>完成，共处理 {progress.rowsDone} 行</span>
                ) : (
                  <span>处理中…已完成 {progress.rowsDone} 行</span>
                )}
              </div>
            )}
          </div>
        </div>
        <div className="dialog-actions">
          {running ? (
            <button className="btn danger sm" onClick={() => void cancel()}>取消</button>
          ) : (
            <>
              <button className="btn ghost sm" onClick={onClose}>关闭</button>
              {resumeAvailable && (
                <button className="btn ghost sm" onClick={() => void start(true)} disabled={!filePath}>继续上次进度</button>
              )}
              <button
                className="btn primary sm"
                onClick={() => void start(false)}
                disabled={!filePath || (mode === "import" && !backupConfirmed)}
              >
                {mode === "export" ? "开始导出" : "开始导入"}
              </button>
            </>
          )}
        </div>
      </div>
    </div>
  );
};
