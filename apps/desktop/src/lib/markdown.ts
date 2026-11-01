// A small, safe Markdown renderer for notebook text cells: everything is
// HTML-escaped first, then a known subset is turned into tags (headings,
// emphasis, code, lists, quotes, links, rules, tables). No raw HTML passes through.
import type { Cell, QueryResult } from "./api";

const esc = (s: string) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");

function inline(s: string): string {
  let t = esc(s);
  const codes: string[] = [];
  t = t.replace(/`([^`]+)`/g, (_, c) => `\u0000${codes.push(c) - 1}\u0000`);
  t = t.replace(/\[([^\]]+)\]\((https?:\/\/[^\s)]+)\)/g, '<a href="$2" target="_blank" rel="noreferrer">$1</a>');
  t = t.replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>").replace(/__([^_]+)__/g, "<strong>$1</strong>");
  t = t.replace(/(^|[^*])\*([^*]+)\*/g, "$1<em>$2</em>").replace(/(^|\W)_([^_]+)_(?=\W|$)/g, "$1<em>$2</em>");
  t = t.replace(/~~([^~]+)~~/g, "<del>$1</del>");
  return t.replace(/\u0000(\d+)\u0000/g, (_, i) => `<code>${codes[Number(i)]}</code>`);
}

export function renderMarkdown(src: string): string {
  const lines = src.replace(/\r\n/g, "\n").split("\n");
  const out: string[] = [];
  let i = 0;
  while (i < lines.length) {
    const l = lines[i];
    if (/^```/.test(l)) {
      const body: string[] = [];
      i++;
      while (i < lines.length && !/^```/.test(lines[i])) body.push(lines[i++]);
      i++;
      out.push(`<pre><code>${esc(body.join("\n"))}</code></pre>`);
      continue;
    }
    const h = /^(#{1,4})\s+(.*)$/.exec(l);
    if (h) {
      out.push(`<h${h[1].length}>${inline(h[2])}</h${h[1].length}>`);
      i++;
      continue;
    }
    if (/^\s*(?:-\s*){3,}$|^\s*(?:\*\s*){3,}$|^\s*(?:_\s*){3,}$/.test(l)) {
      out.push("<hr/>");
      i++;
      continue;
    }
    if (/^\s*[-*+]\s+/.test(l) || /^\s*\d+[.)]\s+/.test(l)) {
      const ordered = /^\s*\d+[.)]\s+/.test(l);
      const items: string[] = [];
      while (i < lines.length && (ordered ? /^\s*\d+[.)]\s+/ : /^\s*[-*+]\s+/).test(lines[i])) {
        const item = lines[i].replace(ordered ? /^\s*\d+[.)]\s+/ : /^\s*[-*+]\s+/, "");
        const task = /^\[([ xX])\]\s+(.*)$/.exec(item);
        items.push(task ? `<li class="task">${task[1] === " " ? "☐" : "☑"} ${inline(task[2])}</li>` : `<li>${inline(item)}</li>`);
        i++;
      }
      out.push(ordered ? `<ol>${items.join("")}</ol>` : `<ul>${items.join("")}</ul>`);
      continue;
    }
    if (/^>\s?/.test(l)) {
      const body: string[] = [];
      while (i < lines.length && /^>\s?/.test(lines[i])) body.push(lines[i++].replace(/^>\s?/, ""));
      out.push(`<blockquote>${inline(body.join(" "))}</blockquote>`);
      continue;
    }
    if (/^\|.*\|\s*$/.test(l) && /^\|?\s*:?-{2,}/.test(lines[i + 1] ?? "")) {
      const row = (r: string) => r.trim().replace(/^\||\|$/g, "").split("|").map((c) => c.trim());
      const head = row(l);
      i += 2;
      const body: string[][] = [];
      while (i < lines.length && /^\|.*\|\s*$/.test(lines[i])) body.push(row(lines[i++]));
      out.push(`<table><thead><tr>${head.map((c) => `<th>${inline(c)}</th>`).join("")}</tr></thead><tbody>${body.map((r) => `<tr>${r.map((c) => `<td>${inline(c)}</td>`).join("")}</tr>`).join("")}</tbody></table>`);
      continue;
    }
    if (!l.trim()) {
      i++;
      continue;
    }
    const para: string[] = [];
    while (i < lines.length && lines[i].trim() && !/^(#{1,4}\s|```|>|\s*[-*+]\s|\s*\d+[.)]\s|\|)/.test(lines[i])) para.push(lines[i++]);
    if (!para.length) para.push(lines[i++]);
    out.push(`<p>${inline(para.join(" "))}</p>`);
  }
  return out.join("\n");
}

const cellText = (v: Cell) => (v === null || v === undefined ? "" : typeof v === "object" ? JSON.stringify(v) : String(v)).replace(/\|/g, "\\|").replace(/\n/g, " ");

/** A result as a Markdown table (first `max` rows). */
export function resultToMarkdown(r: QueryResult, max = 50): string {
  if (!r.columns.length) return `_${r.rows_affected} row${r.rows_affected === 1 ? "" : "s"} affected_`;
  const head = `| ${r.columns.map((c) => c.name.replace(/\|/g, "\\|")).join(" | ")} |`;
  const sep = `| ${r.columns.map(() => "---").join(" | ")} |`;
  const rows = r.rows.slice(0, max).map((row) => `| ${row.map(cellText).join(" | ")} |`);
  const more = r.rows.length > max || r.truncated ? `\n\n_${r.rows.length > max ? `first ${max} of ${r.rows.length.toLocaleString()} rows` : "more rows not shown"}_` : "";
  return [head, sep, ...rows].join("\n") + more;
}
