import { base64ToArrayBuffer } from "./previewFile";

/**
 * HTML 预览用 `<iframe srcDoc>` 展示（CodeEditor.tsx）：`srcdoc` 文档里的相对路径
 * 资源引用（`<link href>`/`<script src>`/`<img src>`/CSS 里的 `url(...)`）默认相对
 * 于宿主页面（roc_desk 自己的地址）解析，不会去用户打开的那个远程/本地文件所在的
 * 目录找，结果是引用了外部 CSS/JS 的页面预览出来一片空白（2026-08-28 用户反馈）。
 *
 * 这里在设置 `srcDoc` 之前，把 HTML 引用的相对路径资源都通过和正文一样的读取通道
 * （`readBinary`，工作区/SFTP 各自绑定）抓下来，转成 `data:` URL 内联进去，让页面
 * 变成"自包含"的再交给 iframe。只处理一层 CSS `url(...)`（链接的 CSS 文件内部引用
 * 的字体/背景图），不递归处理 `@import` 或 JS 运行时动态请求的资源——覆盖"一个
 * HTML + 同目录几个 CSS/JS/图片"这种最常见的静态页面场景，更复杂的单页应用
 * （嵌套 import、运行时 fetch 数据）本来就不是"预览"这个功能该覆盖的范围。
 */
export async function inlineHtmlResources(
  html: string,
  baseDir: string,
  readBinary: (path: string) => Promise<string>,
): Promise<string> {
  const doc = new DOMParser().parseFromString(html, "text/html");
  const budget = { remaining: 40 };
  const tasks: Promise<void>[] = [];

  const inlineAttr = (el: Element, attr: string, isCss: boolean) => {
    const ref = el.getAttribute(attr);
    if (!ref || shouldSkipRef(ref) || budget.remaining <= 0) return;
    budget.remaining--;
    const resolved = resolvePath(baseDir, ref);
    tasks.push(
      readBinary(resolved)
        .then(async (base64) => {
          if (isCss) {
            const cssText = new TextDecoder().decode(base64ToArrayBuffer(base64));
            const inlinedCss = await inlineCssUrls(cssText, dirnameOf(resolved), readBinary, budget);
            el.setAttribute(attr, `data:text/css;base64,${bytesToBase64(new TextEncoder().encode(inlinedCss))}`);
          } else {
            el.setAttribute(attr, `data:${guessMimeType(resolved)};base64,${base64}`);
          }
        })
        // 单个资源拿不到（权限、文件不存在……）就保留原样引用——不会比内联前更差
        // （本来就是加载不出来），不能因为一个资源失败就让整页内联全部失败。
        .catch(() => {}),
    );
  };

  doc.querySelectorAll('link[rel="stylesheet"][href]').forEach((el) => inlineAttr(el, "href", true));
  doc.querySelectorAll("script[src]").forEach((el) => inlineAttr(el, "src", false));
  doc.querySelectorAll("img[src]").forEach((el) => inlineAttr(el, "src", false));

  await Promise.all(tasks);
  return `<!DOCTYPE html>\n${doc.documentElement.outerHTML}`;
}

function shouldSkipRef(ref: string): boolean {
  return /^(data:|https?:|\/\/|mailto:|javascript:|#)/i.test(ref);
}

/** 根相对路径（以 `/` 开头）把 HTML 所在目录当成"站点根目录"处理——常见的简单静态
 * 文件服务器部署方式是把一整个目录原样映射到 web 根，这是启发式，不保证总是对，
 * 但是没有更好的信息来源（roc_desk 不知道远程服务器实际的路由规则）。 */
function resolvePath(baseDir: string, ref: string): string {
  const base = baseDir.replace(/\\/g, "/").replace(/\/$/, "");
  const target = ref.startsWith("/") ? `${base}${ref}` : `${base}/${ref}`;
  const isAbsolute = target.startsWith("/");
  const stack: string[] = [];
  for (const part of target.split("/")) {
    if (part === "" || part === ".") continue;
    if (part === "..") stack.pop();
    else stack.push(part);
  }
  return (isAbsolute ? "/" : "") + stack.join("/");
}

function dirnameOf(path: string): string {
  const idx = path.lastIndexOf("/");
  return idx === -1 ? path : path.slice(0, idx);
}

async function inlineCssUrls(
  cssText: string,
  cssDir: string,
  readBinary: (path: string) => Promise<string>,
  budget: { remaining: number },
): Promise<string> {
  const urlRe = /url\(\s*(['"]?)([^'")]+)\1\s*\)/g;
  const refs = new Set<string>();
  let match: RegExpExecArray | null;
  while ((match = urlRe.exec(cssText))) {
    const ref = match[2];
    if (ref && !shouldSkipRef(ref)) refs.add(ref);
  }

  let result = cssText;
  for (const ref of refs) {
    if (budget.remaining <= 0) break;
    budget.remaining--;
    const resolved = resolvePath(cssDir, ref);
    try {
      const base64 = await readBinary(resolved);
      const dataUrl = `data:${guessMimeType(resolved)};base64,${base64}`;
      // 字符串整体替换（不是正则），避免 ref 里的正则特殊字符（文件名常见的
      // "." "+" 之类）被当成正则语法处理。
      result = result.split(ref).join(dataUrl);
    } catch {
      // 拿不到就保留原样。
    }
  }
  return result;
}

function bytesToBase64(bytes: Uint8Array): string {
  let binary = "";
  for (let i = 0; i < bytes.length; i++) binary += String.fromCharCode(bytes[i]);
  return btoa(binary);
}

const MIME_BY_EXTENSION: Record<string, string> = {
  css: "text/css",
  js: "application/javascript",
  mjs: "application/javascript",
  json: "application/json",
  png: "image/png",
  jpg: "image/jpeg",
  jpeg: "image/jpeg",
  gif: "image/gif",
  svg: "image/svg+xml",
  webp: "image/webp",
  ico: "image/x-icon",
  woff: "font/woff",
  woff2: "font/woff2",
  ttf: "font/ttf",
  otf: "font/otf",
  eot: "application/vnd.ms-fontobject",
};

function guessMimeType(path: string): string {
  const ext = path.includes(".") ? path.split(".").pop()!.toLowerCase() : "";
  return MIME_BY_EXTENSION[ext] ?? "application/octet-stream";
}
