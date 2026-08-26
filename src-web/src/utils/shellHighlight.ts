/**
 * 轻量 shell 语法高亮：不追求完整的 shell 语法解析，只把命令行里最常见的几类
 * token（命令名、flag、字符串、变量、管道/重定向、注释）上色，让 AI 工具面板
 * 里的命令一眼就能和普通说明文字区分开。颜色取自 utils/terminalTheme.ts 的
 * WindTerm 风格调色板（见 theme.css 的 --shell-* 变量），和真正的终端保持一致。
 */

const TOKEN_RE = /("(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*'|\$\{[^}]+\}|\$\w+|&&|\|\||>>|<<|[|<>;])|(\s+)|(\S+)/g;

function escapeHtml(text: string): string {
  return text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

function span(cls: string, text: string): string {
  return `<span class="${cls}">${escapeHtml(text)}</span>`;
}

function findCommentIndex(line: string): number {
  let inSingle = false;
  let inDouble = false;
  for (let i = 0; i < line.length; i++) {
    const c = line[i];
    if (c === "'" && !inDouble) inSingle = !inSingle;
    else if (c === '"' && !inSingle) inDouble = !inDouble;
    else if (c === "#" && !inSingle && !inDouble && (i === 0 || /\s/.test(line[i - 1]))) return i;
  }
  return -1;
}

function highlightLine(line: string): string {
  const commentIdx = findCommentIndex(line);
  const codePart = commentIdx >= 0 ? line.slice(0, commentIdx) : line;
  const commentPart = commentIdx >= 0 ? line.slice(commentIdx) : "";

  let html = "";
  let atLineStart = true;
  let expectCommand = true; // 管道/分号之后的下一个词依然是"命令名"
  let match: RegExpExecArray | null;
  TOKEN_RE.lastIndex = 0;
  while ((match = TOKEN_RE.exec(codePart))) {
    const [, special, space, word] = match;
    if (space) {
      html += escapeHtml(space);
      continue;
    }
    if (special) {
      if (special[0] === '"' || special[0] === "'") html += span("shell-string", special);
      else if (special[0] === "$") html += span("shell-var", special);
      else {
        html += span("shell-op", special);
        if (special === "|" || special === "&&" || special === "||" || special === ";") expectCommand = true;
      }
      atLineStart = false;
      continue;
    }
    if (word) {
      if (atLineStart || expectCommand) html += span("shell-cmd", word);
      else if (word.startsWith("-")) html += span("shell-flag", word);
      else html += span("shell-arg", word);
      atLineStart = false;
      expectCommand = false;
    }
  }
  if (commentPart) html += span("shell-comment", commentPart);
  return html;
}

export function highlightShellCommand(raw: string): string {
  return raw.split("\n").map(highlightLine).join("\n");
}
