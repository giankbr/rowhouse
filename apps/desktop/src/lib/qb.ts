/**
 * Visual query builder model → SQL. Pure functions, no React.
 */
import type { ColumnInfo, Dialect, ErdTable, ForeignKey } from "./api";

export type JoinType = "INNER" | "LEFT" | "RIGHT" | "FULL";
export type Agg = "" | "count" | "count-distinct" | "sum" | "avg" | "min" | "max";
export type QbOp = "=" | "<>" | "<" | "<=" | ">" | ">=" | "contains" | "starts" | "in" | "is null" | "is not null";

export interface QbTable {
  id: string;
  schema: string;
  name: string;
  alias: string;
  x: number;
  y: number;
  columns: ColumnInfo[];
}
export interface QbJoin {
  id: string;
  from: { t: string; col: string };
  to: { t: string; col: string };
  type: JoinType;
}
export interface QbCol {
  t: string;
  col: string;
  agg: Agg;
  alias: string;
}
export interface QbFilter {
  id: string;
  t: string;
  col: string;
  op: QbOp;
  value: string;
}
export interface QbSort {
  t: string;
  col: string;
  desc: boolean;
}
export interface QbState {
  tables: QbTable[];
  joins: QbJoin[];
  columns: QbCol[];
  filters: QbFilter[];
  sorts: QbSort[];
  limit: number | null;
  distinct: boolean;
}

export const emptyQb = (): QbState => ({ tables: [], joins: [], columns: [], filters: [], sorts: [], limit: 100, distinct: false });

const rid = () => Math.random().toString(36).slice(2, 9);

export function quoteIdent(d: Dialect, name: string) {
  if (d === "mysql" || d === "clickhouse") return "`" + name.replace(/`/g, "``") + "`";
  if (d === "mssql") return "[" + name.replace(/]/g, "]]") + "]";
  return /^[a-z_][a-z0-9_]*$/.test(name) && !RESERVED.has(name) ? name : `"${name.replace(/"/g, '""')}"`;
}
const RESERVED = new Set(["user", "order", "group", "select", "from", "where", "table", "limit", "offset", "default", "check", "column", "references", "primary", "key", "to", "end", "desc", "asc", "all", "and", "or", "not", "null", "in", "is", "as", "on", "join"]);

const NUMERIC = /int|serial|numeric|decimal|float|double|real|money|number/i;

