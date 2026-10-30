import { Dices, RefreshCw, Sparkles } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { api, type ColumnGen, type ConnectionConfig, type Gen, type QueryResult, type TableInfo } from "../lib/api";
import { display } from "./ResultsGrid";
import { Button, Modal, cx } from "./ui";

const KINDS: { kind: Gen["kind"]; label: string; group: string }[] = [
  { kind: "skip", label: "Default (leave out)", group: "Basic" },
  { kind: "null", label: "NULL", group: "Basic" },
  { kind: "fixed", label: "Fixed value", group: "Basic" },
  { kind: "pick", label: "One of…", group: "Basic" },
  { kind: "sequence", label: "Sequence", group: "Numbers" },
  { kind: "int", label: "Integer", group: "Numbers" },
  { kind: "float", label: "Decimal", group: "Numbers" },
  { kind: "bool", label: "Boolean", group: "Numbers" },
  { kind: "fullName", label: "Full name", group: "People" },
  { kind: "firstName", label: "First name", group: "People" },
  { kind: "lastName", label: "Last name", group: "People" },
  { kind: "email", label: "Email", group: "People" },
  { kind: "phone", label: "Phone", group: "People" },
  { kind: "company", label: "Company", group: "People" },
  { kind: "city", label: "City", group: "Places" },
  { kind: "country", label: "Country", group: "Places" },
  { kind: "date", label: "Date", group: "Time" },
  { kind: "timestamp", label: "Timestamp", group: "Time" },
  { kind: "words", label: "Words", group: "Text" },
  { kind: "url", label: "URL", group: "Text" },
  { kind: "uuid", label: "UUID", group: "Text" },
  { kind: "json", label: "JSON object", group: "Text" },
  { kind: "reference", label: "Existing value (FK)", group: "Relations" },
];

function defaults(kind: Gen["kind"], prev: Gen): Gen {
  const y = new Date().getFullYear();
  switch (kind) {
    case "fixed":
      return { kind, value: "" };
    case "pick":
      return { kind, values: ["a", "b", "c"] };
    case "sequence":
      return { kind, start: 1 };
    case "int":
      return { kind, min: 1, max: 1000 };
    case "float":
      return { kind, min: 1, max: 1000, decimals: 2 };
    case "date":
    case "timestamp":
      return { kind, from: `${y - 2}-01-01`, to: `${y}-12-31` };
    case "words":
      return { kind, min: 2, max: 6 };
    case "reference":
      return prev.kind === "reference" ? prev : { kind, schema: "", table: "", column: "id" };
    default:
      return { kind } as Gen;
  }
}

const inner = (g: Gen): Gen => (g.kind === "nullable" ? g.inner : g);

