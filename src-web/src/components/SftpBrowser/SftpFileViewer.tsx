import React, { useEffect, useState } from "react";
import Editor from "@monaco-editor/react";
import { KeyCode, KeyMod } from "monaco-editor";
import * as XLSX from "xlsx";
import mammoth from "mammoth";
import DOMPurify from "dompurify";
import { ExternalLink } from "lucide-react";
import { sftpService } from "../../services/sftpService";
import { detectLanguage } from "../../utils/language";
import { formatError } from "../../utils/error";
import { useToastStore } from "../shared/Toast";
import { useThemeStore } from "../../stores/themeStore";
import { ConflictDialog } from "../Editor/ConflictDialog";
import { ExcelPreview } from "../Editor/ExcelPreview";
import { PdfPreview } from "../Editor/PdfPreview";
import { UnsupportedBinaryPanel } from "../Editor/UnsupportedBinaryPanel";
import { BinaryInfoPanel } from "../Editor/BinaryInfoPanel";
import { JarInfoPanel } from "../Editor/JarInfoPanel";
import { LegacyOfficePreview } from "../Editor/LegacyOfficePreview";
import { formatBytes } from "../../utils/format";
import {
  classifyPreview,
  hasNoExtension,
  binaryMimeType,
  base64ToArrayBuffer,
  mammothImageConverter,
  type ExcelSheet,
  type PreviewKind,
} from "../../utils/previewFile";
import type { BinaryInfo, JarInfo } from "../../types/bindings";

interface SftpFileViewerProps {
  profileId: string;
  path: string;
  onBack: () => void;
}

/**
 * SFTP 快捷工具打开文件的查看器（DESIGN.md §3.3.1/§六）：**默认只读**，
 * 需要用户显式点「编辑」才转可写——和 Explorer 打开即可编辑的行为刻意不同，
 * 因为这里常是"顺手看看工作区之外的系统文件"，不该一言不合就被改掉。
 *
 * 图片/PDF/Word/Excel/其它二进制走只读预览（或"用系统程序打开"），不进 Monaco——
 * 2026-08-28 用户反馈这些文件之前被当文本打开，显示一堆乱码。分流逻辑和
 * CodeEditor.tsx 共用 `utils/previewFile.ts` 的 `classifyPreview`。
 */
