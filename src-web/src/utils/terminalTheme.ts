import type { ITheme } from "@xterm/xterm";

export type TerminalKind = "ssh" | "local";

/**
 * WindTerm-like high contrast palette. The application chrome may be light,
 * but the terminal canvas deliberately stays dark, matching common WindTerm
 * layouts and keeping ANSI output consistent between app themes.
 */
const windTermTheme: ITheme = {
  background: "#20221F",
  foreground: "#F2F2EE",
  cursorAccent: "#20221F",
  selectionBackground: "rgba(108, 99, 160, 0.42)",
  selectionInactiveBackground: "rgba(108, 99, 160, 0.22)",

  // Normal ANSI colors are saturated enough for prompts and diagnostics.
  black: "#30332F",
  red: "#E44747",
  green: "#48B84E",
  yellow: "#D9B83F",
  blue: "#4B7BEC",
  magenta: "#B45ACB",
  cyan: "#20B8C5",
  white: "#D8D9D4",

  // Bright colors are intentionally distinct, not merely pale variants.
  brightBlack: "#777B75",
  brightRed: "#FF3B62",
  brightGreen: "#8BEA31",
  brightYellow: "#FFE14D",
  brightBlue: "#7297FF",
  brightMagenta: "#C873FF",
  brightCyan: "#43E5F3",
  brightWhite: "#FFFFFF",
};

export function getTerminalTheme(_mode: "dark" | "light", kind: TerminalKind = "local"): ITheme {
  return {
    ...windTermTheme,
    // Orange immediately distinguishes an SSH input caret from a local shell.
    cursor: kind === "ssh" ? "#FFB000" : "#F2F2EE",
  };
}
