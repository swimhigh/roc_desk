import { create } from "zustand";
import * as XLSX from "xlsx";
import mammoth from "mammoth";
import DOMPurify from "dompurify";
import { fsService } from "../services/fsService";
import {
  classifyPreview,
  hasNoExtension,
  binaryMimeType,
  base64ToArrayBuffer,
  mammothImageConverter,
  type PreviewKind,
  type ExcelSheet,
} from "../utils/previewFile";
import { detectLanguage } from "../utils/language";
import type { BinaryInfo, JarInfo } from "../types/bindings";
import { formatError } from "../utils/error";

/** diff 标签页的 id 前缀——真实文件路径不可能长这样（Windows 盘符是单字母+冒号，
 * Unix/远程路径以 `/` 开头），用来在同一个 `order` 数组里区分"这一项是文件 buffer
 * 还是对比标签"，不用另起一个和 buffers 平行、还要单独维护顺序的列表。 */
const DIFF_ID_PREFIX = "diff:";
export const isDiffId = (id: string) => id.startsWith(DIFF_ID_PREFIX);

/** 工作区文本文件对比（参考 VS Code 的 "Select for Compare" / "Compare with Selected"）：
 * 两侧都是只读快照，不是可编辑 buffer——对比结果本身不该被保存，要改内容回去改
 * 原文件、重新打开一次对比。 */
export interface DiffBuffer {
  id: string;
  leftPath: string;
  rightPath: string;
  leftContent: string;
  rightContent: string;
  language: string;
}

export interface EditorBuffer {
  path: string;
  /** 含义随 kind 变化：text 是文件文本；image/pdf 是 `data:mime;base64,...`；
   * word 是 mammoth 转换并经 DOMPurify 净化后的 HTML；excel/executable/unsupported-binary
   * 不用这个字段。 */
  content: string;
  mtime: number;
  dirty: boolean;
  isPreview: boolean; // 单击=预览态，双击=固定态（UI_DESIGN.md §3.3）
  encoding: string; // 探测到（或用户强制指定）的编码，状态栏展示用（参考 VS Code）
  /** 文件总字节数，来自后端 FileContent.total_size（不是 content 的长度）。 */
  totalSize: number;
  /** 文件过大，content 只是截断预览——此时编辑器应只读，不能保存（2026-08-28
   * 用户反馈 >1GB 文件打开卡死后加的保护，见后端 fsops::EDITOR_PREVIEW_THRESHOLD_BYTES）。 */
  truncated: boolean;
  /** 非 "text" 都是只读展示、不进 Monaco（2026-08-28 用户反馈图片/PDF/Word/Excel
   * 被当文本打开显示乱码）。 */
  kind: PreviewKind;
  /** kind === "excel" 时才有值。 */
  sheets?: ExcelSheet[];
  /** kind === "executable" 时才有值——EXE/DLL/SO 的基本信息 + 依赖库列表
   * （2026-08-28 需求，见 fsops::binary_info）。 */
  binaryInfo?: BinaryInfo;
  /** kind === "executable" 且解析失败时的错误信息（损坏文件、静态库归档等）——
   * 仍然要能打开这个 Tab，只是展示成"解析失败 + 用系统程序打开"，不是直接报错
   * 让 Tab 都开不出来。 */
  binaryInfoError?: string;
  /** kind === "jar" 时才有值——manifest/Main-Class/Class-Path + 内部条目列表
   * （2026-08-28 需求，见 fsops::jar_info）。 */
  jarInfo?: JarInfo;
  jarInfoError?: string;
  /** kind === "legacy-office" 且转换失败时的错误信息（通常是没装 LibreOffice）——
   * 和 binaryInfoError/jarInfoError 同样的退化模式，Tab 仍然打得开，只是展示成
   * "转换失败 + 用系统程序打开"。转换成功时 PDF 走 content 字段（data URL），
   * 不单独加字段。 */
  legacyOfficeError?: string;
}

