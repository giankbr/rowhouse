// Row-level diff between two results of the same query. Same rules as
// rowhouse-core's diff.rs: match rows by a key (picked, or an `id` / `*_id`
// column unique in both), else compare whole rows as a multiset.
import type { Cell, QueryResult } from "./api";

export interface Changed {
  before: Cell[];
  after: Cell[];
  cells: number[];
}

export interface RowDiff {
  columns: string[];
  key: string[];
  columnsAdded: string[];
  columnsRemoved: string[];
  added: Cell[][];
  removed: Cell[][];
  changed: Changed[];
  unchanged: number;
}

const j = (v: unknown) => JSON.stringify(v ?? null);

function unique(r: QueryResult, i: number) {
  const seen = new Set<string>();
  for (const row of r.rows) {
    const v = row[i];
    if (v === null || v === undefined) return false;
    const k = j(v);
    if (seen.has(k)) return false;
    seen.add(k);
  }
  return true;
}

export function autoKey(prev: QueryResult, next: QueryResult): string[] {
  const names = next.columns.map((c) => c.name);
  const cands = [...names.filter((n) => n.toLowerCase() === "id"), ...names.filter((n) => /(_id|^uuid|^key)$/i.test(n) && n.toLowerCase() !== "id")];
  for (const c of cands) {
    const pi = prev.columns.findIndex((x) => x.name === c);
    const ni = next.columns.findIndex((x) => x.name === c);
    if (pi >= 0 && ni >= 0 && unique(prev, pi) && unique(next, ni)) return [c];
  }
  return [];
}

export function diffResults(prev: QueryResult, next: QueryResult, key?: string[]): RowDiff {
  const pn = prev.columns.map((c) => c.name);
  const nn = next.columns.map((c) => c.name);
  const common = nn.filter((n) => pn.includes(n));
  const pi = common.map((c) => pn.indexOf(c));
  const ni = common.map((c) => nn.indexOf(c));
  const prow = prev.rows.map((r) => pi.map((i) => r[i] ?? null));
  const nrow = next.rows.map((r) => ni.map((i) => r[i] ?? null));
  const out: RowDiff = {
    columns: common,
    key: [],
    columnsAdded: nn.filter((n) => !pn.includes(n)),
    columnsRemoved: pn.filter((n) => !nn.includes(n)),
    added: [],
    removed: [],
    changed: [],
    unchanged: 0,
  };
  const k = key?.length ? key.filter((c) => common.includes(c)) : autoKey(prev, next);
  if (!k.length) {
    const counts = new Map<string, number>();
    for (const r of prow) counts.set(j(r), (counts.get(j(r)) ?? 0) + 1);
    for (const r of nrow) {
      const n = counts.get(j(r)) ?? 0;
      if (n > 0) (counts.set(j(r), n - 1), out.unchanged++);
      else out.added.push(r);
    }
    for (const r of prow) {
      const n = counts.get(j(r)) ?? 0;
      if (n > 0) (counts.set(j(r), n - 1), out.removed.push(r));
    }
    return out;
  }
  out.key = k;
  const kidx = k.map((c) => common.indexOf(c));
  const kof = (r: Cell[]) => j(kidx.map((i) => r[i]));
  const before = new Map(prow.map((r) => [kof(r), r]));
  for (const r of nrow) {
    const b = before.get(kof(r));
    if (!b) {
      out.added.push(r);
      continue;
    }
    before.delete(kof(r));
    const cells = common.map((_, i) => i).filter((i) => j(b[i]) !== j(r[i]));
    if (cells.length) out.changed.push({ before: b, after: r, cells });
    else out.unchanged++;
  }
  for (const r of prow) if (before.has(kof(r))) out.removed.push(r);
  return out;
}

export function summaryText(d: { added: number; removed: number; changed: number }) {
  const parts = [d.added && `${d.added} added`, d.removed && `${d.removed} removed`, d.changed && `${d.changed} changed`].filter(Boolean);
  return parts.length ? parts.join(" · ") : "no row changes";
}
