import React from "react";
import Editor, { DiffEditor } from "@monaco-editor/react";
import { KeyCode, KeyMod, Range } from "monaco-editor";
import { Save, Search, X, Eye, EyeOff, ExternalLink, GitCompare } from "lucide-react";
import { useEditorStore } from "../../stores/editorStore";
import { ConflictDialog } from "./ConflictDialog";
import { EncodingMenu } from "./EncodingMenu";
import { ExcelPreview } from "./ExcelPreview";
import { PdfPreview } from "./PdfPreview";
import { UnsupportedBinaryPanel } from "./UnsupportedBinaryPanel";
import { BinaryInfoPanel } from "./BinaryInfoPanel";
import { JarInfoPanel } from "./JarInfoPanel";
import { LegacyOfficePreview } from "./LegacyOfficePreview";
import { ContextMenu, type ContextMenuItem } from "../shared/ContextMenu";
import { useToastStore } from "../shared/Toast";
import { detectLanguage } from "../../utils/language";
import { formatError } from "../../utils/error";
import { renderMarkdown } from "../../utils/markdown";
import { useThemeStore } from "../../stores/themeStore";
import { formatBytes } from "../../utils/format";
import { inlineHtmlResources } from "../../utils/inlineHtmlResources";
import { fsService, localFileService } from "../../services/fsService";
import { FileStack } from "lucide-react";

interface CodeEditorProps {
  /** 没有打开工作区、只剩游离标签的极简编辑器壳（App.tsx 的"无工作区"三态之一）
   * 传 `null`——这种情况下打开的所有标签必然都是 `origin: "standalone"`。 */
  workspaceId: string | null;
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
    diffs,
    order,
    activePath,
    setActive,
    pin,
    close,
    closeOthers,
    closeAll,
    closeToLeft,
    closeToRight,
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
  // roc-dark/roc-light 是 monacoSetup.ts 里在 vs-dark/vs 基础上叠加的自定义主题
  // （加了日志文件的 token 配色规则），其它语言的高亮规则原样继承，不受影响。
  const monacoTheme = useThemeStore((s) => (s.theme === "dark" ? "roc-dark" : "roc-light"));
  const editorRef = React.useRef<import("monaco-editor").editor.IStandaloneCodeEditor | null>(null);
  const highlightDecorationsRef = React.useRef<import("monaco-editor").editor.IEditorDecorationsCollection | null>(null);
  const [previewOpen, setPreviewOpen] = React.useState(false);
  // Tab 右键菜单（参考 VS Code："关闭其他"/"关闭所有"/"关闭左侧的标签页"/
  // "关闭右侧的标签页"，2026-08-29 需求）。
  const [tabMenu, setTabMenu] = React.useState<{ x: number; y: number; path: string } | null>(null);
  // 底部状态栏的光标位置（参考 VS Code 右下角 "行, 列"）——diff.path 换了（切标签/
  // Monaco 实例因 key={active.path} 重新挂载）时清空，避免残留上一个文件的坐标。
  const [cursorPos, setCursorPos] = React.useState<{ line: number; column: number } | null>(null);

