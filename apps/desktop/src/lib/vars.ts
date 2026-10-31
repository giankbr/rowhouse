// Query variables: `{{name}}` (pasted as written, works anywhere — also inside
// strings) and `:name` (outside strings/comments; becomes a quoted literal
// unless the value is a number, NULL, TRUE or FALSE).

export interface VarRef {
  name: string;
  /** `{{name}}` is raw text; `:name` is a literal */
  raw: boolean;
}

interface Hit {
  start: number;
  end: number;
  name: string;
  raw: boolean;
}

const IDENT = /[A-Za-z_][A-Za-z0-9_]*/y;

function scan(sql: string, colon: boolean): Hit[] {
  const hits: Hit[] = [];
  // {{name}} anywhere
  for (const m of sql.matchAll(/\{\{\s*([A-Za-z_][A-Za-z0-9_]*)\s*\}\}/g)) {
    hits.push({ start: m.index!, end: m.index! + m[0].length, name: m[1], raw: true });
  }
  if (!colon) return hits;
  let i = 0;
  const n = sql.length;
  while (i < n) {
    const c = sql[i];
    const next = sql[i + 1];
    if (c === "-" && next === "-") {
      const e = sql.indexOf("\n", i);
      i = e < 0 ? n : e;
    } else if (c === "/" && next === "*") {
      const e = sql.indexOf("*/", i + 2);
      i = e < 0 ? n : e + 2;
    } else if (c === "'" || c === '"' || c === "`") {
      let j = i + 1;
      while (j < n) {
        if (sql[j] === "\\" && c === "'") j += 2;
        else if (sql[j] === c) {
          if (sql[j + 1] === c) j += 2;
          else break;
        } else j++;
      }
      i = j + 1;
    } else if (c === "$") {
      // Postgres dollar quoting: $$…$$ or $tag$…$tag$
      const m = /^\$([A-Za-z_][A-Za-z0-9_]*)?\$/.exec(sql.slice(i));
      if (m) {
        const e = sql.indexOf(m[0], i + m[0].length);
        i = e < 0 ? n : e + m[0].length;
      } else i++;
    } else if (c === ":" && sql[i - 1] !== ":" && next !== ":" && next !== "=") {
      IDENT.lastIndex = i + 1;
      const m = IDENT.exec(sql);
      if (m && !/[A-Za-z0-9_]/.test(sql[i - 1] ?? "")) {
        hits.push({ start: i, end: i + 1 + m[0].length, name: m[0], raw: false });
        i += 1 + m[0].length;
      } else i++;
    } else i++;
  }
  return hits.sort((a, b) => a.start - b.start);
}

/** `:name` doesn't apply to Redis keys (`user:1`) or mongosh objects (`{ a: 1 }`). */
const colonFor = (dialect: string) => !["redis", "mongodb"].includes(dialect);

/** Distinct variables in order of first use. */
export function findVars(sql: string, dialect: string): VarRef[] {
  const seen = new Map<string, VarRef>();
  for (const h of scan(sql, colonFor(dialect))) if (!seen.has(h.name)) seen.set(h.name, { name: h.name, raw: h.raw });
  return [...seen.values()];
}

export function literal(v: string, dialect: string): string {
  const t = v.trim();
  if (/^-?\d+(\.\d+)?$/.test(t) || /^(null|true|false)$/i.test(t)) return t;
  const q = v.replace(/'/g, "''");
  if (dialect === "bigquery") return `'${v.replace(/\\/g, "\\\\").replace(/'/g, "\\'")}'`;
  return dialect === "mysql" || dialect === "clickhouse" ? `'${q.replace(/\\/g, "\\\\")}'` : `'${q}'`;
}

export function substitute(sql: string, values: Record<string, string>, dialect: string): string {
  let out = "";
  let at = 0;
  for (const h of scan(sql, colonFor(dialect))) {
    if (h.start < at) continue;
    const v = values[h.name] ?? "";
    out += sql.slice(at, h.start) + (h.raw ? v : literal(v, dialect));
    at = h.end;
  }
  return out + sql.slice(at);
}
