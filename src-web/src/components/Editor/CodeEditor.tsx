import React from "react";
import Editor from "@monaco-editor/react";
import { KeyCode, KeyMod } from "monaco-editor";
import { Save, Search, X, Eye, EyeOff } from "lucide-react";
import { useEditorStore } from "../../stores/editorStore";
import { ConflictDialog } from "./ConflictDialog";
import { EncodingMenu } from "./EncodingMenu";
import { useToastStore } from "../shared/Toast";
import { detectLanguage } from "../../utils/language";
import { formatError } from "../../utils/error";
import { renderMarkdown } from "../../utils/markdown";
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
    pendingReveal,
    clearReveal,
  } = useEditorStore();
  const push = useToastStore((s) => s.push);
  const monacoTheme = useThemeStore((s) => (s.theme === "dark" ? "vs-dark" : "vs"));
  const editorRef = React.useRef<import("monaco-editor").editor.IStandaloneCodeEditor | null>(null);
  const [previewOpen, setPreviewOpen] = React.useState(false);

  const active = activePath ? buffers[activePath] : null;
  const language = active ? detectLanguage(active.path) : "plaintext";
  const isMarkdown = language === "markdown";
  // 只在预览真正打开时才解析，避免每次敲字符都跑一遍 marked（非 Markdown 文件更是完全不需要）。
  const previewHtml = React.useMemo(
    () => (isMarkdown && previewOpen && active ? renderMarkdown(active.content) : ""),
    [isMarkdown, previewOpen, active?.content],
  );

  // 搜索面板点一个匹配行之后要求跳转（DESIGN.md 左侧目录树搜索功能）：这里统一处理
  // "文件已经打开、只是切换 active" 和 "editor 实例刚挂载" 两种情况——后者靠
  // `active?.path` 变化触发这个 effect 重跑，届时子组件 <Editor> 的 onMount 已经在
  // 同一次 commit 里跑过（子组件的挂载 effect 先于父组件自己的 effect），
  // editorRef.current 保证已经是最新的。
  React.useEffect(() => {
    if (!pendingReveal || !editorRef.current) return;
    if (pendingReveal.path !== active?.path) return;
    editorRef.current.revealLineInCenter(pendingReveal.line);
    editorRef.current.setPosition({ lineNumber: pendingReveal.line, column: 1 });
    editorRef.current.focus();
    clearReveal();
  }, [pendingReveal, active?.path, clearReveal]);

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
          {isMarkdown && (
            <button
              className={`btn ghost sm ${previewOpen ? "active" : ""}`}
              onClick={() => setPreviewOpen((v) => !v)}
              title="Markdown 预览 (Ctrl+Shift+V)"
            >
              {previewOpen ? <EyeOff style={{ width: 14, height: 14 }} /> : <Eye style={{ width: 14, height: 14 }} />} 预览
            </button>
          )}
          <button className="btn ghost sm" onClick={handleSave} title="保存 (Ctrl+S)">
            <Save style={{ width: 14, height: 14 }} /> 保存
          </button>
        </div>
      </div>
      <div style={{ flex: 1, minHeight: 0, position: "relative" }}>
        {/* 参考 VS Code 默认的 Ctrl+Shift+V"打开预览"（不是 Ctrl+K V 那种分栏预览）：
            预览会整体替换掉编辑区域，不是和源码并排——两者占同一块区域，用 display
            互斥切换，Monaco 实例不销毁（只是隐藏），切回编辑态时光标/滚动位置都还在。 */}
        <div style={{ display: previewOpen ? "none" : "block", width: "100%", height: "100%" }}>
          <Editor
            key={active.path}
            path={active.path}
            language={language}
            value={active.content}
            theme={monacoTheme}
            onChange={(value) => updateContent(active.path, value ?? "")}
            onMount={(editor) => {
              editorRef.current = editor;
              editor.addCommand(KeyMod.CtrlCmd | KeyCode.KeyS, () => {
                handleSave();
              });
              if (isMarkdown) {
                editor.addCommand(KeyMod.CtrlCmd | KeyMod.Shift | KeyCode.KeyV, () => {
                  setPreviewOpen((v) => !v);
                });
              }
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
        {isMarkdown && previewOpen && (
          <div
            className="markdown-preview"
            style={{ width: "100%", height: "100%", overflowY: "auto" }}
            dangerouslySetInnerHTML={{ __html: previewHtml }}
          />
        )}
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