export interface SaveConflict {
  path: string;
  currentMtime: number;
  currentPreview: string;
}

export interface PendingHighlight {
  /** 1-based 行号 */
  line: number;
  /** 字符下标（不是字节下标），和 fsops::SearchMatch 的 match_start/match_end 对应。 */
  start: number;
  end: number;
}

export interface PendingReveal {
  path: string;
  line: number;
  /** 搜索面板打开文件时，把这个文件命中的所有位置一起点亮（2026-08-18 用户原话：
   * "通过搜索结果打开后的文本文件，需要点亮搜索到的内容"），不只是跳到点击的那一行。 */
  highlights?: PendingHighlight[];
}

interface EditorState {
  buffers: Record<string, EditorBuffer>;
  /** diff 标签页，key 是上面的 `diff:` 前缀 id，和 buffers 平级但分开存——一个 diff
   * 涉及两个路径、没有单一的 mtime/dirty 语义，套不进 EditorBuffer 的形状。 */
  diffs: Record<string, DiffBuffer>;
  /** 标签栏的展示顺序，元素是文件路径或 diff id，混在一起——Tab 关闭/排序这些操作
   * 天然就该对两种标签一视同仁。 */
  order: string[];
  activePath: string | null;
  conflict: SaveConflict | null;
  /** 搜索面板点一个匹配行 → 要求编辑器跳转到该文件的这一行（DESIGN.md 左侧目录树
   * 搜索功能，2026-08-18 需求）。CodeEditor 消费后自己清空，不在这里自动清。 */
  pendingReveal: PendingReveal | null;

  /** 单击文件：复用/替换预览标签，而不是无限开新标签 */
  openPreview: (workspaceId: string, path: string) => Promise<void>;
  /** 打开一个对比标签（Explorer 右键"选择进行比较"→"与所选文件比较"）。同一对路径
   * 已经开过就直接激活那个标签，不重复读盘。 */
  openDiff: (workspaceId: string, leftPath: string, rightPath: string) => Promise<void>;
  /** 双击文件：转为固定标签 */
  pin: (path: string) => void;
  setActive: (path: string) => void;
  updateContent: (path: string, content: string) => void;
  save: (workspaceId: string, path: string) => Promise<void>;
  resolveConflict: (
    workspaceId: string,
    resolution: "overwrite" | "discard",
  ) => Promise<void>;
  close: (path: string) => void;
  /** Tab 右键菜单（参考 VS Code）："关闭其他"——只留下这一个 Tab。 */
  closeOthers: (path: string) => void;
  /** "关闭所有" */
  closeAll: () => void;
  /** "关闭左侧的标签页"，按 `order` 里的顺序，不是打开时间顺序。 */
  closeToLeft: (path: string) => void;
  /** "关闭右侧的标签页" */
  closeToRight: (path: string) => void;
  revealLine: (path: string, line: number, highlights?: PendingHighlight[]) => void;
  clearReveal: () => void;
  /** "Reopen with Encoding"（参考 VS Code）：丢弃当前内容，强制按指定编码重新读取。*/
  reopenWithEncoding: (workspaceId: string, path: string, encodingLabel: string) => Promise<void>;
  /** "Save with Encoding"（参考 VS Code）：按指定编码写盘，而不是固定 UTF-8。*/
  saveWithEncoding: (workspaceId: string, path: string, encodingLabel: string) => Promise<void>;
  reset: () => void;
}

