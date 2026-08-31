/** 给终端里滚动的原始文本（比如 `tail -f` 日志）按日志级别关键字/时间戳/方括号标签
 * 加颜色，参考 WindTerm 这类终端自带的"输出语法高亮"（2026-08-29 用户反馈："tail
 * 或其他命令行出来的文本全是同一颜色的，需要丰富一下和 WINDTERM 一样"）。
 *
 * xterm.js 的 `term.write()` 本身就认 ANSI SGR 转义序列——应用里已经有先例
 * （`TerminalView.tsx` 断线提示那行 `"\x1b[31m...\x1b[0m"`），这里只是把匹配到的
 * 关键字用同样的转义序列包起来再写进去，不是新机制。颜色数值直接抄 `theme.css`
 * 深色主题的 `--danger`/`--warning`/`--accent`/`--text-secondary`（终端画布固定
 * 深色，不跟随应用整体深浅色切换，见 `terminalTheme.ts`），和日志搜索面板/Monaco
 * 里 `.log` 文件的高亮用同一套颜色语义，全应用范围内保持一致。
 *
 * **安全边界**：一段数据只要已经含有 ESC（`\x1b`）字符就整段跳过、原样透传——
 * 这段内容本来就可能已经被着色过（彩色 shell 提示符、`docker`/`kubectl` 这类自带
 * 颜色的输出），或者恰好是一段 ANSI 转义序列被截断在这次数据的边界上，再往里插
 * 自己的转义码有把已有序列插花/嵌套出问题的风险。终端是"绝对不能出错"的地方，
 * 宁可少高亮几行，也不能有内容错乱或丢字节的风险。
 */

const HIGHLIGHT_RE = new RegExp(
  [
    // ISO 风格时间戳（可能带外层方括号）：2026-03-08 19:01:14.559280 / 2026-03-08T19:01:14Z
    String.raw`\[?\d{4}[-/]\d{2}[-/]\d{2}[T ]\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:?\d{2})?\]?`,
    // syslog 风格：Aug 18 10:23:45
    String.raw`\b(?:Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec)\s+\d{1,2}\s+\d{2}:\d{2}:\d{2}\b`,
    // 方括号包着的级别，必须排在通用方括号标签规则之前，否则会被那条规则原样吃掉
    String.raw`\[\s*(?:FATAL|CRITICAL|PANIC)\s*\]`,
    String.raw`\[\s*ERRORS?\s*\]`,
    String.raw`\[\s*WARN(?:ING)?\s*\]`,
    String.raw`\[\s*NOTICE\s*\]`,
    String.raw`\[\s*INFO\s*\]`,
    String.raw`\[\s*(?:DEBUG|TRACE)\s*\]`,
    // 不带方括号、独立出现的级别单词
    String.raw`\b(?:FATAL|CRITICAL|PANIC)\b`,
    String.raw`\bERRORS?\b`,
    String.raw`\bWARN(?:ING)?\b`,
    String.raw`\bNOTICE\b`,
    String.raw`\bINFO\b`,
    String.raw`\b(?:DEBUG|TRACE)\b`,
    // 剩下没被上面规则消费掉的方括号内容：PID/TID、模块名、来源文件:行号 等标签
    String.raw`\[[^\]\r\n]*\]`,
  ].join("|"),
  "g",
);

const SGR = {
  error: "\x1b[38;2;229;83;75m",
  warn: "\x1b[38;2;217;164;65m",
  info: "\x1b[38;2;88;166;255m",
  debug: "\x1b[38;2;154;156;161m",
  tag: "\x1b[38;2;79;140;255m",
  reset: "\x1b[0m",
} as const;

function colorFor(match: string): string {
  if (/^\[?\d{4}[-/]/.test(match) || /^(Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec)\b/.test(match)) {
    return SGR.debug;
  }
  const inner = match.replace(/^\[|\]$/g, "").trim();
  if (/^(FATAL|CRITICAL|PANIC|ERRORS?)$/i.test(inner)) return SGR.error;
  if (/^WARN(ING)?$/i.test(inner)) return SGR.warn;
  if (/^(NOTICE|INFO)$/i.test(inner)) return SGR.info;
  if (/^(DEBUG|TRACE)$/i.test(inner)) return SGR.debug;
  // 剩下的是通用方括号标签（走到这里说明外层一定是 "[...]"，上面几条 exact-match
  // 规则都没中）。
  return SGR.tag;
}

/** 处理一段数据（一次 PTY/SSH 数据事件对应的文本）。只做整段跳过判断 + 一次
 * 全局正则替换，不做跨多次调用的行缓冲——跨调用缓冲能多"抓"到几个被切在数据
 * 边界上的关键字，但会推迟没有换行符的提示符/交互式输出的显示时机（用户敲密码、
 * 用 vim 之类全屏程序时这个延迟是不可接受的）。极小概率漏高亮一个正好被切成两半
 * 的关键字，好过让终端"卡一下才显示"。 */
export function highlightTerminalChunk(text: string): string {
  if (text.indexOf("\x1b") !== -1) return text;
  return text.replace(HIGHLIGHT_RE, (m) => `${colorFor(m)}${m}${SGR.reset}`);
}
