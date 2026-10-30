/** Turn a block of grid cells into text for the clipboard. */
import type { Cell } from "./api";

export type CopyFormat = "tsv" | "csv" | "json" | "markdown" | "insert";

const text = (v: Cell | undefined) => (v === null || v === undefined ? "" : typeof v === "object" ? JSON.stringify(v) : String(v));

function csvField(s: string) {
  return /[",\n\r]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
}

function sqlLiteral(v: Cell | undefined) {
  if (v === null || v === undefined) return "NULL";
  if (typeof v === "number") return String(v);
  if (typeof v === "boolean") return v ? "TRUE" : "FALSE";
  return `'${text(v).replace(/'/g, "''")}'`;
}

const ident = (s: string) => (/^[a-z_][a-z0-9_]*$/.test(s) ? s : `"${s.replace(/"/g, '""')}"`);

export function formatCells(format: CopyFormat, headers: string[], rows: (Cell | undefined)[][], table?: string): string {
  switch (format) {
    case "tsv":
      return rows.map((r) => r.map((v) => text(v).replace(/[\t\n]/g, " ")).join("\t")).join("\n");
    case "csv":
      return [headers, ...rows.map((r) => r.map(text))].map((r) => r.map(csvField).join(",")).join("\n");
    case "json":
      return JSON.stringify(
        rows.map((r) => Object.fromEntries(headers.map((h, i) => [h, r[i] === undefined ? null : r[i]]))),
        null,
        2,
      );
    case "markdown": {
      const esc = (s: string) => s.replace(/\|/g, "\\|").replace(/\n/g, " ");
      return [
        `| ${headers.map(esc).join(" | ")} |`,
        `| ${headers.map(() => "---").join(" | ")} |`,
        ...rows.map((r) => `| ${r.map((v) => esc(text(v))).join(" | ")} |`),
      ].join("\n");
    }
    case "insert": {
      const t = table ? table.split(".").map(ident).join(".") : "my_table";
      const cols = headers.map(ident).join(", ");
      return rows.map((r) => `INSERT INTO ${t} (${cols}) VALUES (${r.map(sqlLiteral).join(", ")});`).join("\n");
    }
  }
}

/** Parse clipboard text (TSV from spreadsheets, or a single value) into a grid. */
export function parseClipboard(s: string): string[][] {
  const lines = s.replace(/\r\n?/g, "\n").replace(/\n$/, "").split("\n");
  return lines.map((l) => l.split("\t"));
}
