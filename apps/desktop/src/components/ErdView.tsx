import { Download, KeyRound, Link2, Maximize2, Minus, Plus, RefreshCw, Search } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api, dialogs, type Erd, type ErdTable } from "../lib/api";
import { Button, cx } from "./ui";

const W = 220; // table box width
const HEAD = 30;
const ROW = 22;
const MAX_COLS = 16;
const GAP_X = 90;
const GAP_Y = 28;

type Pos = { x: number; y: number };
const key = (t: { schema: string; name: string }) => `${t.schema}.${t.name}`;
const shownCols = (t: ErdTable) => Math.min(t.columns.length, MAX_COLS) + (t.columns.length > MAX_COLS ? 1 : 0);
const height = (t: ErdTable) => HEAD + shownCols(t) * ROW + 6;

/** Layered layout: referenced ("parent") tables to the left of the tables that point at them. */
function layout(erd: Erd): Record<string, Pos> {
  const refs = new Map<string, Set<string>>();
  for (const fk of erd.foreign_keys) {
    const from = `${fk.schema}.${fk.table}`, to = `${fk.ref_schema}.${fk.ref_table}`;
    if (from === to) continue;
    if (!refs.has(from)) refs.set(from, new Set());
    refs.get(from)!.add(to);
  }
  const linked = new Set<string>();
  erd.foreign_keys.forEach((fk) => (linked.add(`${fk.schema}.${fk.table}`), linked.add(`${fk.ref_schema}.${fk.ref_table}`)));

  const rank = new Map<string, number>();
  const rankOf = (k: string, depth = 0): number => {
    if (rank.has(k)) return rank.get(k)!;
    if (depth > 30) return 0; // cycles
    const parents = [...(refs.get(k) ?? [])];
    const r = parents.length ? 1 + Math.max(...parents.map((p) => rankOf(p, depth + 1))) : 0;
    rank.set(k, r);
    return r;
  };

  const pos: Record<string, Pos> = {};
  const cols = new Map<number, ErdTable[]>();
  const loners: ErdTable[] = [];
  for (const t of erd.tables) {
    if (!linked.has(key(t))) {
      loners.push(t);
      continue;
    }
    const r = rankOf(key(t));
    if (!cols.has(r)) cols.set(r, []);
    cols.get(r)!.push(t);
  }
  let maxY = 0;
  [...cols.keys()]
    .sort((a, b) => a - b)
    .forEach((r, ci) => {
      let y = 0;
      // most-referenced first keeps edges short
      const deg = (t: ErdTable) => erd.foreign_keys.filter((f) => f.ref_table === t.name || f.table === t.name).length;
      for (const t of cols.get(r)!.sort((a, b) => deg(b) - deg(a))) {
        pos[key(t)] = { x: ci * (W + GAP_X), y };
        y += height(t) + GAP_Y;
      }
      maxY = Math.max(maxY, y);
    });
  // tables without relations: a grid underneath
  const perRow = Math.max(3, cols.size);
  let rowY = maxY + (maxY ? 40 : 0);
  for (let i = 0; i < loners.length; i += perRow) {
    const slice = loners.slice(i, i + perRow);
    slice.forEach((t, j) => (pos[key(t)] = { x: j * (W + GAP_X), y: rowY }));
    rowY += Math.max(...slice.map(height)) + GAP_Y;
  }
  return pos;
}

function colIndex(t: ErdTable, name: string) {
  const i = t.columns.findIndex((c) => c.name === name);
  return i < 0 || i >= MAX_COLS ? 0 : i;
}