  const active = activePath ? buffers[activePath] : null;
  const activeDiff = activePath ? diffs[activePath] : null;
  React.useEffect(() => setCursorPos(null), [active?.path]);
  const isImage = active?.kind === "image";
  const isPdf = active?.kind === "pdf";
  const isWord = active?.kind === "word";
  const isExcel = active?.kind === "excel";
  const isExecutable = active?.kind === "executable";
  const isJar = active?.kind === "jar";
  const isLegacyOffice = active?.kind === "legacy-office";
  const isUnsupportedBinary = active?.kind === "unsupported-binary";
  // 这几种都是只读展示，不进 Monaco——编码菜单/搜索/Markdown 预览/保存这些跟文本
  // 编辑相关的工具栏按钮对它们没意义。
  const isPreviewOnly = isImage || isPdf || isWord || isExcel || isExecutable || isJar || isLegacyOffice || isUnsupportedBinary;
  const language = active ? detectLanguage(active.path) : "plaintext";
  const isMarkdown = language === "markdown";
  // HTML 文件预览（2026-08-29 需求）：和 Markdown 预览共用同一套"编辑/预览切换"
  // 交互（Ctrl+Shift+V、同一块区域互斥展示），只是渲染方式不同——Markdown 渲染出的
  // 是一段 HTML 片段，用 dangerouslySetInnerHTML 塞进当前页面就行；用户打开的
  // *.html 文件本身是一份完整文档（可能带 <style>/<script>/<meta charset>），
  // 必须用 <iframe> 给它一个独立的文档上下文才能正确渲染，不能直接扔进
  // dangerouslySetInnerHTML（那样会当成 body 片段解析，<head> 里的东西全部丢失，
  // <script> 也不会执行）。
  const isHtml = language === "html";
  const canPreview = isMarkdown || isHtml;
  // 只在预览真正打开时才解析，避免每次敲字符都跑一遍 marked（非 Markdown 文件更是完全不需要）。
  const previewHtml = React.useMemo(
    () => (isMarkdown && previewOpen && active ? renderMarkdown(active.content) : ""),
    [isMarkdown, previewOpen, active?.content],
  );

  // HTML 预览的 srcDoc 内容——不能直接用 active.content：iframe srcdoc 里的相对路径
  // 资源引用（<link href>/<script src>/<img src>）默认相对宿主页面（roc_desk 自己
  // 的地址）解析，会 404，引用了外部 CSS/JS 的页面预览出来一片白屏（2026-08-28
  // 用户反馈）。打开预览时异步把这些资源抓下来内联成 data URL 再喂给 iframe，见
  // `utils/inlineHtmlResources.ts`。转换过程中先显示原始内容（没有外部资源的简单
  // 页面本来就能正常显示，不用等），转换完成后再替换成内联版本。
  const [htmlPreviewSrc, setHtmlPreviewSrc] = React.useState<string | null>(null);
  const [inliningResources, setInliningResources] = React.useState(false);
  React.useEffect(() => {
    if (!isHtml || !previewOpen || !active) {
      setHtmlPreviewSrc(null);
      return;
    }
    let cancelled = false;
    setInliningResources(true);
    const baseDir = active.path.replace(/\\/g, "/").split("/").slice(0, -1).join("/");
    const readPreview = (p: string) =>
      active.origin === "standalone" ? localFileService.readBinaryPreview(p) : fsService.readBinaryPreview(workspaceId!, p);
    inlineHtmlResources(active.content, baseDir, readPreview).then(
      (html) => {
        if (cancelled) return;
        setHtmlPreviewSrc(html);
        setInliningResources(false);
      },
      () => {
        if (cancelled) return;
        setInliningResources(false);
      },
    );
    return () => {
      cancelled = true;
    };
  }, [isHtml, previewOpen, active?.path, active?.content, workspaceId]);

