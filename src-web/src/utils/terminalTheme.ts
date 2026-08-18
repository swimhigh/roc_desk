import type { ITheme } from "@xterm/xterm";

/** 终端配色（2026-08-18 用户原话："终端界面的颜色还是不够丰富，请参考WINDTERM
 * https://github.com/kingToolbox/WindTerm"）。
 *
 * WindTerm 本身不内置固定配色——它的主题是外挂的独立仓库（`dracula/windterm`、
 * `paradise2017/windterm-solarized-light-theme` 等，在 WindTerm 的"配色方案"菜单里
 * 切换），源码仓库里翻不到硬编码的调色板。这里直接采用 WindTerm 官方仓库列表里
 * 两个真实存在、被广泛使用的主题的原始数值，而不是自己拍脑袋调一套"看起来像"的
 * 颜色：深色用 `dracula/windterm` 仓库 `dracula/scheme.theme` 里 `terminal.ansi*`
 * 字段的原始十六进制值（该文件同时定义了 WindTerm 自己的行为——本地终端光标用白色
 * （`line.caret.local`），远程终端光标用橙色（`line.caret.remote`），这是一个挺有
 * 用的细节：光标颜色不一样，一眼就能分清现在敲的命令是发到本地 Shell 还是发到远程
 * SSH 会话，比统一一个颜色更不容易出现"以为在本地结果敲进了远程"这种事故，这里也
 * 一并采用）；浅色用 Solarized Light 的标准发布数值（WindTerm 官方主题列表里的
 * `windterm-solarized-light-theme` 就是这一套，Solarized 本身数值是公开发布且固定
 * 不变的规范，两边取值一致）。 */
const draculaAnsi = {
  black: "#21222C",
  red: "#FF5555",
  green: "#50FA7B",
  yellow: "#F1FA8C",
  blue: "#BD93F9",
  magenta: "#8C6BC8",
  cyan: "#8BE9FD",
  white: "#e3e3dd",
  brightBlack: "#6272A4",
  brightRed: "#FF6E6E",
  brightGreen: "#69FF94",
  brightYellow: "#FFFFA5",
  brightBlue: "#D6ACFF",
  brightMagenta: "#AE81FF",
  brightCyan: "#A4FFFF",
  brightWhite: "#f8f8f2",
};

const darkThemeBase: ITheme = {
  background: "#282A36",
  foreground: "#F8F8F2",
  cursorAccent: "#282A36",
  selectionBackground: "rgba(103, 80, 135, 0.45)",
  ...draculaAnsi,
};

// Solarized 标准 ANSI 映射（8 个强调色在明暗两套配色里数值相同，只有 black/white
// 两端取自不同的 base 色阶——这是 Solarized 规范本身的定义，不是 WindTerm 特有的）。
const solarized = {
  base03: "#002b36",
  base02: "#073642",
  base01: "#586e75",
  base00: "#657b83",
  base0: "#839496",
  base1: "#93a1a1",
  base2: "#eee8d5",
  base3: "#fdf6e3",
  yellow: "#b58900",
  orange: "#cb4b16",
  red: "#dc322f",
  magenta: "#d33682",
  violet: "#6c71c4",
  blue: "#268bd2",
  cyan: "#2aa198",
  green: "#859900",
};

const lightThemeBase: ITheme = {
  background: solarized.base3,
  foreground: solarized.base00,
  cursorAccent: solarized.base3,
  selectionBackground: "rgba(147, 161, 161, 0.45)",
  black: solarized.base02,
  red: solarized.red,
  green: solarized.green,
  yellow: solarized.yellow,
  blue: solarized.blue,
  magenta: solarized.magenta,
  cyan: solarized.cyan,
  white: solarized.base2,
  brightBlack: solarized.base03,
  brightRed: solarized.orange,
  brightGreen: solarized.base01,
  brightYellow: solarized.base00,
  brightBlue: solarized.base0,
  brightMagenta: solarized.violet,
  brightCyan: solarized.base1,
  brightWhite: solarized.base3,
};

export type TerminalKind = "ssh" | "local";

/** 光标颜色按连接类型分开（见上面注释）：本地=前景色（跟随主题，白/深灰），
 * 远程 SSH=橙色，固定不随明暗主题变化——橙色在两套背景上都足够醒目，且"警示这是
 * 远程会话"这层含义比跟着主题变色更重要。 */
function withCursor(base: ITheme, kind: TerminalKind): ITheme {
  return { ...base, cursor: kind === "ssh" ? "#FFA657" : base.foreground };
}

export function getTerminalTheme(mode: "dark" | "light", kind: TerminalKind = "local"): ITheme {
  return withCursor(mode === "light" ? lightThemeBase : darkThemeBase, kind);
}