export function ErdView({
  connId,
  schemas,
  onOpenTable,
  onToast,
}: {
  connId: string;
  schemas: string[];
  onOpenTable: (schema: string, table: string) => void;
  onToast: (msg: string, kind?: "ok" | "err") => void;
}) {
  const [schema, setSchema] = useState<string>(schemas.includes("public") ? "public" : (schemas[0] ?? ""));
  const [erd, setErd] = useState<Erd | null>(null);
  const [loading, setLoading] = useState(false);
  const [pos, setPos] = useState<Record<string, Pos>>({});
  const [view, setView] = useState({ x: 24, y: 24, k: 1 });
  const [focus, setFocus] = useState<string | null>(null);
  const [q, setQ] = useState("");
  const box = useRef<HTMLDivElement>(null);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const e = await api.erd(connId, schemas.length > 1 ? schema : undefined);
      setErd(e);
      setPos(layout(e));
      setFocus(null);
      if (e.truncated) onToast("Large schema: showing the first 250 tables");
    } catch (err) {
      onToast(String(err));
    } finally {
      setLoading(false);
    }
  }, [connId, schema, schemas.length, onToast]);

  useEffect(() => {
    load();
  }, [load]);

  const byKey = useMemo(() => new Map((erd?.tables ?? []).map((t) => [key(t), t])), [erd]);
  const neighbours = useMemo(() => {
    if (!focus || !erd) return null;
    const n = new Set([focus]);
    for (const fk of erd.foreign_keys) {
      const a = `${fk.schema}.${fk.table}`, b = `${fk.ref_schema}.${fk.ref_table}`;
      if (a === focus) n.add(b);
      if (b === focus) n.add(a);
    }
    return n;
  }, [focus, erd]);

  const fit = useCallback(() => {
    const el = box.current;
    const ps = Object.entries(pos);
    if (!el || !ps.length || !erd) return;
    const xs = ps.map(([, p]) => p.x), ys = ps.map(([k, p]) => p.y + height(byKey.get(k)!));
    const w = Math.max(...xs) + W, h = Math.max(...ys);
    const k = Math.min(1.2, Math.max(0.25, Math.min((el.clientWidth - 48) / w, (el.clientHeight - 48) / h)));
    setView({ x: 24, y: 24, k });
  }, [pos, erd, byKey]);

  useEffect(() => {
    fit();
  }, [erd]); // eslint-disable-line react-hooks/exhaustive-deps

  // search → focus first match and centre it
  useEffect(() => {
    if (!q || !erd) return;
    const t = erd.tables.find((t) => t.name.toLowerCase().includes(q.toLowerCase()));
    if (!t || !box.current) return;
    const p = pos[key(t)];
    setFocus(key(t));
    setView((v) => ({ ...v, x: box.current!.clientWidth / 2 - (p.x + W / 2) * v.k, y: 80 - p.y * v.k }));
  }, [q]); // eslint-disable-line react-hooks/exhaustive-deps

  // ---- pan / zoom / drag ---------------------------------------------------
  const onWheel = (e: React.WheelEvent) => {
    if (e.ctrlKey || e.metaKey) {
      const rect = box.current!.getBoundingClientRect();
      const mx = e.clientX - rect.left, my = e.clientY - rect.top;
      setView((v) => {
        const k = Math.min(2, Math.max(0.2, v.k * Math.exp(-e.deltaY * 0.01)));
        return { k, x: mx - ((mx - v.x) * k) / v.k, y: my - ((my - v.y) * k) / v.k };
      });
    } else {
      setView((v) => ({ ...v, x: v.x - e.deltaX, y: v.y - e.deltaY }));
    }
  };

  const drag = (e: React.MouseEvent, table?: string) => {
    if (e.button !== 0) return;
    e.preventDefault();
    const x0 = e.clientX, y0 = e.clientY;
    const start = table ? pos[table] : { x: view.x, y: view.y };
    let moved = false;
    const move = (ev: MouseEvent) => {
      const dx = ev.clientX - x0, dy = ev.clientY - y0;
      if (Math.abs(dx) + Math.abs(dy) > 3) moved = true;
      if (table) setPos((p) => ({ ...p, [table]: { x: start.x + dx / view.k, y: start.y + dy / view.k } }));
      else setView((v) => ({ ...v, x: start.x + dx, y: start.y + dy }));
    };
    const up = () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
      if (!moved) setFocus(table ?? null);
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  };

  // ---- edges --------------------------------------------------------------
  const edges = useMemo(() => {
    if (!erd) return [];
    return erd.foreign_keys.flatMap((fk, i) => {
      const a = byKey.get(`${fk.schema}.${fk.table}`), b = byKey.get(`${fk.ref_schema}.${fk.ref_table}`);
      const pa = a && pos[key(a)], pb = b && pos[key(b)];
      if (!a || !b || !pa || !pb) return [];
      const ya = pa.y + HEAD + colIndex(a, fk.columns[0]) * ROW + ROW / 2;
      const yb = pb.y + HEAD + colIndex(b, fk.ref_columns[0] ?? "") * ROW + ROW / 2;
      // leave from the side facing the other table
      const aLeft = pa.x > pb.x + W / 2;
      const xa = aLeft ? pa.x : pa.x + W, xb = aLeft ? pb.x + W : pb.x;
      const bend = Math.max(40, Math.abs(xa - xb) / 2);
      const d =
        a === b
          ? `M ${pa.x + W} ${ya} C ${pa.x + W + 50} ${ya}, ${pa.x + W + 50} ${yb}, ${pa.x + W} ${yb}`
          : `M ${xa} ${ya} C ${xa + (aLeft ? -bend : bend)} ${ya}, ${xb + (aLeft ? bend : -bend)} ${yb}, ${xb} ${yb}`;
      const on = !neighbours || (neighbours.has(key(a)) && neighbours.has(key(b)) && (key(a) === focus || key(b) === focus));
      return [{ id: `${fk.name}-${i}`, d, on, xb, yb, title: `${fk.table}(${fk.columns.join(", ")}) → ${fk.ref_table}(${fk.ref_columns.join(", ")})` }];
    });
  }, [erd, pos, byKey, neighbours, focus]);

  const exportSvg = async () => {
    if (!erd) return;
    const ps = Object.values(pos);
    const w = Math.max(...ps.map((p) => p.x)) + W + 40;
    const h = Math.max(...erd.tables.map((t) => (pos[key(t)]?.y ?? 0) + height(t))) + 40;
    const esc = (s: string) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;");
    const parts = [
      `<svg xmlns="http://www.w3.org/2000/svg" width="${w}" height="${h}" font-family="ui-monospace, Menlo, monospace" font-size="11">`,
      `<rect width="100%" height="100%" fill="#fbfbfa"/><g transform="translate(20 20)">`,
      ...edges.map((e) => `<path d="${e.d}" fill="none" stroke="#818cf8" stroke-width="1.4"/>`),
      ...erd.tables.map((t) => {
        const p = pos[key(t)];
        const rows = t.columns.slice(0, MAX_COLS).map(
          (c, i) =>
            `<text x="${p.x + 10}" y="${p.y + HEAD + i * ROW + 15}" fill="#18181b">${c.primary_key ? "PK " : ""}${esc(c.name)}</text>` +
            `<text x="${p.x + W - 10}" y="${p.y + HEAD + i * ROW + 15}" fill="#9c9ca1" text-anchor="end">${esc(c.data_type.slice(0, 18))}</text>`,
        );
        return `<rect x="${p.x}" y="${p.y}" width="${W}" height="${height(t)}" rx="8" fill="#fff" stroke="#d4d4d0"/>
<rect x="${p.x}" y="${p.y}" width="${W}" height="${HEAD}" rx="8" fill="#f4f4f2"/>
<text x="${p.x + 10}" y="${p.y + 19}" font-weight="600" font-family="system-ui, sans-serif" font-size="12">${esc(t.name)}</text>${rows.join("")}`;
      }),
      `</g></svg>`,
    ];
    const path = await dialogs.save(`erd-${schema || "schema"}.svg`, "svg");
    if (!path) return;
    try {
      await api.writeTextFile(path, parts.join("\n"));
      onToast(`Saved ${path.split(/[\\/]/).pop()}`, "ok");
    } catch (e) {
      onToast(String(e));
    }
  };

  const fkCols = useMemo(() => {
    const s = new Set<string>();
    erd?.foreign_keys.forEach((f) => f.columns.forEach((c) => s.add(`${f.schema}.${f.table}.${c}`)));
    return s;
  }, [erd]);

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex h-9 shrink-0 items-center gap-2 border-b border-line px-2">
        {schemas.length > 1 && (
          <select
            value={schema}
            onChange={(e) => setSchema(e.target.value)}
            className="h-6 rounded-md border border-line bg-elevated px-1.5 text-xs outline-none"
          >
            {schemas.map((s) => (
              <option key={s}>{s}</option>
            ))}
          </select>
        )}
        <div className="relative">
          <Search size={12} className="pointer-events-none absolute top-1/2 left-2 -translate-y-1/2 text-subtle" />
          <input
            value={q}
            onChange={(e) => setQ(e.target.value)}
            placeholder="Find table"
            spellCheck={false}
            className="h-6 w-40 rounded-md border border-line bg-elevated pr-2 pl-6.5 text-xs outline-none focus:border-accent"
          />
        </div>
        <span className="truncate text-2xs whitespace-nowrap text-subtle">
          {erd ? `${erd.tables.length} tables · ${erd.foreign_keys.length} relations` : "Loading…"}
        </span>
        <span className="flex-1" />
        <span className="hidden truncate text-2xs whitespace-nowrap text-subtle @5xl:inline">Scroll to pan · ⌘/Ctrl+scroll to zoom · click to focus · double-click to open</span>
        <Button variant="ghost" size="icon" onClick={() => setView((v) => ({ ...v, k: Math.max(0.2, v.k / 1.2) }))} title="Zoom out">
          <Minus size={13} />
        </Button>
        <span className="w-9 text-center text-2xs text-muted tabular-nums">{Math.round(view.k * 100)}%</span>
        <Button variant="ghost" size="icon" onClick={() => setView((v) => ({ ...v, k: Math.min(2, v.k * 1.2) }))} title="Zoom in">
          <Plus size={13} />
        </Button>
        <Button variant="ghost" size="icon" onClick={fit} title="Fit">
          <Maximize2 size={12} />
        </Button>
        <Button variant="ghost" size="xs" onClick={load} disabled={loading}>
          <RefreshCw size={11} className={cx(loading && "animate-spin")} />
        </Button>
        <Button variant="ghost" size="xs" onClick={exportSvg} disabled={!erd}>
          <Download size={11} /> SVG
        </Button>
      </div>

      <div
        ref={box}
        onWheel={onWheel}
        onMouseDown={(e) => drag(e)}
        className="relative min-h-0 flex-1 cursor-grab overflow-hidden active:cursor-grabbing"
        style={{
          backgroundImage: "radial-gradient(var(--border) 1px, transparent 1px)",
          backgroundSize: `${18 * view.k}px ${18 * view.k}px`,
          backgroundPosition: `${view.x}px ${view.y}px`,
        }}
      >
        {erd && erd.tables.length === 0 && (
          <div className="absolute inset-0 flex items-center justify-center text-xs text-subtle">No tables in this schema</div>
        )}
        <div className="absolute top-0 left-0 origin-top-left" style={{ transform: `translate(${view.x}px, ${view.y}px) scale(${view.k})` }}>
          <svg className="pointer-events-none absolute top-0 left-0 overflow-visible" width="1" height="1">
            <defs>
              <marker id="rh-fk" viewBox="0 0 8 8" refX="7" refY="4" markerWidth="7" markerHeight="7" orient="auto-start-reverse">
                <path d="M0,0 L8,4 L0,8 z" fill="var(--accent)" />
              </marker>
            </defs>
            {edges.map((e) => (
              <path
                key={e.id}
                d={e.d}
                fill="none"
                stroke="var(--accent)"
                strokeOpacity={e.on ? 0.75 : 0.1}
                strokeWidth={e.on && focus ? 2 : 1.3}
                markerEnd="url(#rh-fk)"
              >
                <title>{e.title}</title>
              </path>
            ))}
          </svg>
          {erd?.tables.map((t) => {
            const k = key(t);
            const p = pos[k];
            if (!p) return null;
            const dim = neighbours && !neighbours.has(k);
            return (
              <div
                key={k}
                onMouseDown={(e) => (e.stopPropagation(), drag(e, k))}
                onDoubleClick={() => onOpenTable(t.schema, t.name)}
                className={cx(
                  "absolute overflow-hidden rounded-lg border bg-elevated shadow-[0_1px_2px_rgb(0_0_0/0.06)] transition-opacity duration-150 select-none",
                  focus === k ? "border-accent ring-2 ring-accent-soft" : "border-line-strong",
                  dim && "opacity-25",
                )}
                style={{ left: p.x, top: p.y, width: W }}
              >
                <div className="flex h-[30px] cursor-move items-center gap-1.5 border-b border-line bg-panel px-2.5">
                  <span className="truncate text-xs font-semibold">{t.name}</span>
                  {t.kind === "view" && <span className="text-[10px] text-subtle">view</span>}
                  <span className="ml-auto text-[10px] text-subtle">{t.columns.length}</span>
                </div>
                <div className="py-[3px]">
                  {t.columns.slice(0, MAX_COLS).map((c) => (
                    <div key={c.name} className="flex h-[22px] items-center gap-1.5 px-2.5 font-mono text-[11px]">
                      <span className="flex w-3 shrink-0 justify-center">
                        {c.primary_key ? (
                          <KeyRound size={10} className="text-warning" />
                        ) : fkCols.has(`${k}.${c.name}`) ? (
                          <Link2 size={10} className="text-accent" />
                        ) : null}
                      </span>
                      <span className={cx("truncate", c.primary_key && "font-semibold")}>{c.name}</span>
                      <span className="ml-auto truncate pl-2 text-[10px] text-subtle">{c.data_type}</span>
                    </div>
                  ))}
                  {t.columns.length > MAX_COLS && (
                    <div className="h-[22px] px-2.5 text-[10.5px] leading-[22px] text-subtle">+{t.columns.length - MAX_COLS} more</div>
                  )}
                </div>
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}
