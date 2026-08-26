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
  black: "#2A2D28",
  red: "#E5484D",
  green: "#3FC97A",
  yellow: "#E0B23D",
  blue: "#4C93F8",
  magenta: "#C15FE0",
  cyan: "#22C3D6",
  white: "#D9DAD5",

  // Bright colors are intentionally distinct, not merely pale variants. These
  // exact hex values are reused verbatim as the --shell-* tokens in theme.css
  // so command syntax highlighting in the AI panel visually matches the
  // terminal — keep both in sync if you retune this palette.
  brightBlack: "#7A7E76",
  brightRed: "#FF5C72",
  brightGreen: "#7CF23A",
  brightYellow: "#FFDD57",
  brightBlue: "#74A6FF",
  brightMagenta: "#D98CFF",
  brightCyan: "#4FF0FF",
  brightWhite: "#FFFFFF",
};

export function getTerminalTheme(_mode: "dark" | "light", kind: TerminalKind = "local"): ITheme {
  return {
    ...windTermTheme,
    // Orange immediately distinguishes an SSH input caret from a local shell.
    cursor: kind === "ssh" ? "#FFB000" : "#F2F2EE",
  };
}
