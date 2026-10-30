import { AlertCircle, KeyRound, Plus, RotateCcw, Trash2 } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { api, kindInfo, type ColumnInfo, type ColumnOp, type ConnectionConfig } from "../lib/api";
import { Button, Modal, cx } from "./ui";

type Row = { orig: ColumnInfo | null; cur: ColumnInfo; dropped: boolean };

const TYPES: Record<string, string[]> = {
  postgres: ["text", "varchar(255)", "integer", "bigint", "numeric(12,2)", "boolean", "timestamptz", "date", "jsonb", "uuid"],
  mysql: ["varchar(255)", "text", "int", "bigint", "decimal(12,2)", "tinyint(1)", "datetime", "date", "json"],
  sqlite: ["TEXT", "INTEGER", "REAL", "BLOB", "NUMERIC"],
  duckdb: ["VARCHAR", "INTEGER", "BIGINT", "DOUBLE", "DECIMAL(12,2)", "BOOLEAN", "TIMESTAMP", "DATE", "JSON"],
  mssql: ["nvarchar(255)", "nvarchar(max)", "int", "bigint", "decimal(12,2)", "bit", "datetime2", "date", "uniqueidentifier"],
  clickhouse: ["String", "Int32", "Int64", "UInt64", "Float64", "Decimal(12,2)", "Bool", "DateTime", "Date", "Nullable(String)"],
};

const same = (a: ColumnInfo, b: ColumnInfo) =>
  a.name === b.name && a.data_type.trim() === b.data_type.trim() && a.nullable === b.nullable && (a.default ?? "") === (b.default ?? "");

