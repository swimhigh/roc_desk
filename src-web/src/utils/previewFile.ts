import mammoth from "mammoth";

/** 编辑器/SFTP 查看器按扩展名分流的文件类型（2026-08-28 用户反馈：图片/PDF/Word/
 * Excel 之前都被当文本打开、显示一堆乱码）。旧版 `.doc`/`.xls`/`.ppt`/`.pptx` 是
 * `mammoth`/`xlsx` 解不了的格式，单独归进 "legacy-office"，走 LibreOffice 转 PDF
 * 预览（见下面 `LEGACY_OFFICE_EXTENSIONS` 的注释）。真正没有任何预览手段的（压缩包/
 * 字体等，参考 fsops::SEARCH_BINARY_EXTENSIONS 的同款列表）才归进 unsupported-binary，
 * 只给"用系统默认程序打开"，不尝试显示内容。
 *
 * "executable"（EXE/DLL/SO 等，2026-08-28 需求）单独一档，不归进 unsupported-binary——
 * 这类文件后端能解析出基本信息 + 依赖库列表（`fsops::binary_info`），值得比"只能用
 * 系统程序打开"更有用的展示。"jar" 同理，只是后端解析的是 ZIP + manifest 结构
 * （`fsops::jar_info`），不是 PE/ELF 头。
 *
 * 注意：`classifyPreview` 只看扩展名，判断不了 Linux 下习惯不带扩展名的可执行文件
 * ——那种情况下这里会先返回 "text"，真正的"是不是可执行文件"由调用方
 * （`editorStore.openPreview`/`SftpFileViewer.tsx`）对没有扩展名的文件额外嗅探文件头
 * 决定要不要把已经算出来的 "text" 分类改写成 "executable"，见 `hasNoExtension`。*/
export type PreviewKind = "text" | "image" | "pdf" | "word" | "excel" | "executable" | "jar" | "legacy-office" | "unsupported-binary";

const IMAGE_MIME: Record<string, string> = {
  png: "image/png",
  jpg: "image/jpeg",
  jpeg: "image/jpeg",
  gif: "image/gif",
  webp: "image/webp",
  bmp: "image/bmp",
  ico: "image/x-icon",
  svg: "image/svg+xml",
};

const WORD_EXTENSIONS = new Set(["docx"]);
const EXCEL_EXTENSIONS = new Set(["xlsx"]);
const PDF_EXTENSIONS = new Set(["pdf"]);

/** 后端 `fsops::binary_info` 能解析基本信息 + 依赖库列表的可执行文件/动态库扩展名
 * （PE/ELF/Mach-O，2026-08-28 需求）。*/
const EXECUTABLE_EXTENSIONS = new Set(["exe", "dll", "so", "dylib"]);

/** 后端 `fsops::jar_info` 能解析 manifest/依赖库/内部条目的 JAR 包扩展名
 * （2026-08-28 需求）。*/
const JAR_EXTENSIONS = new Set(["jar"]);

/** 旧版二进制 Office 文档——不像 .docx/.xlsx 是 zip+XML，mammoth/xlsx.js 这类纯 JS
 * 库解不了 .doc/.xls 的 OLE2 二进制格式，.ppt/.pptx 也没有对应的纯 JS 幻灯片渲染库。
 * 后端 `fsops::office_convert` 用本机安装的 LibreOffice 临时转成 PDF，复用已有的
 * `PdfPreview` 展示（2026-08-28 用户建议）；没装 LibreOffice 时转换会失败，前端退化
 * 成"转换失败 + 用系统程序打开"，和 executable/jar 解析失败的退化方式一致。*/
const LEGACY_OFFICE_EXTENSIONS = new Set(["doc", "xls", "ppt", "pptx"]);

/** 能识别是二进制、但预览不支持的扩展名——不能落到 Monaco 里当文本显示乱码，
 * 只给"用系统默认程序打开"这一条路。*/
const UNSUPPORTED_BINARY_EXTENSIONS = new Set([
  "zip", "tar", "gz", "7z", "rar",
  "pdb",
  "woff", "woff2", "ttf", "eot",
  "db", "sqlite", "class", "wasm",
  "mp4", "mp3", "wav", "avi", "mov",
]);

function extensionOf(path: string): string {
  const fileName = path.split(/[\\/]/).pop() ?? path;
  return fileName.includes(".") ? fileName.split(".").pop()!.toLowerCase() : "";
}

/** Linux 下的可执行文件习惯上不带扩展名（`/usr/bin/*` 里绝大多数都是这样）——
 * `classifyPreview` 单看扩展名会把它们当成普通文本，调用方对这种文件需要额外嗅探
 * 文件头前几个字节才能确认是不是 ELF（2026-08-28 用户反馈）。*/
export function hasNoExtension(path: string): boolean {
  return extensionOf(path) === "";
}

export function classifyPreview(path: string): PreviewKind {
  const ext = extensionOf(path);
  if (ext in IMAGE_MIME) return "image";
  if (PDF_EXTENSIONS.has(ext)) return "pdf";
  if (WORD_EXTENSIONS.has(ext)) return "word";
  if (EXCEL_EXTENSIONS.has(ext)) return "excel";
  if (EXECUTABLE_EXTENSIONS.has(ext)) return "executable";
  if (JAR_EXTENSIONS.has(ext)) return "jar";
  if (LEGACY_OFFICE_EXTENSIONS.has(ext)) return "legacy-office";
  if (UNSUPPORTED_BINARY_EXTENSIONS.has(ext)) return "unsupported-binary";
  return "text";
}

export function isImageFile(path: string): boolean {
  return classifyPreview(path) === "image";
}

export function binaryMimeType(path: string): string {
  const ext = extensionOf(path);
  if (ext in IMAGE_MIME) return IMAGE_MIME[ext];
  if (PDF_EXTENSIONS.has(ext)) return "application/pdf";
  return "application/octet-stream";
}

/** Excel 预览用：每个 sheet 一张表，`rows` 是 `xlsx` 的 `sheet_to_json({ header: 1 })`
 * 结果——数组的数组，第一层是行，第二层是单元格，不强制假设第一行是表头
 * （很多导出的 Excel 第一行不是表头，交给用户自己看）。 */
export interface ExcelSheet {
  name: string;
  rows: (string | number)[][];
}

/** mammoth 默认丢弃 docx 里嵌入的图片，只留文字——2026-08-28 用户反馈"调用流程"
 * 这类图文混排的文档预览出来图片位置是空的（浏览器找不到图片资源的默认占位图标）。
 * 这是 mammoth 文档里给的标准写法：把每张图读成 base64 内联进 `<img src="data:...">`，
 * 不依赖任何外部资源，图文混排的文档就能正常显示了。*/
export const mammothImageConverter = mammoth.images.imgElement((image) =>
  image.read("base64").then((imageBuffer) => ({ src: `data:${image.contentType};base64,${imageBuffer}` })),
);

export function base64ToArrayBuffer(base64: string): ArrayBuffer {
  const binary = atob(base64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  return bytes.buffer;
}