export const SftpFileViewer: React.FC<SftpFileViewerProps> = ({ profileId, path, onBack }) => {
  const [content, setContent] = useState("");
  const [sheets, setSheets] = useState<ExcelSheet[]>([]);
  const [binaryInfo, setBinaryInfo] = useState<BinaryInfo | null>(null);
  const [binaryInfoError, setBinaryInfoError] = useState<string | null>(null);
  const [jarInfo, setJarInfo] = useState<JarInfo | null>(null);
  const [jarInfoError, setJarInfoError] = useState<string | null>(null);
  const [legacyOfficeError, setLegacyOfficeError] = useState<string | null>(null);
  // 初值直接按扩展名分类；没有扩展名的文件（典型是 Linux 下的 ELF 可执行文件）
  // 还需要靠下面 effect 里的文件头嗅探确认，确认后可能会从 "text" 改写成
  // "executable"（2026-08-28 用户反馈）——所以这里是 state 而不是每次渲染重算的
  // 派生值，允许被异步嗅探结果覆盖。
  const [kind, setKind] = useState<PreviewKind>(() => classifyPreview(path));
  const [mtime, setMtime] = useState<number>(0);
  const [loading, setLoading] = useState(true);
  const [editMode, setEditMode] = useState(false);
  // >EDITOR_PREVIEW_THRESHOLD_BYTES 的文件后端只回了截断预览，不能当成完整内容编辑保存
  // （2026-08-28 用户反馈 >1GB 文件打开卡死后加的保护，见 fsops::read_bytes_for_editor）。
  const [truncated, setTruncated] = useState(false);
  const [totalSize, setTotalSize] = useState(0);
  const [dirty, setDirty] = useState(false);
  const [conflict, setConflict] = useState<{ mtime: number; preview: string } | null>(null);
  const push = useToastStore((s) => s.push);
  // roc-dark/roc-light：monacoSetup.ts 里注册的自定义主题，带 .log 文件的 token 配色。
  const monacoTheme = useThemeStore((s) => (s.theme === "dark" ? "roc-dark" : "roc-light"));
  const isPreviewOnly = kind !== "text";

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setEditMode(false);
    setDirty(false);

    const fail = (e: unknown) => {
      if (cancelled) return;
      push("error", `打开失败：${formatError(e)}`);
      setLoading(false);
    };

    (async () => {
      let resolvedKind = classifyPreview(path);
      if (resolvedKind === "text" && hasNoExtension(path)) {
        try {
          if (await sftpService.peekIsBinary(profileId, path)) resolvedKind = "executable";
        } catch {
          // 嗅探失败不影响后续正常走文本读取路径。
        }
      }
      if (cancelled) return;
      setKind(resolvedKind);

      if (resolvedKind === "unsupported-binary") {
        // 不读内容——直接给"用系统默认程序打开"的入口。
        setLoading(false);
        return;
      }

      if (resolvedKind === "executable") {
        setBinaryInfo(null);
        setBinaryInfoError(null);
        sftpService.inspectBinary(profileId, path).then(
          (info) => {
            if (cancelled) return;
            setBinaryInfo(info);
            setLoading(false);
          },
          (e) => {
            if (cancelled) return;
            setBinaryInfoError(formatError(e));
            setLoading(false);
          },
        );
        return;
      }

      if (resolvedKind === "jar") {
        setJarInfo(null);
        setJarInfoError(null);
        sftpService.inspectJar(profileId, path).then(
          (info) => {
            if (cancelled) return;
            setJarInfo(info);
            setLoading(false);
          },
          (e) => {
            if (cancelled) return;
            setJarInfoError(formatError(e));
            setLoading(false);
          },
        );
        return;
      }

      if (resolvedKind === "legacy-office") {
        setLegacyOfficeError(null);
        sftpService.convertLegacyOfficeToPdf(profileId, path).then(
          (base64) => {
            if (cancelled) return;
            setContent(`data:application/pdf;base64,${base64}`);
            setLoading(false);
          },
          (e) => {
            if (cancelled) return;
            setLegacyOfficeError(formatError(e));
            setLoading(false);
          },
        );
        return;
      }

      if (resolvedKind === "image" || resolvedKind === "pdf") {
        sftpService.readBinaryPreview(profileId, path).then((base64) => {
          if (cancelled) return;
          setContent(`data:${binaryMimeType(path)};base64,${base64}`);
          setLoading(false);
        }, fail);
        return;
      }

      if (resolvedKind === "word") {
        sftpService.readBinaryPreview(profileId, path).then(async (base64) => {
          if (cancelled) return;
          const { value: html } = await mammoth.convertToHtml({ arrayBuffer: base64ToArrayBuffer(base64) }, { convertImage: mammothImageConverter });
          if (cancelled) return;
          setContent(DOMPurify.sanitize(html));
          setLoading(false);
        }, fail);
        return;
      }

      if (resolvedKind === "excel") {
        sftpService.readBinaryPreview(profileId, path).then((base64) => {
          if (cancelled) return;
          const workbook = XLSX.read(base64, { type: "base64" });
          setSheets(
            workbook.SheetNames.map((name) => ({
              name,
              rows: XLSX.utils.sheet_to_json<(string | number)[]>(workbook.Sheets[name], { header: 1, defval: "" }),
            })),
          );
          setLoading(false);
        }, fail);
        return;
      }

      sftpService.readFile(profileId, path).then((res) => {
        if (cancelled) return;
        setContent(res.text);
        setMtime(res.mtime);
        setTruncated(res.truncated);
        setTotalSize(res.total_size);
        setLoading(false);
      }, fail);
    })();

    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [profileId, path]);

  const handleSave = async () => {
    if (isPreviewOnly) {
      push("error", "该文件类型不支持编辑保存");
      return;
    }
    if (truncated) {
      push("error", "文件过大，当前只是只读预览，无法保存");
      return;
    }
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

  const handleOpenExternally = async () => {
    try {
      await sftpService.openExternally(profileId, path);
    } catch (e) {
      push("error", `打开失败：${formatError(e)}`);
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
          {isPreviewOnly ? (
            <button className="btn ghost sm" onClick={handleOpenExternally} title="用系统默认程序打开">
              <ExternalLink style={{ width: 14, height: 14 }} /> 用系统程序打开
            </button>
          ) : (
            <>
              {dirty && <span className="dirty-dot" title="未保存" />}
              {!editMode ? (
                <button
                  className="btn ghost sm"
                  onClick={() => setEditMode(true)}
                  disabled={truncated}
                  title={truncated ? "文件过大，当前只是只读预览，无法编辑" : undefined}
                >
                  ✎ 编辑
                </button>
              ) : (
                <button className="btn ghost sm" onClick={handleSave}>💾 保存</button>
              )}
            </>
          )}
          <button className="btn ghost sm" onClick={onBack}>↩ 返回</button>
        </div>
      </div>

      {!isPreviewOnly && truncated && (
        <div className="editor-large-file-banner">
          文件过大（{formatBytes(totalSize)}），仅预览前 {formatBytes(content.length)}，只读模式，无法编辑保存。
        </div>
      )}

      {loading ? (
        <div style={{ padding: 16, fontSize: 12, color: "var(--text-secondary)" }}>加载中…</div>
      ) : kind === "image" ? (
        <div
          style={{
            flex: 1,
            minHeight: 0,
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
            overflow: "auto",
            background: "var(--bg-base)",
          }}
        >
          <img src={content} alt={path} style={{ maxWidth: "100%", maxHeight: "100%", objectFit: "contain" }} />
        </div>
      ) : kind === "pdf" ? (
        <PdfPreview base64={content.split(",")[1] ?? ""} />
      ) : kind === "word" ? (
        <div style={{ flex: 1, minHeight: 0, overflow: "auto", background: "var(--bg-base)" }}>
          <div className="office-word-preview" dangerouslySetInnerHTML={{ __html: content }} />
        </div>
      ) : kind === "excel" ? (
        <ExcelPreview sheets={sheets} />
      ) : kind === "executable" ? (
        <BinaryInfoPanel path={path} info={binaryInfo} error={binaryInfoError} onOpenExternally={handleOpenExternally} />
      ) : kind === "jar" ? (
        <JarInfoPanel path={path} info={jarInfo} error={jarInfoError} onOpenExternally={handleOpenExternally} />
      ) : kind === "legacy-office" ? (
        <LegacyOfficePreview path={path} content={content} error={legacyOfficeError ?? undefined} onOpenExternally={handleOpenExternally} />
      ) : kind === "unsupported-binary" ? (
        <UnsupportedBinaryPanel path={path} onOpenExternally={handleOpenExternally} />
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
              readOnly: !editMode || truncated,
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
