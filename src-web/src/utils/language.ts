/** 文件名 → Monaco 语言 id，覆盖 DESIGN.md §3.3.1 定义的常见文本类型。*/
const EXTENSION_LANGUAGE: Record<string, string> = {
  json: "json",
  jsonc: "jsonc",
  md: "markdown",
  markdown: "markdown",
  log: "log", // 自定义 Monarch 语言（monacoSetup.ts），识别时间戳/级别/标签等通用日志元素
  txt: "plaintext",
  yml: "yaml",
  yaml: "yaml",
  toml: "toml",
  ini: "ini",
  conf: "ini",
  cfg: "ini",
  xml: "xml",
  html: "html",
  htm: "html",
  css: "css",
  scss: "scss",
  less: "less",
  js: "javascript",
  jsx: "javascript",
  ts: "typescript",
  tsx: "typescript",
  py: "python",
  sh: "shell",
  bash: "shell",
  env: "ini",
  csv: "plaintext",
  sql: "sql",
  lua: "lua",
  rs: "rust",
  go: "go",
  java: "java",
  c: "c",
  h: "c",
  cpp: "cpp",
  hpp: "cpp",
  rb: "ruby",
  php: "php",
  dockerfile: "dockerfile",
  mk: "makefile",
  mak: "makefile",
};

/** 没有扩展名、靠固定文件名识别的类型（不区分大小写）——Dockerfile/Makefile 都是这类。*/
const FILENAME_LANGUAGE: Record<string, string> = {
  dockerfile: "dockerfile",
  makefile: "makefile",
  gnumakefile: "makefile",
};

export function detectLanguage(path: string): string {
  const fileName = path.split(/[\\/]/).pop() ?? path;
  const byName = FILENAME_LANGUAGE[fileName.toLowerCase()];
  if (byName) return byName;
  const ext = fileName.includes(".") ? fileName.split(".").pop()!.toLowerCase() : "";
  return EXTENSION_LANGUAGE[ext] ?? "plaintext";
}
