import { marked } from "marked";
import DOMPurify from "dompurify";

marked.setOptions({ breaks: true, gfm: true });

/** Markdown 预览（VS Code 的 Ctrl+Shift+V）：`marked` 解析成 HTML 后必须过一遍
 * `DOMPurify` 再渲染——预览的内容可能来自远程工作区（不完全可信的远端文件），
 * Markdown 本身允许内嵌原始 HTML，不净化直接 `dangerouslySetInnerHTML` 就是一个
 * 现成的 XSS 通道（比如文件里塞一段 `<img onerror=...>`）。*/
export function renderMarkdown(text: string): string {
  const html = marked.parse(text, { async: false }) as string;
  return DOMPurify.sanitize(html);
}