function literal(d: Dialect, v: string, numeric: boolean) {
  const t = v.trim();
  if (numeric && t !== "" && !isNaN(Number(t))) return t;
  if (/^(true|false)$/i.test(t) && d !== "mssql" && d !== "sqlite") return t.toUpperCase();
  return (d === "mssql" ? "N'" : "'") + t.replace(/'/g, "''") + "'";
}

/** Unique alias for a new table: `orders` → `o`, then `o2`… (short, readable). */
export function nextAlias(state: QbState, name: string) {
  const words = name.split(/[_\W]+/).filter(Boolean);
  let base = words.map((w) => w[0]).join("").toLowerCase() || "t";
  if (/^\d/.test(base)) base = "t" + base;
  if (RESERVED.has(base)) base = base + "_";
  let a = base;
  let n = 2;
  while (state.tables.some((t) => t.alias === a)) a = `${base}${n++}`;
  return a;
}

/** Add a table; joins it to tables already on the canvas through foreign keys. */
export function addTable(state: QbState, t: ErdTable, fks: ForeignKey[], pos?: { x: number; y: number }): QbState {
  const id = rid();
  const n = state.tables.length;
  const table: QbTable = {
    id,
    schema: t.schema,
    name: t.name,
    alias: nextAlias(state, t.name),
    x: pos?.x ?? 32 + (n % 4) * 310,
    y: pos?.y ?? 28 + (n % 2) * 36 + Math.floor(n / 4) * 280,
    columns: t.columns,
  };
  const joins = [...state.joins];
  // first FK between the new table and one already placed (either direction)
  for (const fk of fks) {
    const out = fk.schema === t.schema && fk.table === t.name;
    const inn = fk.ref_schema === t.schema && fk.ref_table === t.name;
    if (!out && !inn) continue;
    const other = state.tables.find((x) =>
      out ? x.schema === fk.ref_schema && x.name === fk.ref_table : x.schema === fk.schema && x.name === fk.table,
    );
    if (!other) continue;
    fk.columns.forEach((c, i) => {
      const rc = fk.ref_columns[i];
      joins.push(
        out
          ? { id: rid(), from: { t: other.id, col: rc }, to: { t: id, col: c }, type: "INNER" }
          : { id: rid(), from: { t: other.id, col: c }, to: { t: id, col: rc }, type: "INNER" },
      );
    });
    break;
  }
  return { ...state, tables: [...state.tables, table], joins };
}

export function removeTable(state: QbState, id: string): QbState {
  return {
    ...state,
    tables: state.tables.filter((t) => t.id !== id),
    joins: state.joins.filter((j) => j.from.t !== id && j.to.t !== id),
    columns: state.columns.filter((c) => c.t !== id),
    filters: state.filters.filter((f) => f.t !== id),
    sorts: state.sorts.filter((s) => s.t !== id),
  };
}

export function toggleColumn(state: QbState, t: string, col: string): QbState {
  const has = state.columns.some((c) => c.t === t && c.col === col);
  return {
    ...state,
    columns: has ? state.columns.filter((c) => !(c.t === t && c.col === col)) : [...state.columns, { t, col, agg: "", alias: "" }],
  };
}

export function addJoin(state: QbState, a: { t: string; col: string }, b: { t: string; col: string }): QbState {
  if (a.t === b.t) return state;
  if (state.joins.some((j) => (j.from.t === a.t && j.from.col === a.col && j.to.t === b.t && j.to.col === b.col) || (j.from.t === b.t && j.from.col === b.col && j.to.t === a.t && j.to.col === a.col)))
    return state;
  return { ...state, joins: [...state.joins, { id: rid(), from: a, to: b, type: "INNER" }] };
}

export interface BuiltSql {
  sql: string;
  warnings: string[];
}

export function buildSql(state: QbState, d: Dialect): BuiltSql {
  const warnings: string[] = [];
  if (!state.tables.length) return { sql: "", warnings };
  const q = (s: string) => quoteIdent(d, s);
  const byId = new Map(state.tables.map((t) => [t.id, t]));
  const target = (t: QbTable) => (d === "sqlite" ? q(t.name) : `${q(t.schema)}.${q(t.name)}`);
  const ref = (t: string, col: string) => `${byId.get(t)?.alias ?? "?"}.${q(col)}`;
  const isNum = (t: string, col: string) => NUMERIC.test(byId.get(t)?.columns.find((c) => c.name === col)?.data_type ?? "");

  // FROM + JOINs: walk the join graph from the first table
  const [root, ...rest] = state.tables;
  const placed = new Set([root.id]);
  const lines = [`FROM ${target(root)} ${root.alias}`];
  let pending = rest.slice();
  while (pending.length) {
    const next = pending.find((t) => state.joins.some((j) => (j.to.t === t.id && placed.has(j.from.t)) || (j.from.t === t.id && placed.has(j.to.t))));
    if (!next) {
      const t = pending[0];
      warnings.push(`${t.alias} isn't joined to anything — that's a cross join (every row × every row)`);
      lines.push(`CROSS JOIN ${target(t)} ${t.alias}`);
      placed.add(t.id);
      pending = pending.slice(1);
      continue;
    }
    const conds = state.joins.filter((j) => (j.to.t === next.id && placed.has(j.from.t)) || (j.from.t === next.id && placed.has(j.to.t)));
    // the join type is set on the join whose `to` side is the new table; flip it otherwise
    const j0 = conds[0];
    let type = j0.type;
    if (j0.from.t === next.id) type = type === "LEFT" ? "RIGHT" : type === "RIGHT" ? "LEFT" : type;
    if (type === "FULL" && (d === "mysql" || d === "sqlite")) warnings.push(`${d === "mysql" ? "MySQL" : "SQLite"} has no FULL JOIN`);
    if (type === "RIGHT" && d === "sqlite") warnings.push("older SQLite versions have no RIGHT JOIN");
    const kw = type === "INNER" ? "JOIN" : `${type} JOIN`;
    lines.push(`${kw} ${target(next)} ${next.alias} ON ${conds.map((c) => `${ref(c.from.t, c.from.col)} = ${ref(c.to.t, c.to.col)}`).join(" AND ")}`);
    placed.add(next.id);
    pending = pending.filter((t) => t !== next);
  }

  // SELECT list
  const hasAgg = state.columns.some((c) => c.agg);
  const sel = state.columns.length
    ? state.columns.map((c) => {
        const r = ref(c.t, c.col);
        const e =
          c.agg === "count-distinct" ? `COUNT(DISTINCT ${r})` : c.agg ? `${c.agg.toUpperCase()}(${r})` : r;
        const alias = c.alias.trim() || (c.agg ? `${c.agg.replace("-distinct", "_distinct")}_${c.col}` : "");
        return alias ? `${e} AS ${q(alias)}` : e;
      })
    : [state.tables.length > 1 ? state.tables.map((t) => `${t.alias}.*`).join(", ") : "*"];

  // WHERE
  const like = d === "postgres" || d === "duckdb" || d === "clickhouse" ? "ILIKE" : "LIKE";
  const where = state.filters
    .filter((f) => f.col)
    .map((f) => {
      const r = ref(f.t, f.col);
      const n = isNum(f.t, f.col);
      switch (f.op) {
        case "is null":
          return `${r} IS NULL`;
        case "is not null":
          return `${r} IS NOT NULL`;
        case "contains":
          return `${r} ${like} ${literal(d, `%${f.value}%`, false)}`;
        case "starts":
          return `${r} ${like} ${literal(d, `${f.value}%`, false)}`;
        case "in": {
          const items = f.value.split(",").map((s) => s.trim()).filter(Boolean);
          return items.length ? `${r} IN (${items.map((v) => literal(d, v, n)).join(", ")})` : "1 = 0";
        }
        default:
          return `${r} ${f.op} ${literal(d, f.value, n)}`;
      }
    });

  const groupBy = hasAgg ? state.columns.filter((c) => !c.agg).map((c) => ref(c.t, c.col)) : [];
  const orderBy = state.sorts.filter((s) => s.col).map((s) => `${ref(s.t, s.col)}${s.desc ? " DESC" : ""}`);
  if (hasAgg && state.sorts.some((s) => !state.columns.some((c) => c.t === s.t && c.col === s.col)))
    warnings.push("with aggregates, sort only by selected columns");

  const top = d === "mssql" && state.limit ? `TOP ${state.limit} ` : "";
  const out = [`SELECT ${state.distinct ? "DISTINCT " : ""}${top}${sel.join(",\n       ")}`, ...lines];
  if (where.length) out.push(`WHERE ${where.join("\n  AND ")}`);
  if (groupBy.length) out.push(`GROUP BY ${groupBy.join(", ")}`);
  if (orderBy.length) out.push(`ORDER BY ${orderBy.join(", ")}`);
  if (state.limit && d === "oracle") out.push(`FETCH FIRST ${state.limit} ROWS ONLY`);
  else if (state.limit && d !== "mssql") out.push(`LIMIT ${state.limit}`);
  return { sql: out.join("\n") + ";", warnings };
}
