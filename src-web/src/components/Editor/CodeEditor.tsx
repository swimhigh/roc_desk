import React from "react";
import Editor, { DiffEditor } from "@monaco-editor/react";
import { KeyCode, KeyMod, Range, editor as monacoEditorApi, Uri as monacoUri } from "monaco-editor";
import { Save, Search, X, Eye, EyeOff, ExternalLink, GitCompare } from "lucide-react";
import { useEditorStore, isDiffId } from "../../stores/editorStore";
import { ConflictDialog } from "./ConflictDialog";
import { EncodingMenu } from "./EncodingMenu";
import { ExcelPreview } from "./ExcelPreview";
import { ImageViewer } from "./ImageViewer";
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
import { symbolService } from "../../services/symbolService";
import type { SymbolLocation } from "../../types/bindings";
import { FileStack } from "lucide-react";
import { useExplorerStore } from "../../stores/explorerStore";

/** 只会有一份 CodeEditor 挂载（App.tsx 的"无工作区"/"游离文件"/"工作区"三种编辑器壳
 * 是同一棵渲染树的不同分支，不会同时存在两份），下面给 Monaco 全局注册一次的"转到
 * 定义/声明" provider 拿不到 React props，只能靠这个模块级 ref 读到"当前是哪个工作区"
 * （游离文件模式没有工作区概念，ref 是 null 时 provider 直接不生效）。 */
const activeWorkspaceIdRef: { current: string | null } = { current: null };
/** provideDefinition 为每个候选位置生成的 Monaco Uri -> 真实符号位置的映射，供
 * `registerEditorOpener` 把"要打开哪个 Uri"还原回真实文件路径。不能反向解析
 * monaco.Uri（`@monaco-editor/react` 用 `Uri.parse(path)` 建模型，Windows 路径
 * 的盘符冒号会被误判成 URI scheme，这是它自己的既有行为——只能顺着同一个方向查表，
 * 不能从 Uri 反推回原始路径字符串）。 */
const uriToSymbolLocation = new Map<string, SymbolLocation>();
let definitionProviderRegistered = false;
/** Monaco 内置支持右键"转到定义/转到声明"的语言 id（和 utils/language.ts 里
 * detectLanguage 产出的 id 对齐）——本轮符号索引只覆盖这几种语言，见
 * src-tauri/src/symbols/mod.rs。 */
const DEFINITION_LANGUAGES = ["c", "cpp", "rust", "python", "go", "javascript", "typescript"];

