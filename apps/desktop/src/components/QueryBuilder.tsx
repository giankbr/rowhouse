import { ArrowDown, ArrowUp, Check, Code2, Copy, KeyRound, Play, Plus, Search, SquareTerminal, Trash2, X } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api, kindInfo, type ConnectionConfig, type Erd } from "../lib/api";
import {
  addJoin,
  addTable,
  buildSql,
  emptyQb,
  removeTable,
  toggleColumn,
  type Agg,
  type JoinType,
  type QbOp,
  type QbState,
} from "../lib/qb";
import { Button, Segmented, Switch, cx } from "./ui";

const CARD_W = 220;
const HEAD_H = 30;
const ROW_H = 22;
const MAX_ROWS = 30;

const OPS: { value: QbOp; label: string }[] = [
  { value: "=", label: "=" },
  { value: "<>", label: "≠" },
  { value: "<", label: "<" },
  { value: "<=", label: "≤" },
  { value: ">", label: ">" },
  { value: ">=", label: "≥" },
  { value: "contains", label: "contains" },
  { value: "starts", label: "starts with" },
  { value: "in", label: "in list" },
  { value: "is null", label: "is empty" },
  { value: "is not null", label: "is not empty" },
];
const AGGS: { value: Agg; label: string }[] = [
  { value: "", label: "—" },
  { value: "count", label: "count" },
  { value: "count-distinct", label: "count distinct" },
  { value: "sum", label: "sum" },
  { value: "avg", label: "avg" },
  { value: "min", label: "min" },
  { value: "max", label: "max" },
];

const sel = "h-6 rounded-md border border-line bg-elevated px-1.5 text-2xs outline-none focus:border-accent";

