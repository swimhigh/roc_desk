import type { ITheme } from "@xterm/xterm";

/** 终端配色（参考 WindTerm/One Dark 系配色，比 xterm.js 默认的纯黑+基础 8 色更有层次）。
 * 浅色 UI 模式下终端本身仍保持深色——这是大多数终端类应用（含 VS Code 默认）的惯例，
 * 长时间盯着的等宽文本区域用深色背景对比度更舒适，不需要跟随整体 UI 主题变浅。 */
const darkTheme: ITheme = {
  background: "#0d1117",
  foreground: "#c9d1d9",
  cursor: "#4f8cff",
  cursorAccent: "#0d1117",
  selectionBackground: "rgba(79, 140, 255, 0.35)",
  black: "#484f58",
  red: "#ff7b72",
  green: "#3fb950",
  yellow: "#d29922",
  blue: "#4f8cff",
  magenta: "#bc8cff",
  cyan: "#39c5cf",
  white: "#c9d1d9",
  brightBlack: "#6e7681",
  brightRed: "#ffa198",
  brightGreen: "#56d364",
  brightYellow: "#e3b341",
  brightBlue: "#79c0ff",
  brightMagenta: "#d2a8ff",
  brightCyan: "#56d4dd",
  brightWhite: "#f0f6fc",
};

const lightTheme: ITheme = {
  background: "#1e1e1e",
  foreground: "#d4d4d4",
  cursor: "#5a9bf6",
  cursorAccent: "#1e1e1e",
  selectionBackground: "rgba(90, 155, 246, 0.35)",
  black: "#4d4d4d",
  red: "#f2777a",
  green: "#8bc34a",
  yellow: "#e0b04d",
  blue: "#5a9bf6",
  magenta: "#b48ead",
  cyan: "#5fb3b3",
  white: "#d4d4d4",
  brightBlack: "#666666",
  brightRed: "#f78c8c",
  brightGreen: "#a3d977",
  brightYellow: "#f0c674",
  brightBlue: "#82aaff",
  brightMagenta: "#c792ea",
  brightCyan: "#7fdbdb",
  brightWhite: "#ffffff",
};

export function getTerminalTheme(mode: "dark" | "light"): ITheme {
  return mode === "light" ? lightTheme : darkTheme;
}
