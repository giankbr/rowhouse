import type { Cell, ChangeSet, ColumnInfo, QueryResult } from "./api";

/** Unsaved grid edits, keyed by original result-row index and column index. */
export interface Pending {
  edits: Record<number, Record<number, Cell>>;
  deletes: number[];
  /** new rows: column index → value (missing = DEFAULT) */
  inserts: Record<number, Cell>[];
}

export const emptyPending = (): Pending => ({ edits: {}, deletes: [], inserts: [] });

export function countPending(p: Pending | undefined): number {
  if (!p) return 0;
  return Object.keys(p.edits).length + p.deletes.length + p.inserts.length;
}

/** Why this result can't be edited, or null if it can. */
export function editBlocker(result: QueryResult | undefined, columns: ColumnInfo[] | undefined): string | null {
  if (!result || !columns) return "loading";
  const pk = columns.filter((c) => c.primary_key).map((c) => c.name);
  if (pk.length === 0) return "Table has no primary key — read-only";
  const names = result.columns.map((c) => c.name);
  if (!pk.every((k) => names.includes(k))) return "Primary key not in result — read-only";
  return null;
}

export function toChangeSet(
  p: Pending,
  result: QueryResult,
  columns: ColumnInfo[],
  schema: string,
  table: string,
): ChangeSet {
  const names = result.columns.map((c) => c.name);
  const pkIdx = columns.filter((c) => c.primary_key).map((c) => names.indexOf(c.name));
  const keyOf = (rowIdx: number) =>
    Object.fromEntries(pkIdx.map((ci) => [names[ci], result.rows[rowIdx][ci]]));
  const byName = (row: Record<number, Cell>) =>
    Object.fromEntries(Object.entries(row).map(([ci, v]) => [names[Number(ci)], v]));

  return {
    schema,
    table,
    updates: Object.entries(p.edits)
      .filter(([ri]) => !p.deletes.includes(Number(ri)))
      .map(([ri, row]) => ({ key: keyOf(Number(ri)), set: byName(row) })),
    inserts: p.inserts.map(byName),
    deletes: p.deletes.map(keyOf),
  };
}
