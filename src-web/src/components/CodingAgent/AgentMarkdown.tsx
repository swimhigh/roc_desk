import React, { useMemo } from "react";
import { renderMarkdown } from "../../utils/markdown";

interface AgentMarkdownProps {
  content: string;
  onOpenFile?: (path: string, line?: number) => void;
}

const FILE_REF = /^(.*?\.(?:rs|ts|tsx|js|jsx|json|md|css|scss|html|toml|yaml|yml|sql|py|go|java|kt|sh|ps1|xml))(?:[:#]L?(\d+))?$/i;

function parseFileRef(value: string): { path: string; line?: number } | null {
  const cleaned = value.trim().replace(/^file:\/\//, "").replace(/^['"`]|['"`]$/g, "");
  const match = cleaned.match(FILE_REF);
  if (!match || /^https?:\/\//i.test(cleaned)) return null;
  return { path: match[1].replace(/\\/g, "/"), line: match[2] ? Number(match[2]) : undefined };
}

function decorateFileReferences(html: string): string {
  const document = new DOMParser().parseFromString(`<div id="agent-md-root">${html}</div>`, "text/html");
  const root = document.getElementById("agent-md-root");
  if (!root) return html;

  root.querySelectorAll("a, code").forEach((element) => {
    const raw = element instanceof HTMLAnchorElement ? element.getAttribute("href") || element.textContent || "" : element.textContent || "";
    if (parseFileRef(raw) && !element.closest("pre")) element.classList.add("agent-file-ref");
  });
  return root.innerHTML;
}

/** 安全 Markdown 渲染，并把代码样式的文件路径/相对链接变成可打开的引用。 */
export const AgentMarkdown: React.FC<AgentMarkdownProps> = ({ content, onOpenFile }) => {
  const html = useMemo(() => decorateFileReferences(renderMarkdown(content)), [content]);

  const handleClick = (event: React.MouseEvent<HTMLDivElement>) => {
    if (!onOpenFile) return;
    const element = (event.target as HTMLElement).closest("a, code");
    if (!element || element.closest("pre")) return;
    const raw = element instanceof HTMLAnchorElement ? element.getAttribute("href") || element.textContent || "" : element.textContent || "";
    const ref = parseFileRef(raw);
    if (!ref) return;
    event.preventDefault();
    onOpenFile(ref.path, ref.line);
  };

  return <div className="agent-markdown" onClick={handleClick} dangerouslySetInnerHTML={{ __html: html }} />;
};