/** Visual query builder: tables on a canvas, joins drawn between columns, criteria below, live SQL. */
export function QueryBuilder({
  conn,
  state: initial,
  onState,
  onOpenSql,
  onRunSql,
  onToast,
}: {
  conn: ConnectionConfig;
  state?: QbState;
  onState: (s: QbState) => void;
  onOpenSql: (sql: string) => void;
  onRunSql: (sql: string) => void;
  onToast: (m: string) => void;
}) {
  const dialect = kindInfo(conn.kind).dialect;
  const [erd, setErd] = useState<Erd | null>(null);
  const [st, setSt] = useState<QbState>(initial ?? emptyQb());
  const [pick, setPick] = useState("");
  const [showSql, setShowSql] = useState(true);
  const [menu, setMenu] = useState<{ id: string; x: number; y: number } | null>(null);
  const [drag, setDrag] = useState<{ t: string; col: string; x: number; y: number } | null>(null);
  const canvas = useRef<HTMLDivElement>(null);

  useEffect(() => {
    api.erd(conn.id).then(setErd).catch((e) => onToast(String(e)));
  }, [conn.id, onToast]);

  const update = useCallback((f: (s: QbState) => QbState) => setSt(f), []);
  // keep the tab's copy in sync (so the builder survives tab switches)
  const onStateRef = useRef(onState);
  onStateRef.current = onState;
  const first = useRef(true);
  useEffect(() => {
    if (first.current) {
      first.current = false;
      return;
    }
    const t = setTimeout(() => onStateRef.current(st), 150);
    return () => clearTimeout(t);
  }, [st]);

  const built = useMemo(() => buildSql(st, dialect), [st, dialect]);
  const byId = useMemo(() => new Map(st.tables.map((t) => [t.id, t])), [st.tables]);

  const candidates = useMemo(() => {
    const n = pick.trim().toLowerCase();
    return (erd?.tables ?? []).filter((t) => !n || `${t.schema}.${t.name}`.toLowerCase().includes(n)).slice(0, 60);
  }, [erd, pick]);

  const colY = (tid: string, col: string) => {
    const t = byId.get(tid);
    if (!t) return 0;
    const i = Math.min(t.columns.findIndex((c) => c.name === col), MAX_ROWS);
    return t.y + HEAD_H + i * ROW_H + ROW_H / 2;
  };
  const anchors = (a: { t: string; col: string }, b: { t: string; col: string }) => {
    const ta = byId.get(a.t)!, tb = byId.get(b.t)!;
    const right = ta.x + CARD_W / 2 < tb.x + CARD_W / 2;
    const x1 = right ? ta.x + CARD_W : ta.x;
    const x2 = right ? tb.x : tb.x + CARD_W;
    return { x1, y1: colY(a.t, a.col), x2, y2: colY(b.t, b.col), dir: right ? 1 : -1 };
  };

  // move a card by its header
  const startMove = (e: React.PointerEvent, id: string) => {
    if ((e.target as HTMLElement).closest("button")) return;
    const t = byId.get(id)!;
    const sx = e.clientX, sy = e.clientY, ox = t.x, oy = t.y;
    const move = (ev: PointerEvent) =>
      setSt((s) => ({ ...s, tables: s.tables.map((x) => (x.id === id ? { ...x, x: Math.max(0, ox + ev.clientX - sx), y: Math.max(0, oy + ev.clientY - sy) } : x)) }));
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  };

  // drag from a column handle to another column to join
  const startJoin = (e: React.PointerEvent, t: string, col: string) => {
    e.preventDefault();
    e.stopPropagation();
    const box = canvas.current!.getBoundingClientRect();
    const pt = (ev: { clientX: number; clientY: number }) => ({ x: ev.clientX - box.left + canvas.current!.scrollLeft, y: ev.clientY - box.top + canvas.current!.scrollTop });
    setDrag({ t, col, ...pt(e) });
    const move = (ev: PointerEvent) => setDrag((d) => d && { ...d, ...pt(ev) });
    const up = (ev: PointerEvent) => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      setDrag(null);
      const el = (document.elementFromPoint(ev.clientX, ev.clientY) as HTMLElement | null)?.closest("[data-qb-col]") as HTMLElement | null;
      if (el) update((s) => addJoin(s, { t, col }, { t: el.dataset.qbT!, col: el.dataset.qbCol! }));
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  };

  const colOptions = st.tables.flatMap((t) => t.columns.map((c) => ({ key: `${t.id}|${c.name}`, label: `${t.alias}.${c.name}` })));
  const split = (k: string) => {
    const [t, col] = k.split("|");
    return { t, col };
  };
  const canvasH = Math.max(380, ...st.tables.map((t) => t.y + HEAD_H + Math.min(t.columns.length, MAX_ROWS + 1) * ROW_H + 40));
  const canvasW = Math.max(800, ...st.tables.map((t) => t.x + CARD_W + 40));

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {/* toolbar */}
      <div className="flex h-9 shrink-0 items-center gap-2 border-b border-line px-2">
        <div className="relative">
          <Search size={12} className="pointer-events-none absolute top-1/2 left-2 -translate-y-1/2 text-subtle" />
          <input
            value={pick}
            onChange={(e) => setPick(e.target.value)}
            placeholder={erd ? "Add table…" : "Loading tables…"}
            className="h-6 w-56 rounded-md border border-line bg-elevated pr-2 pl-6 text-xs outline-none focus:border-accent"
          />
          {pick && (
            <div className="absolute top-7 left-0 z-30 max-h-72 w-72 overflow-y-auto rounded-lg border border-line bg-elevated p-1 shadow-pop">
              {candidates.length === 0 && <div className="p-2 text-2xs text-subtle">No table matches</div>}
              {candidates.map((t) => (
                <button
                  key={`${t.schema}.${t.name}`}
                  className="flex w-full items-center gap-2 rounded-md px-2 py-1 text-left text-xs hover:bg-hover"
                  onClick={() => {
                    update((s) => addTable(s, t, erd!.foreign_keys));
                    setPick("");
                  }}
                >
                  <span className="text-subtle">{t.schema}.</span>
                  {t.name}
                  <span className="ml-auto text-2xs text-subtle">{t.columns.length} cols</span>
                </button>
              ))}
            </div>
          )}
        </div>
        <Switch checked={st.distinct} onChange={(distinct) => update((s) => ({ ...s, distinct }))} label={<span className="text-2xs">Distinct</span>} />
        <span className="flex items-center gap-1 text-2xs text-muted">
          Limit
          <input
            value={st.limit ?? ""}
            inputMode="numeric"
            placeholder="none"
            onChange={(e) => {
              const n = Number(e.target.value.replace(/\D/g, ""));
              update((s) => ({ ...s, limit: n > 0 ? n : null }));
            }}
            className="h-6 w-16 rounded-md border border-line bg-elevated px-1.5 text-2xs outline-none focus:border-accent"
          />
        </span>
        <span className="flex-1" />
        <Button size="xs" variant="ghost" onClick={() => update(() => emptyQb())} disabled={!st.tables.length}>
          Clear
        </Button>
        <Button size="xs" variant="ghost" onClick={() => setShowSql((v) => !v)} className={cx(showSql && "bg-selected text-fg")}>
          <Code2 size={11} /> SQL
        </Button>
        <Button size="xs" variant="ghost" disabled={!built.sql} onClick={() => (navigator.clipboard.writeText(built.sql), onToast("SQL copied"))}>
          <Copy size={11} /> Copy
        </Button>
        <Button size="xs" variant="ghost" disabled={!built.sql} onClick={() => onOpenSql(built.sql)}>
          <SquareTerminal size={11} /> Open in editor
        </Button>
        <Button size="xs" variant="primary" disabled={!built.sql} onClick={() => onRunSql(built.sql)}>
          <Play size={11} /> Run
        </Button>
      </div>

      {/* canvas */}
      <div ref={canvas} className="qb-canvas relative min-h-0 flex-1 overflow-auto" onPointerDown={() => setMenu(null)}>
        <div className="relative" style={{ width: canvasW, height: canvasH }}>
          {!st.tables.length && (
            <div className="absolute inset-0 flex flex-col items-center justify-center gap-1.5 text-xs text-muted">
              Add a table to start.
              <span className="text-2xs text-subtle">Tables with foreign keys join automatically · drag from a column dot to another column to join by hand</span>
            </div>
          )}
          <svg className="pointer-events-none absolute inset-0" width={canvasW} height={canvasH}>
            {st.joins.map((j) => {
              if (!byId.get(j.from.t) || !byId.get(j.to.t)) return null;
              const a = anchors(j.from, j.to);
              const c = 50 * a.dir;
              return (
                <path
                  key={j.id}
                  d={`M${a.x1},${a.y1} C${a.x1 + c},${a.y1} ${a.x2 - c},${a.y2} ${a.x2},${a.y2}`}
                  fill="none"
                  stroke="var(--accent)"
                  strokeWidth={1.5}
                  strokeDasharray={j.type === "INNER" ? undefined : "4 3"}
                />
              );
            })}
            {drag &&
              (() => {
                const t = byId.get(drag.t)!;
                const right = drag.x > t.x + CARD_W / 2;
                const x1 = right ? t.x + CARD_W : t.x, y1 = colY(drag.t, drag.col);
                return <line x1={x1} y1={y1} x2={drag.x} y2={drag.y} stroke="var(--accent)" strokeWidth={1.5} strokeDasharray="3 3" />;
              })()}
          </svg>
          {/* join type pills */}
          {st.joins.map((j) => {
            if (!byId.get(j.from.t) || !byId.get(j.to.t)) return null;
            const a = anchors(j.from, j.to);
            return (
              <button
                key={j.id}
                onPointerDown={(e) => e.stopPropagation()}
                onClick={() => setMenu({ id: j.id, x: (a.x1 + a.x2) / 2, y: (a.y1 + a.y2) / 2 + 14 })}
                className="absolute z-10 -translate-x-1/2 -translate-y-1/2 rounded-full border border-accent/40 bg-elevated px-1.5 py-px text-[10px] font-medium text-accent shadow-sm hover:bg-accent hover:text-accent-fg"
                style={{ left: (a.x1 + a.x2) / 2, top: (a.y1 + a.y2) / 2 }}
                title={`${byId.get(j.from.t)!.alias}.${j.from.col} = ${byId.get(j.to.t)!.alias}.${j.to.col}`}
              >
                {j.type}
              </button>
            );
          })}
          {menu && (
            <div
              className="absolute z-20 -translate-x-1/2 rounded-lg border border-line bg-elevated p-1 shadow-pop"
              style={{ left: menu.x, top: menu.y }}
              onPointerDown={(e) => e.stopPropagation()}
            >
              {(["INNER", "LEFT", "RIGHT", "FULL"] as JoinType[]).map((type) => (
                <button
                  key={type}
                  className="flex w-full items-center gap-2 rounded-md px-2 py-1 text-left text-2xs hover:bg-hover"
                  onClick={() => (update((s) => ({ ...s, joins: s.joins.map((j) => (j.id === menu.id ? { ...j, type } : j)) })), setMenu(null))}
                >
                  <Check size={10} className={cx(st.joins.find((j) => j.id === menu.id)?.type !== type && "invisible")} />
                  {type === "INNER" ? "Inner join (matching rows)" : type === "LEFT" ? "Left join (keep all left)" : type === "RIGHT" ? "Right join (keep all right)" : "Full join (keep all)"}
                </button>
              ))}
              <button
                className="flex w-full items-center gap-2 rounded-md px-2 py-1 text-left text-2xs text-danger hover:bg-danger/10"
                onClick={() => (update((s) => ({ ...s, joins: s.joins.filter((j) => j.id !== menu.id) })), setMenu(null))}
              >
                <X size={10} /> Remove join
              </button>
            </div>
          )}
          {/* table cards */}
          {st.tables.map((t) => (
            <div
              key={t.id}
              className="absolute z-[5] overflow-hidden rounded-lg border border-line bg-elevated shadow-sm"
              style={{ left: t.x, top: t.y, width: CARD_W }}
            >
              <div
                className="flex cursor-grab items-center gap-1.5 border-b border-line bg-panel px-2.5 active:cursor-grabbing"
                style={{ height: HEAD_H }}
                onPointerDown={(e) => startMove(e, t.id)}
              >
                <span className="rounded bg-accent/12 px-1 font-mono text-[10px] font-semibold text-accent">{t.alias}</span>
                <span className="truncate text-xs font-medium">{t.name}</span>
                <button className="ml-auto rounded p-0.5 text-subtle hover:bg-hover hover:text-fg" onClick={() => update((s) => removeTable(s, t.id))} title="Remove table">
                  <X size={11} />
                </button>
              </div>
              {t.columns.slice(0, MAX_ROWS).map((c) => {
                const on = st.columns.some((x) => x.t === t.id && x.col === c.name);
                return (
                  <div
                    key={c.name}
                    data-qb-t={t.id}
                    data-qb-col={c.name}
                    className={cx("group relative flex items-center gap-1.5 px-2.5 text-[11.5px] hover:bg-hover", on && "bg-accent-soft/60")}
                    style={{ height: ROW_H }}
                  >
                    <span
                      className="absolute top-1/2 -left-0 size-2 -translate-y-1/2 cursor-crosshair rounded-full border border-accent/60 bg-elevated opacity-0 group-hover:opacity-100"
                      onPointerDown={(e) => startJoin(e, t.id, c.name)}
                      title="Drag to another column to join"
                    />
                    <input type="checkbox" checked={on} onChange={() => update((s) => toggleColumn(s, t.id, c.name))} className="size-3 accent-[var(--accent)]" />
                    <span className="truncate font-mono">{c.name}</span>
                    {c.primary_key && <KeyRound size={9} className="shrink-0 text-warning" />}
                    <span className="ml-auto truncate pl-1 text-[10px] text-subtle">{c.data_type}</span>
                    <span
                      className="absolute top-1/2 -right-0 size-2 -translate-y-1/2 cursor-crosshair rounded-full border border-accent/60 bg-elevated opacity-0 group-hover:opacity-100"
                      onPointerDown={(e) => startJoin(e, t.id, c.name)}
                    />
                  </div>
                );
              })}
              {t.columns.length > MAX_ROWS && <div className="px-2.5 py-1 text-[10px] text-subtle">+{t.columns.length - MAX_ROWS} more columns</div>}
            </div>
          ))}
        </div>
      </div>

      {/* criteria + sql */}
      <div className="flex h-60 shrink-0 border-t border-line">
        <div className="flex min-w-0 flex-1 flex-col gap-3 overflow-y-auto p-3 text-xs">
          <section>
            <h4 className="mb-1 text-2xs font-medium tracking-wide text-subtle uppercase">Columns</h4>
            {st.columns.length === 0 && <p className="text-2xs text-subtle">All columns · tick columns on the cards to pick</p>}
            {st.columns.map((c, i) => (
              <div key={`${c.t}.${c.col}`} className="mb-1 flex items-center gap-1.5">
                <span className="w-40 truncate font-mono text-[11px]">
                  {byId.get(c.t)?.alias}.{c.col}
                </span>
                <select value={c.agg} onChange={(e) => update((s) => ({ ...s, columns: s.columns.map((x, k) => (k === i ? { ...x, agg: e.target.value as Agg } : x)) }))} className={cx(sel, "w-28")}>
                  {AGGS.map((a) => (
                    <option key={a.value} value={a.value}>
                      {a.label}
                    </option>
                  ))}
                </select>
                <input
                  className={cx(sel, "w-32")}
                  placeholder="alias"
                  value={c.alias}
                  onChange={(e) => update((s) => ({ ...s, columns: s.columns.map((x, k) => (k === i ? { ...x, alias: e.target.value } : x)) }))}
                />
                <button className="rounded p-0.5 text-subtle hover:text-danger" onClick={() => update((s) => toggleColumn(s, c.t, c.col))}>
                  <Trash2 size={11} />
                </button>
              </div>
            ))}
          </section>
          <section>
            <h4 className="mb-1 flex items-center gap-2 text-2xs font-medium tracking-wide text-subtle uppercase">
              Filters
              <button
                className="rounded p-0.5 text-muted hover:bg-hover hover:text-fg disabled:opacity-40"
                disabled={!colOptions.length}
                onClick={() => {
                  const first = split(colOptions[0].key);
                  update((s) => ({ ...s, filters: [...s.filters, { id: Math.random().toString(36).slice(2), ...first, op: "=", value: "" }] }));
                }}
              >
                <Plus size={11} />
              </button>
            </h4>
            {st.filters.map((f) => (
              <div key={f.id} className="mb-1 flex items-center gap-1.5">
                <select
                  value={`${f.t}|${f.col}`}
                  onChange={(e) => update((s) => ({ ...s, filters: s.filters.map((x) => (x.id === f.id ? { ...x, ...split(e.target.value) } : x)) }))}
                  className={cx(sel, "w-40 font-mono")}
                >
                  {colOptions.map((o) => (
                    <option key={o.key} value={o.key}>
                      {o.label}
                    </option>
                  ))}
                </select>
                <select value={f.op} onChange={(e) => update((s) => ({ ...s, filters: s.filters.map((x) => (x.id === f.id ? { ...x, op: e.target.value as QbOp } : x)) }))} className={cx(sel, "w-28")}>
                  {OPS.map((o) => (
                    <option key={o.value} value={o.value}>
                      {o.label}
                    </option>
                  ))}
                </select>
                {f.op !== "is null" && f.op !== "is not null" && (
                  <input
                    className={cx(sel, "w-44")}
                    placeholder={f.op === "in" ? "a, b, c" : "value"}
                    value={f.value}
                    onChange={(e) => update((s) => ({ ...s, filters: s.filters.map((x) => (x.id === f.id ? { ...x, value: e.target.value } : x)) }))}
                  />
                )}
                <button className="rounded p-0.5 text-subtle hover:text-danger" onClick={() => update((s) => ({ ...s, filters: s.filters.filter((x) => x.id !== f.id) }))}>
                  <Trash2 size={11} />
                </button>
              </div>
            ))}
          </section>
          <section>
            <h4 className="mb-1 flex items-center gap-2 text-2xs font-medium tracking-wide text-subtle uppercase">
              Sort
              <button
                className="rounded p-0.5 text-muted hover:bg-hover hover:text-fg disabled:opacity-40"
                disabled={!colOptions.length}
                onClick={() => update((s) => ({ ...s, sorts: [...s.sorts, { ...split(colOptions[0].key), desc: false }] }))}
              >
                <Plus size={11} />
              </button>
            </h4>
            {st.sorts.map((o, i) => (
              <div key={i} className="mb-1 flex items-center gap-1.5">
                <select
                  value={`${o.t}|${o.col}`}
                  onChange={(e) => update((s) => ({ ...s, sorts: s.sorts.map((x, k) => (k === i ? { ...x, ...split(e.target.value) } : x)) }))}
                  className={cx(sel, "w-40 font-mono")}
                >
                  {colOptions.map((c) => (
                    <option key={c.key} value={c.key}>
                      {c.label}
                    </option>
                  ))}
                </select>
                <Segmented
                  value={o.desc ? "desc" : "asc"}
                  onChange={(v) => update((s) => ({ ...s, sorts: s.sorts.map((x, k) => (k === i ? { ...x, desc: v === "desc" } : x)) }))}
                  options={[
                    { value: "asc", label: <ArrowUp size={10} /> },
                    { value: "desc", label: <ArrowDown size={10} /> },
                  ]}
                />
                <button className="rounded p-0.5 text-subtle hover:text-danger" onClick={() => update((s) => ({ ...s, sorts: s.sorts.filter((_, k) => k !== i) }))}>
                  <Trash2 size={11} />
                </button>
              </div>
            ))}
          </section>
        </div>
        {showSql && (
          <div className="flex w-[42%] min-w-72 flex-col border-l border-line">
            <pre className="selectable min-h-0 flex-1 overflow-auto bg-panel p-3 font-mono text-[11.5px] leading-relaxed text-fg">
              {built.sql || <span className="text-subtle">-- SQL appears here as you build</span>}
            </pre>
            {built.warnings.length > 0 && (
              <div className="border-t border-line bg-warning/10 px-3 py-1.5 text-2xs text-warning">{built.warnings.join(" · ")}</div>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
