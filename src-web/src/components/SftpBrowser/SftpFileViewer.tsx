import React, { useEffect, useState } from "react";
import Editor from "@monaco-editor/react";
import { KeyCode, KeyMod } from "monaco-editor";
import { sftpService } from "../../services/sftpService";
import { detectLanguage } from "../../utils/language";
import { formatError } from "../../utils/error";
import { useToastStore } from "../shared/Toast";
import { useThemeStore } from "../../stores/themeStore";
import { ConflictDialog } from "../Editor/ConflictDialog";

interface SftpFileViewerProps {
  profileId: string;
  path: string;
  onBack: () => void;
}

/**
 * SFTP 快捷工具打开文件的查看器（DESIGN.md §3.3.1/§六）：**默认只读**，
 * 需要用户显式点「编辑」才转可写——和 Explorer 打开即可编辑的行为刻意不同，
 * 因为这里常是"顺手看看工作区之外的系统文件"，不该一言不合就被改掉。
 */
export const SftpFileViewer: React.FC<SftpFileViewerProps> = ({ profileId, path, onBack }) => {
  const [content, setContent] = useState("");
  const [mtime, setMtime] = useState<number>(0);
  const [loading, setLoading] = useState(true);
  const [editMode, setEditMode] = useState(false);
  const [dirty, setDirty] = useState(false);
  const [conflict, setConflict] = useState<{ mtime: number; preview: string } | null>(null);
  const push = useToastStore((s) => s.push);
  const monacoTheme = useThemeStore((s) => (s.theme === "dark" ? "vs-dark" : "vs"));

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setEditMode(false);
    setDirty(false);
    sftpService.readFile(profileId, path).then(
      (res) => {
        if (cancelled) return;
        setContent(res.text);
        setMtime(res.mtime);
        setLoading(false);
      },
      (e) => {
        if (cancelled) return;
        push("error", `打开失败：${formatError(e)}`);
        setLoading(false);
      },
    );
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [profileId, path]);

  const handleSave = async () => {
    try {
      const outcome = await sftpService.writeFile(profileId, path, content, mtime);
      if (outcome.type === "Conflict") {
        setConflict({ mtime: outcome.current_mtime, preview: outcome.current_preview });
        return;
      }
      setMtime(outcome.mtime);
      setDirty(false);
      push("success", "已保存");
    } catch (e) {
      push("error", `保存失败：${formatError(e)}`, { label: "重试保存", onClick: handleSave });
    }
  };

  const fileName = path.split("/").pop() ?? path;

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      <div className="sftp-toolbar">
        <div className="breadcrumb">
          <span className="crumb" onClick={onBack}>{path.slice(0, path.length - fileName.length).replace(/\/$/, "") || "/"}</span>
          <span className="sep">›</span>
          <span className="crumb current">{fileName}</span>
        </div>
        <div style={{ marginLeft: "auto", display: "flex", gap: 4, alignItems: "center" }}>
          {dirty && <span className="dirty-dot" title="未保存" />}
          {!editMode ? (
            <button className="btn ghost sm" onClick={() => setEditMode(true)}>✎ 编辑</button>
          ) : (
            <button className="btn ghost sm" onClick={handleSave}>💾 保存</button>
          )}
          <button className="btn ghost sm" onClick={onBack}>↩ 返回</button>
        </div>
      </div>

      {loading ? (
        <div style={{ padding: 16, fontSize: 12, color: "var(--text-secondary)" }}>加载中…</div>
      ) : (
        <div style={{ flex: 1, minHeight: 0 }}>
          <Editor
            path={path}
            language={detectLanguage(path)}
            value={content}
            theme={monacoTheme}
            onChange={(value) => {
              setContent(value ?? "");
              setDirty(true);
            }}
            onMount={(editor) => {
              editor.addCommand(KeyMod.CtrlCmd | KeyCode.KeyS, () => {
                if (editMode) handleSave();
              });
            }}
            options={{
              readOnly: !editMode,
              fontSize: 13,
              fontFamily: "var(--font-mono)",
              minimap: { enabled: true },
              wordWrap: "on",
              automaticLayout: true,
              scrollBeyondLastLine: false,
            }}
          />
        </div>
      )}

      {conflict && (
        <ConflictDialog
          open
          path={path}
          onViewDiff={() => setConflict(null)}
          onSaveAsCopy={() => setConflict(null)}
          onOverwrite={async () => {
            setConflict(null);
            const outcome = await sftpService.writeFile(profileId, path, content, null);
            if (outcome.type === "Written") {
              setMtime(outcome.mtime);
              setDirty(false);
              push("success", "已保存（已覆盖）");
            }
          }}
        />
      )}
    </div>
  );
};