function registerGoToDefinitionOnce(monaco: typeof import("monaco-editor")) {
  if (definitionProviderRegistered) return;
  definitionProviderRegistered = true;

  const resolve = async (
    model: import("monaco-editor").editor.ITextModel,
    position: import("monaco-editor").Position,
  ) => {
    const workspaceId = activeWorkspaceIdRef.current;
    if (!workspaceId) return null;
    const word = model.getWordAtPosition(position);
    if (!word) return null;
    let locations: SymbolLocation[];
    try {
      locations = await symbolService.goToDefinition(workspaceId, word.word);
    } catch {
      return null;
    }
    if (locations.length === 0) return null;
    return locations.map((loc) => {
      const uri = monaco.Uri.parse(loc.path);
      uriToSymbolLocation.set(uri.toString(), loc);
      return { uri, range: new monaco.Range(loc.line, 1, loc.line, 1) };
    });
  };

  monaco.languages.registerDefinitionProvider(DEFINITION_LANGUAGES, { provideDefinition: resolve });
  monaco.languages.registerDeclarationProvider(DEFINITION_LANGUAGES, { provideDeclaration: resolve });

  // 目标位置在另一个还没打开的文件里时，Monaco 自己不知道怎么"打开"一个任意路径——
  // 这个 opener 就是那座桥：查表拿到真实路径后，复用 Explorer 单击打开文件的同一条
  // 路径（editorStore.openPreview），打开后再跳到目标行（复用搜索结果跳转用的
  // pendingReveal 机制，见下面 CodeEditor 组件里消费 pendingReveal 的 effect）。
  monaco.editor.registerEditorOpener({
    openCodeEditor: (_source, resource) => {
      const loc = uriToSymbolLocation.get(resource.toString());
      const workspaceId = activeWorkspaceIdRef.current;
      if (!loc || !workspaceId) return false;
      void (async () => {
        const store = useEditorStore.getState();
        await store.openPreview(workspaceId, loc.path);
        store.revealLine(loc.path, loc.line);
      })();
      return true;
    },
  });
}

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

  // "转到定义/声明"（2026-09-16 需求）：让上面模块级注册的 Monaco provider 知道
  // "现在是哪个工作区"；工作区一打开就顺带建一次符号索引（失败不打扰用户，只是
  // 这个功能暂时用不了）。游离文件模式 workspaceId 是 null，provider 直接跳过。
  React.useEffect(() => {
    activeWorkspaceIdRef.current = workspaceId;
  }, [workspaceId]);
  React.useEffect(() => {
    if (!workspaceId) return;
    void symbolService.buildIndex(workspaceId).catch(() => {});
  }, [workspaceId]);

  // 切标签时光标/滚动位置丢失（2026-09-16 用户反馈）：根因是下面 `<Editor
  // key={active.path}>` 每切一个文件都整个卸载重挂载一次 Monaco 实例——
  // `@monaco-editor/react` 自带按路径缓存 view state 的机制（`saveViewState`
  // prop，默认开启），但它只在传了 `keepCurrentModel` 时才会在卸载前把 view
  // state 存下来（见该库源码，`keepCurrentModel` 为 false 时卸载分支直接
  // dispose 掉 model，根本不存 view state），不传就是"每次切走都白丢"。
  // 加 `keepCurrentModel` 解决了丢失问题，代价是模型不再随标签切换自动释放——
  // 于是这里补一个"真正关掉标签才释放模型"的清理：比较 `order` 变化，对
  // 消失的路径手动 dispose，避免一个会话里打开过的每个文件的模型永久占内存。
  const prevOrderRef = React.useRef(order);
  React.useEffect(() => {
    const removed = prevOrderRef.current.filter((p) => !order.includes(p) && !isDiffId(p));
    for (const path of removed) {
      monacoEditorApi.getModel(monacoUri.parse(path))?.dispose();
    }
    prevOrderRef.current = order;
  }, [order]);

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

  const revealInExplorer = React.useCallback(async (path: string) => {
    if (!workspaceId) {
      window.dispatchEvent(new CustomEvent("roc:reveal-standalone", { detail: { path } }));
      return;
    }
    const explorer = useExplorerStore.getState();
    const normalized = path.replace(/\\/g, "/");
    const root = rootPath.replace(/\\/g, "/").replace(/\/$/, "");
    const parts = normalized.split("/").filter(Boolean);
    const rootParts = root.split("/").filter(Boolean);
    // Expand every ancestor below workspace root so the target entry becomes visible.
    for (let i = rootParts.length; i < parts.length - 1; i++) {
      const dir = parts.slice(0, i + 1).join("/");
      if (!explorer.expanded.has(dir)) await explorer.toggleDir(workspaceId, dir);
    }
    explorer.select(path);
    window.dispatchEvent(new CustomEvent("roc:reveal-explorer", { detail: { path } }));
  }, [workspaceId, rootPath]);

  // 打开的文件左侧 Explorer 树不联动（2026-09 用户反馈）——之前只有手动点标签页/
  // 面包屑才会调 `revealInExplorer`，AI 工具面板打开文件（`App.tsx` 的
  // `onOpenFile` 直接调 `openPreview`）完全绕过了这条路径。改成响应式：只要
  // "当前激活的文件"变了（不管是谁触发的——Explorer 点击、AI 面板打开、标签页
  // 切换），都自动展开/定位一次，不用每个打开文件的入口各自记得调用一遍。
  // 幂等操作（展开已展开的目录、选中已选中的项）重复调用无副作用，不需要额外
  // 判断"是不是已经点过 Explorer 触发的"。
  React.useEffect(() => {
    if (active?.path) void revealInExplorer(active.path);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active?.path]);

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
      // 保存成功后增量重建这一个文件的符号索引，让"转到定义"很快就能看到新写的
      // 函数/类型——不用等下次重新打开工作区触发全量扫描。游离文件没有工作区可挂，
      // 不接这个索引。
      const savedBuf = useEditorStore.getState().buffers[activePath];
      if (workspaceId && savedBuf?.origin === "workspace" && savedBuf.kind === "text") {
        void symbolService.reindexFile(workspaceId, activePath, savedBuf.content).catch(() => {});
      }
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
            onClick={() => {
              setActive(path);
              // 点标签页联动左侧 Explorer 选中当前文件（2026-09 用户需求，参考
              // VS Code 的 "Auto Reveal"）——游离文件（不属于工作区）和对比标签
              // 没有对应的 Explorer 条目可选，跳过；`revealInExplorer` 已经是
              // 面包屑点击复用的同一套"展开祖先目录 + 选中"逻辑，不需要另写一份。
              if (buf?.origin === "workspace") void revealInExplorer(path);
            }}
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

  // PDF 标签切走再切回来会整个重新加载、滚动位置也跳回顶部——原因是内容区只按
  // `active.kind` 走三元分支渲染，切到别的标签时 `isPdf` 变 false，React 直接把
  // 整棵 `<PdfPreview>` 子树连同它内部状态（pdf.js 文档、滚动位置、缩放、搜索）
  // 卸载掉；切回来时是全新挂载，不是恢复（2026-09 用户反馈）。修法参考同一个
  // 组件里 Markdown/HTML 预览已经用过的思路（439 行注释）：不给每个 PDF 标签各自
  // 一份"只在 active 时挂载"的实例，而是给**所有当前打开着的** PDF 标签各常驻一份
  // `PdfPreview`，用 `visible` 控制 `display`，标签页存在期间实例不销毁。下面
  // `isPdf` 分支因此改成渲染 `null`——真正可见的那个实例由 `pdfLayer` 提供。
  const pdfBuffers = order
    .map((path) => buffers[path])
    .filter((buf): buf is NonNullable<typeof buf> => Boolean(buf) && buf.kind === "pdf");
  const pdfLayer = pdfBuffers.map((buf) => (
    <PdfPreview key={buf.path} base64={buf.content.split(",")[1] ?? ""} visible={buf.path === activePath} />
  ));

  // Windows 本地工作区下 `p`（buffer 路径，来自 Explorer 的 `entry.path`，后端
  // `fsops/local.rs::list_dir` 统一正规化成 `/`）和 `rootPath`（原生目录选择器
  // 选出来的，保留系统原样的 `\`）分隔符不一致，直接 `startsWith` 永远不命中，
  // 面包屑会显示完整绝对路径的每一段而不是相对路径（2026-09 用户反馈同一个根因
  // 在 Explorer"复制相对路径"上的表现，这里是同一个 bug 的另一个出现点）。两边都
  // 正规化成 `/` 再按小写比较——Windows 路径大小写不敏感。
  const relOf = (p: string) => {
    const normalizedRoot = rootPath.replace(/\\/g, "/").replace(/\/$/, "");
    const normalizedPath = p.replace(/\\/g, "/");
    return normalizedPath.toLowerCase().startsWith(normalizedRoot.toLowerCase())
      ? normalizedPath.slice(normalizedRoot.length).replace(/^\//, "")
      : p;
  };

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
          {pdfLayer}
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
        {pdfLayer}
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
              <span className={i === segments.length - 1 ? "crumb current" : "crumb"} onClick={() => {
                if (i === 0 || !active) return;
                const separator = active.path.includes("\\") ? "\\" : "/";
                const pathSegs = active.path.split(/[/\\]/).filter(Boolean);
                // standalone 面包屑包含“本地文件”前缀；工作区面包屑第一项是工作区名，
                // 后续项是相对根目录路径。始终把点击项映射回完整绝对路径。
                const target = active.origin === "standalone"
                  ? pathSegs.slice(0, i).join(separator)
                  : [rootPath.replace(/[\\/]$/, ""), ...segments.slice(1, i)].join(separator);
                void revealInExplorer(target);
              }}>{seg}</span>
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
        <ImageViewer src={active.content} alt={active.path} />
      ) : isPdf ? null : isWord ? (
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
            keepCurrentModel
            language={language}
            value={active.content}
            theme={monacoTheme}
            onChange={(value) => updateContent(active.path, value ?? "")}
            onMount={(editor, monaco) => {
              editorRef.current = editor;
              // provider 注册在 monaco 命名空间级别、只需要一次——`key={active.path}`
              // 导致每切一个文件就重新挂载一个新的 Editor 实例并重新触发 onMount，
              // 内部用 definitionProviderRegistered 挡掉重复注册。
              registerGoToDefinitionOnce(monaco);
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
      {pdfLayer}
    </div>
  );
};