  // 搜索面板点一个匹配行之后要求跳转（DESIGN.md 左侧目录树搜索功能）：这里统一处理
  // "文件已经打开、只是切换 active" 和 "editor 实例刚挂载" 两种情况——后者靠
  // `active?.path` 变化触发这个 effect 重跑，届时子组件 <Editor> 的 onMount 已经在
  // 同一次 commit 里跑过（子组件的挂载 effect 先于父组件自己的 effect），
  // editorRef.current 保证已经是最新的。
  React.useEffect(() => {
    if (!pendingReveal || !editorRef.current) return;
    if (pendingReveal.path !== active?.path) return;
    const editor = editorRef.current;
    editor.revealLineInCenter(pendingReveal.line);
    editor.setPosition({ lineNumber: pendingReveal.line, column: 1 });
    editor.focus();

    // 点亮这个文件命中的全部位置（不只是点击跳转到的那一行），用户原话："通过搜索
    // 结果打开后的文本文件，需要点亮搜索到的内容"。字符级精确高亮（不是整行背景色），
    // 复用后端 SearchMatch 已经算好的字符下标，Monaco 的列号是 1-based 所以要 +1。
    if (pendingReveal.highlights && pendingReveal.highlights.length > 0) {
      const decorations = pendingReveal.highlights.map((h) => ({
        range: new Range(h.line, h.start + 1, h.line, h.end + 1),
        options: { inlineClassName: "search-match-highlight" },
      }));
      if (highlightDecorationsRef.current) {
        highlightDecorationsRef.current.set(decorations);
      } else {
        highlightDecorationsRef.current = editor.createDecorationsCollection(decorations);
      }
    } else {
      highlightDecorationsRef.current?.clear();
    }

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

  const handleOpenExternally = React.useCallback(async () => {
    if (!activePath || !active) return;
    try {
      if (active.origin === "standalone") await localFileService.openExternally(activePath);
      else await fsService.openExternally(workspaceId!, activePath);
    } catch (e) {
      push("error", `打开失败：${formatError(e)}`);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [workspaceId, activePath, active?.origin]);

  const fileName = (path: string) => path.split(/[/\\]/).filter(Boolean).pop() ?? path;

  const tabStrip = order.length > 0 && (
    <div className="editor-tabs">
      {order.map((path) => {
        const buf = buffers[path];
        const diff = diffs[path];
        if (!buf && !diff) return null;
        const label = diff ? `${fileName(diff.leftPath)} ↔ ${fileName(diff.rightPath)}` : fileName(path);
        return (
          <div
            key={path}
            className={`editor-tab ${path === activePath ? "active" : ""} ${buf?.isPreview ? "preview" : ""}`}
            title={diff ? `${diff.leftPath}  ↔  ${diff.rightPath}` : path}
            onClick={() => setActive(path)}
            onDoubleClick={() => buf && pin(path)}
            onContextMenu={(e) => {
              e.preventDefault();
              setTabMenu({ x: e.clientX, y: e.clientY, path });
            }}
          >
            {diff && <GitCompare className="editor-tab-diff-icon" />}
            {/* 游离文件（不属于当前工作区）在标签上加个小图标区分——title 已经是
                完整绝对路径，hover 能看到，这里只做"一眼扫过去就能分辨"的视觉提示。 */}
            {buf?.origin === "standalone" && (
              <span title="游离文件，不属于当前工作区" style={{ display: "flex" }}>
                <FileStack className="editor-tab-standalone-icon" />
              </span>
            )}
            <span className="editor-tab-name">{label}</span>
            {buf?.dirty && <span className="dirty-dot" />}
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
      {tabMenu && (
        <ContextMenu
          x={tabMenu.x}
          y={tabMenu.y}
          items={(() => {
            const { path } = tabMenu;
            const idx = order.indexOf(path);
            const items: ContextMenuItem[] = [{ label: "关闭", onClick: () => close(path) }];
            if (order.length > 1) items.push({ label: "关闭其他", onClick: () => closeOthers(path) });
            if (idx > 0) items.push({ label: "关闭左侧的标签页", onClick: () => closeToLeft(path) });
            if (idx >= 0 && idx < order.length - 1) items.push({ label: "关闭右侧的标签页", onClick: () => closeToRight(path) });
            items.push({ label: "关闭所有", onClick: () => closeAll(), separatorBefore: true });
            return items;
          })()}
          onClose={() => setTabMenu(null)}
        />
      )}
    </div>
  );

  const relOf = (p: string) => (p.startsWith(rootPath) ? p.slice(rootPath.length).replace(/^[/\\]/, "") : p);

  if (!active) {
    if (activeDiff) {
      return (
        <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
          {tabStrip}
          <div className="editor-toolbar">
            <div className="breadcrumb">
              <span className="crumb">{relOf(activeDiff.leftPath)}</span>
              <span className="sep" style={{ margin: "0 8px" }}>⇄</span>
              <span className="crumb current">{relOf(activeDiff.rightPath)}</span>
            </div>
          </div>
          <div style={{ flex: 1, minHeight: 0 }}>
            <DiffEditor
              language={activeDiff.language}
              original={activeDiff.leftContent}
              modified={activeDiff.rightContent}
              theme={monacoTheme}
              options={{
                readOnly: true,
                renderSideBySide: true,
                fontSize: 13,
                fontFamily: "var(--font-mono)",
                wordWrap: "on",
                automaticLayout: true,
                minimap: { enabled: false },
                scrollBeyondLastLine: false,
              }}
            />
          </div>
        </div>
      );
    }
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
          {workspaceId ? "从左侧 Explorer 中选择一个文件开始编辑" : "拖拽文件到此处，或按 Ctrl+O 打开文件"}
        </div>
      </div>
    );
  }

  // 游离文件不属于当前工作区，面包屑用"本地文件 › 完整路径"而不是"工作区名 › 相对路径"
  // ——套用工作区的相对路径规则毫无意义（rootPath 甚至可能不存在，比如无工作区的
  // 独立编辑器壳），完整路径才能让人分清这是磁盘上的哪个文件。
  const segments =
    active.origin === "standalone"
      ? ["本地文件", ...active.path.split(/[/\\]/).filter(Boolean)]
      : [workspaceName, ...relOf(active.path).split(/[/\\]/).filter(Boolean)];

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
          {isPreviewOnly && (
            <button className="btn ghost sm" onClick={handleOpenExternally} title="用系统默认程序打开">
              <ExternalLink style={{ width: 14, height: 14 }} /> 用系统程序打开
            </button>
          )}
          {!isPreviewOnly && (
            <>
              <button
                className="btn ghost sm"
                title="搜索 (Ctrl+F)"
                onClick={() => editorRef.current?.trigger("toolbar", "actions.find", null)}
              >
                <Search style={{ width: 14, height: 14 }} />
              </button>
              {canPreview && (
                <button
                  className={`btn ghost sm ${previewOpen ? "active" : ""}`}
                  onClick={() => setPreviewOpen((v) => !v)}
                  title={`${isHtml ? "HTML" : "Markdown"} 预览 (Ctrl+Shift+V)`}
                >
                  {previewOpen ? <EyeOff style={{ width: 14, height: 14 }} /> : <Eye style={{ width: 14, height: 14 }} />} 预览
                </button>
              )}
              <button
                className="btn ghost sm"
                onClick={handleSave}
                disabled={active.truncated}
                title={active.truncated ? "文件过大，当前只是只读预览，无法保存" : "保存 (Ctrl+S)"}
              >
                <Save style={{ width: 14, height: 14 }} /> 保存
              </button>
            </>
          )}
        </div>
      </div>
      {active.truncated && (
        <div className="editor-large-file-banner">
          文件过大（{formatBytes(active.totalSize)}），仅预览前 {formatBytes(active.content.length)}，只读模式，无法编辑保存。
        </div>
      )}
      {isImage ? (
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
          <img src={active.content} alt={active.path} style={{ maxWidth: "100%", maxHeight: "100%", objectFit: "contain" }} />
        </div>
      ) : isPdf ? (
        <PdfPreview base64={active.content.split(",")[1] ?? ""} />
      ) : isWord ? (
        <div style={{ flex: 1, minHeight: 0, overflow: "auto", background: "var(--bg-base)" }}>
          <div className="office-word-preview" dangerouslySetInnerHTML={{ __html: active.content }} />
        </div>
      ) : isExcel ? (
        <ExcelPreview sheets={active.sheets ?? []} />
      ) : isExecutable ? (
        <BinaryInfoPanel
          path={active.path}
          info={active.binaryInfo ?? null}
          error={active.binaryInfoError ?? null}
          onOpenExternally={handleOpenExternally}
        />
      ) : isJar ? (
        <JarInfoPanel
          path={active.path}
          info={active.jarInfo ?? null}
          error={active.jarInfoError ?? null}
          onOpenExternally={handleOpenExternally}
        />
      ) : isLegacyOffice ? (
        <LegacyOfficePreview
          path={active.path}
          content={active.content}
          error={active.legacyOfficeError}
          onOpenExternally={handleOpenExternally}
        />
      ) : isUnsupportedBinary ? (
        <UnsupportedBinaryPanel path={active.path} onOpenExternally={handleOpenExternally} />
      ) : (
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
              // 新文件挂载了一个全新的 Monaco 实例，旧的 decorations collection 对象
              // 属于已经销毁的上一个实例，不能带过来复用。
              highlightDecorationsRef.current = null;

              // 重启 roc_desk 后重新进工作区时，标签页恢复（App.tsx 的 localStorage
              // 恢复逻辑）会连续挂载好几个 Editor 实例，还会和"自动打开一个终端"的
              // 副作用并发抢布局——2026-08-29 用户反馈"重启再进工作区，一部分可编辑
              // 文件显示不出来"，用 CDP 远程调试连进实际渲染的页面查过：内容其实一直
              // 是对的（`fs_read_file` 返回正确文本），问题是 Monaco 自己的根节点在
              // 那个繁忙的挂载瞬间把容器测量成了 5x5 像素（父级 flex 容器当时还没
              // 完全撑开），`automaticLayout` 的 ResizeObserver 之后没有再纠正回来，
              // 切一下标签页（强制卸载重挂载 Monaco，此时布局已经稳定）就会恢复正常，
              // 证实是挂载时机问题，不是数据问题。等浏览器完成当前这轮布局/绘制后
              // （下一帧）再手动调一次 layout() 强制重新测量，避免赶上这个繁忙窗口。
              requestAnimationFrame(() => editor.layout());

              const pos = editor.getPosition();
              if (pos) setCursorPos({ line: pos.lineNumber, column: pos.column });
              editor.onDidChangeCursorPosition((e) => {
                setCursorPos({ line: e.position.lineNumber, column: e.position.column });
              });

              editor.addCommand(KeyMod.CtrlCmd | KeyCode.KeyS, () => {
                handleSave();
              });
              if (canPreview) {
                editor.addCommand(KeyMod.CtrlCmd | KeyMod.Shift | KeyCode.KeyV, () => {
                  setPreviewOpen((v) => !v);
                });
              }
            }}
            options={{
              readOnly: active.truncated,
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
        {isHtml && previewOpen && (
          <div style={{ position: "relative", width: "100%", height: "100%" }}>
            {inliningResources && (
              <div
                style={{
                  position: "absolute",
                  top: 8,
                  right: 8,
                  zIndex: 1,
                  fontSize: 11,
                  padding: "3px 8px",
                  borderRadius: "var(--radius-sm)",
                  background: "var(--bg-surface-raised)",
                  color: "var(--text-secondary)",
                  boxShadow: "var(--shadow)",
                }}
              >
                正在加载页面引用的外部资源…
              </div>
            )}
            <iframe
              // key 换成 active.path：切换到另一个 html 文件时强制换一个全新的
              // iframe，避免上一个文档残留的全局状态（比如它自己的定时器/DOM 引用）
              // 泄漏进下一份预览。
              key={active.path}
              title={`${active.path} 预览`}
              srcDoc={htmlPreviewSrc ?? active.content}
              // 特意不给 allow-same-origin：预览的是用户自己打开的文件，允许里面的
              // <script> 跑起来（这就是"预览"的意义，和浏览器直接打开这个文件行为一致），
              // 但不给它同源权限去摸这个 Tauri 应用自身的 window/IPC 桥。
              sandbox="allow-scripts allow-forms allow-modals allow-popups"
              style={{ width: "100%", height: "100%", border: "none", background: "#fff" }}
            />
          </div>
        )}
      </div>
      )}
      {/* 底部状态栏（参考 VS Code 右下角编码/语言/光标位置徽标）：文件编码之前只是
          工具栏里一个不起眼的小按钮，容易被当成普通图标划过去（2026-08-29 用户反馈
          "要在某个地方能看到文件编码"）——挪到固定可见的状态栏，不需要点开任何东西
          就能看到当前编码，点击仍然能唤出"重新打开为"/"保存为"两组动作。 */}
      {!isPreviewOnly && (
        <div className="editor-status-bar">
          <div className="status-left">
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
            <span className="status-item">{language}</span>
          </div>
          <div className="status-right">
            {cursorPos && (
              <span className="status-item">
                第 {cursorPos.line} 行，第 {cursorPos.column} 列
              </span>
            )}
            <span className="status-item">{formatBytes(active.totalSize || active.content.length)}</span>
          </div>
        </div>
      )}
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