export const useEditorStore = create<EditorState>((set, get) => ({
  buffers: {},
  diffs: {},
  order: [],
  activePath: null,
  conflict: null,
  pendingReveal: null,

  openPreview: async (workspaceId, path) => {
    const existing = get().buffers[path];
    if (existing) {
      set({ activePath: path });
      return;
    }

    // 单击直接开一个常驻标签，不做"预览态复用/替换上一个标签"那一套——早期版本
    // 参照 VS Code 单击=预览/双击=固定的语义，但对不熟悉这个手势的用户来说，
    // 单击点开另一个文件却把原来那个标签"顶掉"，读出来就是"没法同时开多个文件"
    // （真实反馈，2026-08-18）。多文件同时编辑是更基础的诉求，优先满足它。
    const addBuffer = (buf: EditorBuffer) =>
      set((s) => {
        const buffers = { ...s.buffers, [path]: buf };
        const order = s.order.includes(path) ? s.order : [...s.order, path];
        return { buffers, order, activePath: path };
      });

    const blankBuffer = (kind: PreviewKind, content: string, extra: Partial<EditorBuffer> = {}): EditorBuffer => ({
      path,
      content,
      mtime: 0,
      dirty: false,
      isPreview: false,
      encoding: "",
      totalSize: 0,
      truncated: false,
      kind,
      ...extra,
    });

    let kind = classifyPreview(path);

    // Linux 下的可执行文件习惯上不带扩展名，`classifyPreview` 单看扩展名会把它们
    // 归成 "text"（2026-08-28 用户反馈）——只对没有扩展名的文件额外嗅探文件头前
    // 几个字节，不会给每次打开普通文本文件都加一次往返。
    if (kind === "text" && hasNoExtension(path)) {
      try {
        if (await fsService.peekIsBinary(workspaceId, path)) kind = "executable";
      } catch {
        // 嗅探本身失败（比如权限问题）不影响后续正常走文本读取路径。
      }
    }

    if (kind === "unsupported-binary") {
      // 不尝试读取内容——压缩包这类可能很大，读都不用读，直接给"用系统
      // 默认程序打开"的入口就够了（CodeEditor.tsx/SftpFileViewer.tsx 渲染这个分支）。
      addBuffer(blankBuffer("unsupported-binary", ""));
      return;
    }

    if (kind === "executable") {
      // 解析失败（损坏文件、静态库归档等）不让 Tab 打不开——记下错误信息，
      // BinaryInfoPanel 会退化成"解析失败 + 用系统程序打开"，见该组件注释。
      try {
        const info = await fsService.inspectBinary(workspaceId, path);
        addBuffer(blankBuffer("executable", "", { binaryInfo: info }));
      } catch (e) {
        addBuffer(blankBuffer("executable", "", { binaryInfoError: formatError(e) }));
      }
      return;
    }

    if (kind === "jar") {
      try {
        const info = await fsService.inspectJar(workspaceId, path);
        addBuffer(blankBuffer("jar", "", { jarInfo: info }));
      } catch (e) {
        addBuffer(blankBuffer("jar", "", { jarInfoError: formatError(e) }));
      }
      return;
    }

    if (kind === "image" || kind === "pdf") {
      const base64 = await fsService.readBinaryPreview(workspaceId, path);
      addBuffer(blankBuffer(kind, `data:${binaryMimeType(path)};base64,${base64}`));
      return;
    }

    if (kind === "word") {
      const base64 = await fsService.readBinaryPreview(workspaceId, path);
      const { value: html } = await mammoth.convertToHtml({ arrayBuffer: base64ToArrayBuffer(base64) }, { convertImage: mammothImageConverter });
      // mammoth 转换结果可能带来自文档内容的原始 HTML 片段，和 Markdown 预览
      // （utils/markdown.ts）同样的理由，渲染前必须过一遍 DOMPurify。
      addBuffer(blankBuffer("word", DOMPurify.sanitize(html)));
      return;
    }

    if (kind === "legacy-office") {
      // 转换失败（通常是没装 LibreOffice）不让 Tab 打不开——和 executable/jar 解析
      // 失败一样的退化模式，记下错误信息，UnsupportedBinaryPanel 会展示成
      // "转换失败 + 用系统程序打开"。
      try {
        const base64 = await fsService.convertLegacyOfficeToPdf(workspaceId, path);
        addBuffer(blankBuffer("legacy-office", `data:application/pdf;base64,${base64}`));
      } catch (e) {
        addBuffer(blankBuffer("legacy-office", "", { legacyOfficeError: formatError(e) }));
      }
      return;
    }

    if (kind === "excel") {
      const base64 = await fsService.readBinaryPreview(workspaceId, path);
      const workbook = XLSX.read(base64, { type: "base64" });
      const sheets: ExcelSheet[] = workbook.SheetNames.map((name) => ({
        name,
        rows: XLSX.utils.sheet_to_json<(string | number)[]>(workbook.Sheets[name], { header: 1, defval: "" }),
      }));
      addBuffer(blankBuffer("excel", "", { sheets }));
      return;
    }

    const { text, mtime, encoding, total_size, truncated } = await fsService.readFile(workspaceId, path);
    addBuffer({ path, content: text, mtime, dirty: false, isPreview: false, encoding, totalSize: total_size, truncated, kind: "text" });
  },

  openDiff: async (workspaceId, leftPath, rightPath) => {
    const already = Object.values(get().diffs).find((d) => d.leftPath === leftPath && d.rightPath === rightPath);
    if (already) {
      set({ activePath: already.id });
      return;
    }
    const [left, right] = await Promise.all([
      fsService.readFile(workspaceId, leftPath),
      fsService.readFile(workspaceId, rightPath),
    ]);
    const id = `${DIFF_ID_PREFIX}${Date.now().toString(36)}${Math.random().toString(36).slice(2, 8)}`;
    const diff: DiffBuffer = {
      id,
      leftPath,
      rightPath,
      leftContent: left.text,
      rightContent: right.text,
      language: detectLanguage(rightPath),
    };
    set((s) => ({
      diffs: { ...s.diffs, [id]: diff },
      order: [...s.order, id],
      activePath: id,
    }));
  },

  pin: (path) => {
    set((s) => {
      const buf = s.buffers[path];
      if (!buf) return s;
      return { buffers: { ...s.buffers, [path]: { ...buf, isPreview: false } } };
    });
  },

  setActive: (path) => set({ activePath: path }),

  updateContent: (path, content) => {
    set((s) => {
      const buf = s.buffers[path];
      if (!buf) return s;
      return { buffers: { ...s.buffers, [path]: { ...buf, content, dirty: true } } };
    });
  },

  save: async (workspaceId, path) => {
    const buf = get().buffers[path];
    if (!buf) return;
    // 截断预览不能保存——写回去会把真实的大文件覆盖成预览这么小一份，等于删数据。
    // 图片/PDF/Word/Excel/不支持的二进制都是只读展示，content（或 sheets）不是原始
    // 文件字节，写回去只会把文件损坏成别的东西。这几种情况下正常 UI 都不会露出保存
    // 入口，这里是最后一道防线。
    if (buf.truncated || buf.kind !== "text") {
      throw new Error(buf.kind !== "text" ? "该文件类型不支持编辑保存" : "文件过大，当前只是只读预览，无法保存");
    }

    const outcome = await fsService.writeFile(workspaceId, path, buf.content, buf.mtime);

    if (outcome.type === "Conflict") {
      set({
        conflict: {
          path,
          currentMtime: outcome.current_mtime,
          currentPreview: outcome.current_preview,
        },
      });
      return;
    }

    set((s) => ({
      buffers: { ...s.buffers, [path]: { ...buf, mtime: outcome.mtime, dirty: false } },
    }));
  },

  resolveConflict: async (workspaceId, resolution) => {
    const conflict = get().conflict;
    if (!conflict) return;
    set({ conflict: null });

    if (resolution === "discard") {
      // 放弃本地修改，重新读取远端/磁盘当前内容
      const { text, mtime, encoding, total_size, truncated } = await fsService.readFile(workspaceId, conflict.path);
      set((s) => ({
        buffers: {
          ...s.buffers,
          [conflict.path]: {
            path: conflict.path,
            content: text,
            mtime,
            dirty: false,
            isPreview: false,
            encoding,
            totalSize: total_size,
            truncated,
            kind: "text",
          },
        },
      }));
      return;
    }

    // 仍要覆盖：不带 expectedMtime 强制写入
    const buf = get().buffers[conflict.path];
    if (!buf) return;
    const outcome = await fsService.writeFile(workspaceId, conflict.path, buf.content, null);
    if (outcome.type === "Written") {
      set((s) => ({
        buffers: {
          ...s.buffers,
          [conflict.path]: { ...buf, mtime: outcome.mtime, dirty: false },
        },
      }));
    }
  },

  reopenWithEncoding: async (workspaceId, path, encodingLabel) => {
    const { text, mtime, encoding, total_size, truncated } = await fsService.readFileWithEncoding(workspaceId, path, encodingLabel);
    set((s) => {
      const buf = s.buffers[path];
      if (!buf) return s;
      return {
        buffers: {
          ...s.buffers,
          [path]: { ...buf, content: text, mtime, dirty: false, encoding, totalSize: total_size, truncated },
        },
      };
    });
  },

  saveWithEncoding: async (workspaceId, path, encodingLabel) => {
    const buf = get().buffers[path];
    if (!buf) return;
    if (buf.truncated || buf.kind !== "text") {
      throw new Error(buf.kind !== "text" ? "该文件类型不支持编辑保存" : "文件过大，当前只是只读预览，无法保存");
    }
    const outcome = await fsService.writeFileWithEncoding(workspaceId, path, buf.content, encodingLabel, buf.mtime);
    if (outcome.type === "Conflict") {
      set({ conflict: { path, currentMtime: outcome.current_mtime, currentPreview: outcome.current_preview } });
      return;
    }
    set((s) => ({
      buffers: { ...s.buffers, [path]: { ...buf, mtime: outcome.mtime, dirty: false, encoding: encodingLabel } },
    }));
  },

  close: (path) => {
    set((s) => {
      const buffers = { ...s.buffers };
      const diffs = { ...s.diffs };
      delete buffers[path];
      delete diffs[path];
      const order = s.order.filter((p) => p !== path);
      const activePath = s.activePath === path ? order[order.length - 1] ?? null : s.activePath;
      return { buffers, diffs, order, activePath };
    });
  },

  closeOthers: (path) => {
    set((s) => {
      if (!s.order.includes(path)) return s;
      const buffers = s.buffers[path] ? { [path]: s.buffers[path] } : {};
      const diffs = s.diffs[path] ? { [path]: s.diffs[path] } : {};
      return { buffers, diffs, order: [path], activePath: path };
    });
  },

  closeAll: () => set({ buffers: {}, diffs: {}, order: [], activePath: null }),

  closeToLeft: (path) => {
    set((s) => {
      const idx = s.order.indexOf(path);
      if (idx <= 0) return s;
      const closed = new Set(s.order.slice(0, idx));
      const order = s.order.filter((p) => !closed.has(p));
      const buffers = { ...s.buffers };
      const diffs = { ...s.diffs };
      closed.forEach((p) => {
        delete buffers[p];
        delete diffs[p];
      });
      const activePath = s.activePath && closed.has(s.activePath) ? path : s.activePath;
      return { buffers, diffs, order, activePath };
    });
  },

  closeToRight: (path) => {
    set((s) => {
      const idx = s.order.indexOf(path);
      if (idx < 0 || idx === s.order.length - 1) return s;
      const closed = new Set(s.order.slice(idx + 1));
      const order = s.order.filter((p) => !closed.has(p));
      const buffers = { ...s.buffers };
      const diffs = { ...s.diffs };
      closed.forEach((p) => {
        delete buffers[p];
        delete diffs[p];
      });
      const activePath = s.activePath && closed.has(s.activePath) ? path : s.activePath;
      return { buffers, diffs, order, activePath };
    });
  },

  revealLine: (path, line, highlights) => set({ pendingReveal: { path, line, highlights } }),
  clearReveal: () => set({ pendingReveal: null }),

  reset: () => set({ buffers: {}, diffs: {}, order: [], activePath: null, conflict: null, pendingReveal: null }),
}));
