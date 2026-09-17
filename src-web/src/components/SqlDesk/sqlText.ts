/** Locate a statement without splitting quoted literals, identifiers or comments. */
export function statementAt(sql: string, offset: number): string {
  let start = 0, quote = "", blockDepth = 0, line = false, dollar = "";
  for (let i = 0; i < sql.length; i++) {
    const c = sql[i], next = sql[i + 1];
    if (line) { if (c === "\n") line = false; continue; }
    if (blockDepth) {
      if (c === "/" && next === "*") { blockDepth++; i++; }
      else if (c === "*" && next === "/") { blockDepth--; i++; }
      continue;
    }
    if (dollar) { if (sql.startsWith(dollar, i)) { i += dollar.length - 1; dollar = ""; } continue; }
    if (quote) {
      if (c === "\\" && quote !== "]") { i++; continue; }
      if (c === quote) { if (next === quote) i++; else quote = ""; }
      continue;
    }
    if (c === "-" && next === "-") { line = true; i++; }
    else if (c === "/" && next === "*") { blockDepth = 1; i++; }
    else if (c === "'" || c === '"' || c === "`") quote = c;
    else if (c === "[") quote = "]";
    else if (c === "$") {
      const match = sql.slice(i).match(/^\$(?:[a-zA-Z_][\w]*)?\$/);
      if (match) { dollar = match[0]; i += dollar.length - 1; }
    } else if (c === ";") {
      if (offset <= i) return sql.slice(start, i + 1).trim();
      start = i + 1;
    }
  }
  return sql.slice(start).trim();
}
