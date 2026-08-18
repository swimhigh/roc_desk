import React from "react";
import Editor from "@monaco-editor/react";
import { KeyCode, KeyMod } from "monaco-editor";
import { Save, Search, X } from "lucide-react";
import { useEditorStore } from "../../stores/editorStore";
import { ConflictDialog } from "./ConflictDialog";
import { EncodingMenu } from "./EncodingMenu";
import { useToastStore } from "../shared/Toast";
import { detectLanguage } from "../../utils/language";
import { formatError } from "../../utils/error";
import { useThemeStore } from "../../stores/themeStore";

interface CodeEditorProps {
  workspaceId: string;
  workspaceName: string;
  rootPath: string;
}

/**
 * 通用可编辑代码编辑器（DESIGN.md §3.1.4 / §2.2 选定的 Monaco 内核）。
 * 本地缓冲区编辑 + Ctrl+S 保存回写 + mtime 冲突检测，JSON/Markdown/日志/配置文件等
 * 常见类型按扩展名给语言高亮（`utils/language.ts`），远程文件走 SFTP 写回但用户侧体验一致。
 */
export const CodeEditor: React.FC<CodeEditorProps> = ({ workspaceId, workspaceName, rootPath }) => {
  const {
    buffers,
    order,
    activePath,
    setActive,
    pin,
    close,
    updateContent,
    save,
    conflict,
    resolveConflict,
    reopenWithEncoding,
    saveWithEncoding,
  } = useEditorStore();
  const push = useToastStore((s) => s.push);
  const monacoTheme = useThemeStore((s) => (s.theme === "dark" ? "vs-dark" : "vs"));
  const editorRef = React.useRef<import("monaco-editor").editor.IStandaloneCodeEditor | null>(null);

  const active = activePath ? buffers[activePath] : null;

  const handleSave = React.useCallback(async () => {
    if (!activePath) return;
    try {
      await save(workspaceId, activePath);
      push("success", "已保存");
    } catch (e) {
      push("error", `保存失败：${formatError(e)}`, { label: "重试保存", onClick: handleSave });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [workspaceId, activePath]);

  const fileName = (path: string) => path.split(/[/\\]/).filter(Boolean).pop() ?? path;

  const tabStrip = order.length > 0 && (
    <div className="editor-tabs">
      {order.map((path) => {
        const buf = buffers[path];
        if (!buf) return null;
        return (
          <div
            key={path}
            className={`editor-tab ${path === activePath ? "active" : ""} ${buf.isPreview ? "preview" : ""}`}
            title={path}
            onClick={() => setActive(path)}
            onDoubleClick={() => pin(path)}
          >
            <span className="editor-tab-name">{fileName(path)}</span>
            {buf.dirty && <span className="dirty-dot" />}
            <button
              className="editor-tab-close"
              title="关闭"
              onClick={(e) => {
                e.stopPropagation();
                close(path);
              }}
            >
              <X />
            </button>
          </div>
        );
      })}
    </div>
  );

  if (!active) {
    return (
      <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
        {tabStrip}
        <div
          style={{
            flex: 1,
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
            color: "var(--text-secondary)",
            fontSize: 13,
          }}
        >
          从左侧 Explorer 中选择一个文件开始编辑
        </div>
      </div>
    );
  }

  const relativePath = active.path.startsWith(rootPath)
    ? active.path.slice(rootPath.length).replace(/^[/\\]/, "")
    : active.path;
  const segments = [workspaceName, ...relativePath.split(/[/\\]/).filter(Boolean)];

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      {tabStrip}
      <div className="editor-toolbar">
        <div className="breadcrumb">
          {segments.map((seg, i) => (
            <span key={i}>
              {i > 0 && <span className="sep">›</span>}{" "}
              <span className={i === segments.length - 1 ? "crumb current" : "crumb"}>{seg}</span>
            </span>
          ))}
          {active.dirty && <span className="dirty-dot" style={{ marginLeft: 6 }} title="未保存" />}
        </div>
        <div style={{ marginLeft: "auto", display: "flex", gap: 4, alignItems: "center" }}>
          <EncodingMenu
            current={active.encoding}
            onReopen={async (enc) => {
              try {
                await reopenWithEncoding(workspaceId, active.path, enc);
              } catch (e) {
                push("error", `重新打开失败：${formatError(e)}`);
              }
            }}
            onSaveAs={async (enc) => {
              try {
                await saveWithEncoding(workspaceId, active.path, enc);
                push("success", `已以 ${enc} 编码保存`);
              } catch (e) {
                push("error", `保存失败：${formatError(e)}`);
              }
            }}
          />
          <button
            className="btn ghost sm"
            title="搜索 (Ctrl+F)"
            onClick={() => editorRef.current?.trigger("toolbar", "actions.find", null)}
          >
            <Search style={{ width: 14, height: 14 }} />
          </button>
          <button className="btn ghost sm" onClick={handleSave} title="保存 (Ctrl+S)">
            <Save style={{ width: 14, height: 14 }} /> 保存
          </button>
        </div>
      </div>
      <div style={{ flex: 1, minHeight: 0 }}>
        <Editor
          key={active.path}
          path={active.path}
          language={detectLanguage(active.path)}
          value={active.content}
          theme={monacoTheme}
          onChange={(value) => updateContent(active.path, value ?? "")}
          onMount={(editor) => {
            editorRef.current = editor;
            editor.addCommand(KeyMod.CtrlCmd | KeyCode.KeyS, () => {
              handleSave();
            });
          }}
          options={{
            fontSize: 13,
            fontFamily: "var(--font-mono)",
            minimap: { enabled: true },
            wordWrap: "on",
            automaticLayout: true,
            scrollBeyondLastLine: false,
          }}
        />
      </div>
      {conflict && (
        <ConflictDialog
          open
          path={conflict.path}
          onViewDiff={() => resolveConflict(workspaceId, "discard")}
          onSaveAsCopy={() => resolveConflict(workspaceId, "discard")}
          onOverwrite={() => resolveConflict(workspaceId, "overwrite")}
        />
      )}
    </div>
  );
};