/** Editable column list; changes become DDL that is previewed before it runs. */
export function StructureEditor({
  conn,
  schema,
  table,
  columns,
  onApplied,
  onToast,
}: {
  conn: ConnectionConfig;
  schema: string;
  table: string;
  columns: ColumnInfo[];
  onApplied: () => void;
  onToast: (msg: string, kind?: "ok" | "err") => void;
}) {
  const reset = () => columns.map((c) => ({ orig: c, cur: { ...c }, dropped: false }));
  const [rows, setRows] = useState<Row[]>(reset);
  const [review, setReview] = useState<{ sql: string[]; busy?: boolean; error?: string } | null>(null);
  useEffect(() => setRows(reset()), [columns]); // eslint-disable-line react-hooks/exhaustive-deps
  const dialect = kindInfo(conn.kind).dialect;

  const ops = useMemo<ColumnOp[]>(() => {
    const out: ColumnOp[] = [];
    for (const r of rows) {
      if (!r.orig) {
        if (!r.dropped && r.cur.name.trim()) out.push({ op: "add", column: { ...r.cur, name: r.cur.name.trim() } });
        continue;
      }
      if (r.dropped) {
        out.push({ op: "drop", name: r.orig.name });
        continue;
      }
      const name = r.cur.name.trim();
      if (name && name !== r.orig.name) out.push({ op: "rename", from: r.orig.name, to: name });
      const from = { ...r.orig, name: name || r.orig.name };
      const to = { ...r.cur, name: name || r.orig.name, default: r.cur.default?.trim() ? r.cur.default : null };
      if (!same(from, to)) out.push({ op: "alter", from, to });
    }
    return out;
  }, [rows]);

  const set = (i: number, p: Partial<ColumnInfo>) => setRows((rs) => rs.map((r, j) => (j === i ? { ...r, cur: { ...r.cur, ...p } } : r)));
  const changed = (r: Row) => !r.orig || r.dropped || !same(r.orig, r.cur);

  const openReview = async () => {
    try {
      setReview({ sql: await api.structurePreview(conn.id, schema, table, ops) });
    } catch (e) {
      setReview({ sql: [], error: String(e) });
    }
  };
  const apply = async () => {
    if (!review) return;
    setReview({ ...review, busy: true, error: undefined });
    try {
      await api.applyStructure(conn.id, schema, table, ops);
      setReview(null);
      onToast(`Updated ${table}`, "ok");
      onApplied();
    } catch (e) {
      setReview({ ...review, busy: false, error: String(e) });
    }
  };

  const cell = "h-7 w-full rounded-md border border-transparent bg-transparent px-1.5 font-mono text-xs outline-none hover:border-line focus:border-accent focus:bg-elevated";

  return (
    <div className="flex h-full flex-col">
      <div className="min-h-0 flex-1 overflow-auto">
        <table className="w-full border-collapse text-xs">
          <thead className="sticky top-0 z-10 bg-panel text-left text-2xs text-muted">
            <tr className="h-7 border-b border-line">
              <th className="w-8" />
              <th className="px-2 font-medium">Column</th>
              <th className="px-2 font-medium">Type</th>
              <th className="w-20 px-2 text-center font-medium">Nullable</th>
              <th className="px-2 font-medium">Default</th>
              <th className="w-10" />
            </tr>
          </thead>
          <tbody>
            {rows.map((r, i) => (
              <tr
                key={i}
                className={cx(
                  "h-8 border-b border-line/60",
                  r.dropped && "bg-danger/8 line-through opacity-70",
                  !r.orig && "bg-success/8",
                  r.orig && !r.dropped && changed(r) && "bg-warning/10",
                )}
              >
                <td className="pl-3 text-warning">{r.cur.primary_key && <KeyRound size={11} />}</td>
                <td className="px-1">
                  <input className={cx(cell, "font-medium")} value={r.cur.name} disabled={r.dropped} placeholder="column_name" onChange={(e) => set(i, { name: e.target.value })} />
                </td>
                <td className="px-1">
                  <input className={cx(cell, "text-accent")} list={`rh-types-${dialect}`} value={r.cur.data_type} disabled={r.dropped} placeholder="type" onChange={(e) => set(i, { data_type: e.target.value })} />
                </td>
                <td className="text-center">
                  <input type="checkbox" className="accent-[var(--accent)]" checked={r.cur.nullable} disabled={r.dropped || r.cur.primary_key} onChange={(e) => set(i, { nullable: e.target.checked })} />
                </td>
                <td className="px-1">
                  <input className={cx(cell, "text-muted")} value={r.cur.default ?? ""} disabled={r.dropped} placeholder="—" onChange={(e) => set(i, { default: e.target.value || null })} />
                </td>
                <td className="pr-2 text-right">
                  {r.dropped || (r.orig && changed(r)) ? (
                    <Button
                      variant="ghost"
                      size="icon"
                      title="Undo"
                      onClick={() => setRows((rs) => rs.map((x, j) => (j === i ? { orig: x.orig, cur: { ...x.orig! }, dropped: false } : x)))}
                    >
                      <RotateCcw size={12} />
                    </Button>
                  ) : (
                    <Button
                      variant="ghost"
                      size="icon"
                      title={r.cur.primary_key ? "Primary key columns can't be dropped here" : "Drop column"}
                      disabled={r.cur.primary_key}
                      onClick={() => setRows((rs) => (r.orig ? rs.map((x, j) => (j === i ? { ...x, dropped: true } : x)) : rs.filter((_, j) => j !== i)))}
                    >
                      <Trash2 size={12} />
                    </Button>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        <datalist id={`rh-types-${dialect}`}>
          {(TYPES[dialect] ?? TYPES.postgres).map((t) => (
            <option key={t} value={t} />
          ))}
        </datalist>
      </div>
      <div className="flex h-9 shrink-0 items-center gap-2 border-t border-line bg-panel px-2">
        <Button
          variant="ghost"
          size="xs"
          onClick={() =>
            setRows((rs) => [...rs, { orig: null, dropped: false, cur: { name: "", data_type: (TYPES[dialect] ?? TYPES.postgres)[0], nullable: true, default: null, primary_key: false } }])
          }
        >
          <Plus size={12} /> Column
        </Button>
        <span className="flex-1" />
        {ops.length > 0 && (
          <>
            <span className="text-2xs text-warning">{ops.length} change{ops.length === 1 ? "" : "s"}</span>
            <Button variant="ghost" size="xs" onClick={() => setRows(reset())}>
              Discard
            </Button>
            <Button variant="primary" size="xs" onClick={openReview}>
              Review & apply
            </Button>
          </>
        )}
      </div>
      <Modal
        open={!!review}
        onClose={() => !review?.busy && setReview(null)}
        width={640}
        title={`Change structure · ${table}`}
        footer={
          <>
            <span className="flex-1 text-2xs text-subtle">
              {dialect === "mysql" ? "MySQL commits each DDL statement on its own." : "Runs in one transaction where the database supports it."}
            </span>
            <Button onClick={() => setReview(null)} disabled={review?.busy}>
              Cancel
            </Button>
            <Button variant="primary" onClick={apply} disabled={review?.busy || !review?.sql.length}>
              {review?.busy ? "Applying…" : "Apply"}
            </Button>
          </>
        }
      >
        <pre className="selectable max-h-72 overflow-auto rounded-lg border border-line bg-panel p-3 font-mono text-2xs leading-relaxed whitespace-pre-wrap">
          {review?.sql.map((s) => (s.startsWith("--") ? s : s + ";")).join("\n")}
        </pre>
        {review?.error && (
          <div className="selectable mt-2 flex gap-2 rounded-lg border border-danger/30 bg-danger/5 p-2.5 font-mono text-2xs text-danger">
            <AlertCircle size={13} className="shrink-0" />
            <pre className="whitespace-pre-wrap">{review.error}</pre>
          </div>
        )}
      </Modal>
    </div>
  );
}