/** Fill a table with realistic dummy rows; foreign keys use existing values. */
export function FakeDataDialog({
  conn,
  table,
  onClose,
  onDone,
  onToast,
}: {
  conn: ConnectionConfig;
  table: TableInfo | null;
  onClose: () => void;
  onDone: (t: TableInfo, n: number) => void;
  onToast: (m: string, k?: "ok" | "err") => void;
}) {
  const [plan, setPlan] = useState<ColumnGen[] | null>(null);
  const [count, setCount] = useState(100);
  const [seed, setSeed] = useState(() => Math.floor(Math.random() * 1e9));
  const [preview, setPreview] = useState<QueryResult | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [progress, setProgress] = useState<number | null>(null);

  useEffect(() => {
    if (!table) return;
    setPlan(null);
    setPreview(null);
    setErr(null);
    setProgress(null);
    api
      .fakePlan(conn.id, table.schema, table.name)
      .then(setPlan)
      .catch((e) => setErr(String(e)));
  }, [conn.id, table]);

  const key = useMemo(() => JSON.stringify(plan), [plan]);
  useEffect(() => {
    if (!plan) return;
    const t = setTimeout(
      () =>
        api
          .fakePreview(conn.id, plan, seed)
          .then((r) => (setPreview(r), setErr(null)))
          .catch((e) => (setPreview(null), setErr(String(e)))),
      200,
    );
    return () => clearTimeout(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [conn.id, key, seed]);

  if (!table) return null;
  const set = (i: number, gen: Gen) => setPlan((p) => p && p.map((c, j) => (j === i ? { ...c, gen } : c)));
  const run = async () => {
    if (!plan) return;
    setProgress(0);
    try {
      const n = await api.fakeInsert(conn.id, table.schema, table.name, plan, count, seed, setProgress);
      onDone(table, n);
    } catch (e) {
      onToast(String(e));
    } finally {
      setProgress(null);
    }
  };
  const busy = progress !== null;
  const field = "h-6 rounded-md border border-line bg-elevated px-1.5 text-[11.5px] outline-none focus:border-accent";

  return (
    <Modal
      open
      onClose={() => !busy && onClose()}
      width={880}
      title={
        <span className="flex items-center gap-2">
          <Sparkles size={14} className="text-accent" /> Generate data · {table.name}
        </span>
      }
      footer={
        <>
          <span className="text-2xs text-muted">Rows</span>
          <input type="number" min={1} max={1_000_000} value={count} onChange={(e) => setCount(Math.max(1, Math.min(1_000_000, Number(e.target.value) || 1)))} className={cx(field, "w-24 tabular-nums")} />
          {[100, 1000, 10000].map((n) => (
            <button key={n} onClick={() => setCount(n)} className={cx("h-6 rounded px-1.5 text-2xs", count === n ? "bg-selected text-fg" : "text-muted hover:text-fg")}>
              {n.toLocaleString()}
            </button>
          ))}
          <span className="flex-1">
            {busy && (
              <span className="ml-2 flex items-center gap-2 text-2xs text-muted">
                <span className="h-1.5 w-40 overflow-hidden rounded-full bg-hover">
                  <span className="block h-full rounded-full bg-accent transition-[width]" style={{ width: `${Math.round(((progress ?? 0) / count) * 100)}%` }} />
                </span>
                {(progress ?? 0).toLocaleString()} / {count.toLocaleString()}
              </span>
            )}
          </span>
          <Button variant="ghost" disabled={busy} onClick={onClose}>
            Cancel
          </Button>
          <Button variant="primary" disabled={!plan || !!err || busy || conn.readOnly} onClick={run}>
            Insert {count.toLocaleString()} rows
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-3">
        {!plan && !err && <div className="p-6 text-center text-xs text-subtle">Reading columns…</div>}
        {plan && (
          <div className="rounded-lg border border-line">
            <div className="grid grid-cols-[180px_200px_1fr_90px] items-center gap-2 border-b border-line bg-panel px-3 py-1 text-2xs text-subtle">
              <span>Column</span>
              <span>Generator</span>
              <span>Options</span>
              <span className="text-right">NULLs</span>
            </div>
            <div className="max-h-[34vh] overflow-y-auto">
              {plan.map((c, i) => {
                const g = inner(c.gen);
                const nullPct = c.gen.kind === "nullable" ? c.gen.percent : 0;
                const wrap = (x: Gen): Gen => (nullPct ? { kind: "nullable", percent: nullPct, inner: x } : x);
                return (
                  <div key={c.column} className={cx("grid grid-cols-[180px_200px_1fr_90px] items-center gap-2 border-b border-line/60 px-3 py-1 last:border-0", g.kind === "skip" && "opacity-60")}>
                    <span className="min-w-0">
                      <span className="block truncate text-xs font-medium">{c.column}</span>
                      <span className="block truncate font-mono text-[10.5px] text-subtle">{c.dataType}</span>
                    </span>
                    <select value={g.kind} onChange={(e) => set(i, wrap(defaults(e.target.value as Gen["kind"], g)))} className={field}>
                      {[...new Set(KINDS.map((k) => k.group))].map((grp) => (
                        <optgroup key={grp} label={grp}>
                          {KINDS.filter((k) => k.group === grp).map((k) => (
                            <option key={k.kind} value={k.kind}>
                              {k.label}
                            </option>
                          ))}
                        </optgroup>
                      ))}
                    </select>
                    <span className="flex min-w-0 items-center gap-1.5 text-2xs text-muted">
                      {(g.kind === "int" || g.kind === "float" || g.kind === "words") && (
                        <>
                          <input type="number" value={g.min} onChange={(e) => set(i, wrap({ ...g, min: Number(e.target.value) }))} className={cx(field, "w-20")} />
                          to
                          <input type="number" value={g.max} onChange={(e) => set(i, wrap({ ...g, max: Number(e.target.value) }))} className={cx(field, "w-20")} />
                          {g.kind === "words" && "words"}
                        </>
                      )}
                      {g.kind === "sequence" && (
                        <>
                          from <input type="number" value={g.start} onChange={(e) => set(i, wrap({ ...g, start: Number(e.target.value) }))} className={cx(field, "w-24")} />
                        </>
                      )}
                      {(g.kind === "date" || g.kind === "timestamp") && (
                        <>
                          <input type="date" value={g.from} onChange={(e) => set(i, wrap({ ...g, from: e.target.value }))} className={field} />
                          to
                          <input type="date" value={g.to} onChange={(e) => set(i, wrap({ ...g, to: e.target.value }))} className={field} />
                        </>
                      )}
                      {g.kind === "fixed" && <input value={g.value} onChange={(e) => set(i, wrap({ ...g, value: e.target.value }))} className={cx(field, "w-full")} placeholder="value" />}
                      {g.kind === "pick" && (
                        <input value={g.values.join(", ")} onChange={(e) => set(i, wrap({ ...g, values: e.target.value.split(",").map((x) => x.trim()).filter(Boolean) }))} className={cx(field, "w-full")} placeholder="a, b, c" />
                      )}
                      {g.kind === "reference" && (
                        <span className="truncate font-mono text-[11px]">
                          {g.table ? `${g.table}.${g.column}` : <input value={g.table} onChange={(e) => set(i, wrap({ ...g, schema: table.schema, table: e.target.value }))} className={field} placeholder="table" />}
                        </span>
                      )}
                    </span>
                    <span className="flex justify-end">
                      {g.kind !== "skip" && g.kind !== "null" && (
                        <select
                          value={nullPct}
                          onChange={(e) => {
                            const pct = Number(e.target.value);
                            set(i, pct ? { kind: "nullable", percent: pct, inner: g } : g);
                          }}
                          className={cx(field, "w-20")}
                        >
                          {[0, 5, 10, 25, 50].map((n) => (
                            <option key={n} value={n}>
                              {n ? `${n}%` : "none"}
                            </option>
                          ))}
                        </select>
                      )}
                    </span>
                  </div>
                );
              })}
            </div>
          </div>
        )}
        {err && <div className="selectable rounded-lg border border-danger/30 bg-danger/5 p-2.5 text-2xs text-danger">{err}</div>}
        {preview && (
          <div>
            <div className="mb-1 flex items-center gap-2 text-2xs text-subtle">
              <Dices size={12} /> Preview
              <button onClick={() => setSeed(Math.floor(Math.random() * 1e9))} className="ml-auto flex items-center gap-1 hover:text-fg">
                <RefreshCw size={11} /> Reroll
              </button>
            </div>
            <div className="max-h-48 overflow-auto rounded-lg border border-line">
              <table className="w-max min-w-full border-collapse font-mono text-[11.5px]">
                <thead className="sticky top-0 bg-panel text-left text-2xs text-muted">
                  <tr>
                    {preview.columns.map((c) => (
                      <th key={c.name} className="border-b border-line px-2 py-1 font-sans font-medium">
                        {c.name}
                      </th>
                    ))}
                  </tr>
                </thead>
                <tbody>
                  {preview.rows.map((r, i) => (
                    <tr key={i} className="border-b border-line/50 last:border-0">
                      {r.map((v, j) => (
                        <td key={j} className={cx("max-w-56 truncate px-2 py-1", v === null && "text-subtle italic")}>
                          {display(v)}
                        </td>
                      ))}
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </div>
        )}
      </div>
    </Modal>
  );
}
