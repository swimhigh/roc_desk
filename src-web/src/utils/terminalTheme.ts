import type { ITheme } from "@xterm/xterm";

export type TerminalKind = "ssh" | "local" | "agent";

/**
 * 真实的 Dracula 主题官方 ANSI 数值（WindTerm 官方主题列表里 `dracula/windterm`
 * 仓库 `dracula/scheme.theme` 用的就是这一份，draculatheme.com 的规范色号也是
 * 同一套）——2026-08-27 用户要求"字体配色多参考 WindTerm"：之前这里是自己按
 * "看着顺眼"调的一套近似色（背景 #20221F、前景 #F2F2EE 等，不是任何真实主题的
 * 数值），拿真实 Dracula 数值替换掉，不再是"风格上像"而是就是这一份主题。
 * 应用主界面可能是浅色主题，但终端画布本身固定深色，不随应用亮/暗切换——这是
 * WindTerm 等专业终端工具常见的做法，也让 ANSI 输出颜色在两种应用主题下保持一致。
 */
const draculaTheme: ITheme = {
  background: "#282A36",
  foreground: "#F8F8F2",
  cursorAccent: "#282A36",
  selectionBackground: "rgba(68, 71, 90, 0.75)", // Dracula Selection #44475A
  selectionInactiveBackground: "rgba(68, 71, 90, 0.4)",

  black: "#21222C",
  red: "#FF5555",
  green: "#50FA7B",
  yellow: "#F1FA8C",
  blue: "#BD93F9", // Dracula 规范里 ANSI blue 槽位就是这支紫色，不是常见的蓝色
  magenta: "#FF79C6",
  cyan: "#8BE9FD",
  white: "#F8F8F2",

  // Bright 系列同样是 Dracula 官方数值，不是苍白版的 normal 色。这些精确的十六
  // 进制值原样复用为 theme.css 里的 --shell-* 令牌，让 AI 工具面板展示的命令
  // 和真正的终端用同一套颜色——重新调这份调色板时两处必须一起改。
  brightBlack: "#6272A4",
  brightRed: "#FF6E6E",
  brightGreen: "#69FF94",
  brightYellow: "#FFFFA5",
  brightBlue: "#D6ACFF",
  brightMagenta: "#FF92DF",
  brightCyan: "#A4FFFF",
  brightWhite: "#FFFFFF",
};

export function getTerminalTheme(_mode: "dark" | "light", kind: TerminalKind = "local"): ITheme {
  return {
    ...draculaTheme,
    // 本地/远程光标用不同颜色一眼分清当前敲的命令发去哪——本地跟随前景色，
    // 远程固定橙色（同一个真实细节取自 WindTerm 主题文件里的
    // line.caret.local/line.caret.remote，不是随手挑的颜色）。SSH 和 Agent
    // 都是"远程"，用同一个颜色。
    cursor: kind === "ssh" || kind === "agent" ? "#FFB000" : "#F8F8F2",
  };
}
