import { AlertCircle, BarChart3, Copy, Download, GripVertical, Hash, LayoutGrid, MoreHorizontal, Pencil, Plus, RefreshCw, Send, SquareTerminal, Table2, Trash2, X } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api, dialogs, kindInfo, type ConnectionConfig, type Dashboard, type DashTile, type Delivery, type DeliveryKind, type QueryResult, type Snippet } from "../lib/api";
import { uid } from "../lib/types";
import { findVars, substitute, type VarRef } from "../lib/vars";
import { display } from "./ResultsGrid";
import { columnKind, connMasks, maskCell } from "../lib/mask";
import { useAppearance } from "../lib/appearance";
import { ResultsChart, defaultSpec, type ChartSpec } from "./ResultsChart";
import { ContextMenu } from "./TableActions";
import { Button, Input, Menu, Modal, Segmented, cx } from "./ui";

const ROW_H = 168;
const WIDTHS: { w: number; label: string }[] = [
  { w: 3, label: "¼" },
  { w: 4, label: "⅓" },
  { w: 6, label: "½" },
  { w: 8, label: "⅔" },
  { w: 12, label: "Full" },
];

type TileState = { result?: QueryResult; error?: string; loading: boolean; at?: number };

function ago(t?: number) {
  if (!t) return "";
  const s = Math.round((Date.now() - t) / 1000);
  return s < 5 ? "just now" : s < 60 ? `${s}s ago` : `${Math.floor(s / 60)}m ago`;
}

/** A grid of read-only query tiles: charts, tables and single numbers. */
export function DashboardView({
  dashboardId,
  conn,
  onOpenQuery,
  onToast,
  onDeleted,
  headless = false,
  onSnapshotReady,
}: {
  /** render for a scheduled snapshot: no toolbar, report when every tile has loaded */
  headless?: boolean;
  onSnapshotReady?: (node: HTMLElement, dash: Dashboard) => void;
  dashboardId: string;
  conn: ConnectionConfig | undefined;
  onOpenQuery: (sql: string) => void;
  onToast: (m: string, k?: "ok" | "err") => void;
  onDeleted: () => void;
}) {
  const [dash, setDash] = useState<Dashboard | null>(null);
  const [states, setStates] = useState<Record<string, TileState>>({});
  const [snippets, setSnippets] = useState<Snippet[]>([]);
  const [editing, setEditing] = useState<DashTile | "new" | null>(null);
  const [menu, setMenu] = useState<{ x: number; y: number; tile: DashTile } | null>(null);
  const [dragId, setDragId] = useState<string | null>(null);
  const [rename, setRename] = useState<string | null>(null);
  const [confirmDel, setConfirmDel] = useState(false);
  const [exporting, setExporting] = useState(false);
  const shot = useRef<HTMLDivElement>(null);
  const exportPng = async (mode: "save" | "copy") => {
    if (!shot.current || !dash) return;
    setExporting(true);
    await new Promise((r) => setTimeout(r, 60));
    try {
      const { toBlob } = await import("html-to-image");
      const blob = await toBlob(shot.current, {
        pixelRatio: 2,
        backgroundColor: getComputedStyle(document.body).backgroundColor,
        filter: (n) => !(n instanceof HTMLElement && n.dataset.noExport !== undefined),
      });
      if (!blob) throw "couldn't render the dashboard";
      if (mode === "copy") {
        await navigator.clipboard.write([new ClipboardItem({ "image/png": blob })]);
        onToast("Dashboard copied as PNG", "ok");
      } else {
        const path = await dialogs.save(`${dash.name.replace(/[^\w.-]+/g, "-")}.png`, "png");
        if (path) {
          const buf = new Uint8Array(await blob.arrayBuffer());
          let bin = "";
          for (let i = 0; i < buf.length; i += 0x8000) bin += String.fromCharCode(...buf.subarray(i, i + 0x8000));
          await api.writeBase64File(path, btoa(bin));
          onToast(`Saved ${path.split(/[\\/]/).pop()}`, "ok");
        }
      }
    } catch (e) {
      onToast(String(e));
    } finally {
      setExporting(false);
    }
  };
  const [, tick] = useState(0);
  const closeMenu = useCallback(() => setMenu(null), []);

  useEffect(() => {
    api.dashboards
      .list()
      .then((l) => setDash(l.find((d) => d.id === dashboardId) ?? null))
      .catch((e) => onToast(String(e)));
  }, [dashboardId, onToast]);
  useEffect(() => {
    if (dash) api.listSnippets(dash.connectionId).then(setSnippets).catch(() => {});
  }, [dash?.connectionId]);

  const dialect = conn ? kindInfo(conn.kind).dialect : "postgres";
  const filtersRef = useRef<Record<string, string>>({});
  filtersRef.current = dash?.filters ?? {};
  const rawSql = useCallback((t: DashTile) => (t.snippetId != null ? (snippets.find((s) => s.id === t.snippetId)?.sql ?? t.sql) : t.sql), [snippets]);
  const sqlOf = useCallback((t: DashTile) => substitute(rawSql(t), filtersRef.current, dialect), [rawSql, dialect]);
  const vars = useMemo(() => {
    const seen = new Map<string, VarRef>();
    for (const t of dash?.tiles ?? []) for (const v of findVars(rawSql(t), dialect)) if (!seen.has(v.name)) seen.set(v.name, v);
    return [...seen.values()];
  }, [dash?.tiles, rawSql, dialect]);

  const runTile = useCallback(
    async (t: DashTile) => {
      if (!dash) return;
      setStates((m) => ({ ...m, [t.id]: { ...m[t.id], loading: true } }));
      try {
        const out = await api.dashboards.query(dash.connectionId, sqlOf(t));
        const result = [...out.results].reverse().find((r) => r.columns.length > 0) ?? out.results[0];
        setStates((m) => ({ ...m, [t.id]: { result, loading: false, at: Date.now() } }));
      } catch (e) {
        setStates((m) => ({ ...m, [t.id]: { error: String(e), loading: false, at: Date.now() } }));
      }
    },
    [dash, sqlOf],
  );
  const runAll = useCallback(() => dash?.tiles.forEach(runTile), [dash, runTile]);
  const loadedFor = useRef<string | null>(null);
  useEffect(() => {
    if (!dash || loadedFor.current === dash.id) return;
    loadedFor.current = dash.id;
    runAll();
  }, [dash, runAll]);
  useEffect(() => {
    if (!dash?.refreshSecs) return;
    const t = setInterval(() => document.visibilityState === "visible" && runAll(), dash.refreshSecs * 1000);
    return () => clearInterval(t);
  }, [dash?.refreshSecs, runAll]);
  useEffect(() => {
    const t = setInterval(() => tick((x) => x + 1), 15000);
    return () => clearInterval(t);
  }, []);

  // headless snapshot: hand the grid over once every tile has finished
  const reported = useRef(false);
  useEffect(() => {
    if (!headless || !dash || reported.current || !shot.current) return;
    const done = dash.tiles.every((t) => states[t.id] && !states[t.id].loading);
    if (!done) return;
    reported.current = true;
    setTimeout(() => shot.current && onSnapshotReady?.(shot.current, dash), 400);
  }, [headless, dash, states, onSnapshotReady]);

  const setFilter = (name: string, value: string) => {
    if (!dash) return;
    const next = { ...dash, filters: { ...(dash.filters ?? {}), [name]: value } };
    filtersRef.current = next.filters;
    setDash(next);
    clearTimeout(filterTimer.current);
    filterTimer.current = setTimeout(() => {
      api.dashboards.save(next).catch((e) => onToast(String(e)));
      next.tiles.filter((t) => findVars(rawSql(t), dialect).some((v) => v.name === name)).forEach(runTile);
    }, 450);
  };
  const filterTimer = useRef<ReturnType<typeof setTimeout>>(undefined);
  const [snapOpen, setSnapOpen] = useState(false);

  const persist = async (d: Dashboard) => {
    setDash(d);
    try {
      setDash(await api.dashboards.save(d));
    } catch (e) {
      onToast(String(e));
    }
  };
  const patchTile = (id: string, p: Partial<DashTile>) => dash && persist({ ...dash, tiles: dash.tiles.map((t) => (t.id === id ? { ...t, ...p } : t)) });
  const moveTile = (from: string, to: string) => {
    if (!dash || from === to) return;
    const tiles = [...dash.tiles];
    const a = tiles.findIndex((t) => t.id === from);
    const b = tiles.findIndex((t) => t.id === to);
    const [x] = tiles.splice(a, 1);
    tiles.splice(b, 0, x);
    persist({ ...dash, tiles });
  };

  if (!dash) return <div className="flex h-full items-center justify-center text-xs text-subtle">Loading dashboard…</div>;

  const lastAt = Math.max(0, ...Object.values(states).map((s) => s.at ?? 0));
  const anyLoading = Object.values(states).some((s) => s.loading);

  return (
    <div className="rh-print flex h-full min-h-0 flex-1 flex-col bg-bg">
      {!headless && <div className="flex h-10 shrink-0 items-center gap-2 border-b border-line px-3 print:hidden">
        <LayoutGrid size={14} className="text-accent" />
        {rename !== null ? (
          <input
            autoFocus
            value={rename}
            onChange={(e) => setRename(e.target.value)}
            onBlur={() => (rename.trim() && persist({ ...dash, name: rename.trim() }), setRename(null))}
            onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()}
            className="h-7 rounded-md border border-accent bg-elevated px-2 text-sm font-semibold outline-none"
          />
        ) : (
          <button onDoubleClick={() => setRename(dash.name)} title="Double-click to rename" className="text-sm font-semibold">
            {dash.name}
          </button>
        )}
        <span className="text-2xs text-subtle">
          {conn?.name ?? "connection missing"} · {dash.tiles.length} tile{dash.tiles.length === 1 ? "" : "s"}
          {lastAt ? ` · updated ${ago(lastAt)}` : ""}
        </span>
        <span className="flex-1" />
        <select
          value={dash.refreshSecs}
          onChange={(e) => persist({ ...dash, refreshSecs: Number(e.target.value) })}
          title="Auto refresh"
          className="h-7 rounded-md border border-line bg-elevated px-1.5 text-2xs text-fg outline-none"
        >
          {[0, 60, 300, 900, 3600].map((s) => (
            <option key={s} value={s}>
              {s === 0 ? "Auto refresh: off" : `Every ${s < 3600 ? `${s / 60} min` : "hour"}`}
            </option>
          ))}
        </select>
        <Menu
          label={
            <>
              <Download size={12} /> Export
            </>
          }
          disabled={dash.tiles.length === 0}
          items={[
            { label: "Save as PNG…", onSelect: () => exportPng("save") },
            { label: "Copy as PNG", onSelect: () => exportPng("copy") },
            { label: "Print / save as PDF…", onSelect: () => window.print() },
            { label: dash.snapshot?.enabled ? "Snapshot schedule (on)…" : "Schedule snapshots…", onSelect: () => setSnapOpen(true) },
          ]}
        />
        <Button size="xs" variant="ghost" onClick={runAll} title="Refresh all tiles">
          <RefreshCw size={12} className={cx(anyLoading && "animate-spin")} /> Refresh
        </Button>
        <Button size="xs" variant="primary" onClick={() => setEditing("new")}>
          <Plus size={12} /> Add tile
        </Button>
        {confirmDel ? (
          <span className="flex items-center gap-1 text-2xs">
            <span className="text-danger">Delete dashboard?</span>
            <Button
              size="xs"
              variant="danger"
              onClick={async () => {
                await api.dashboards.remove(dash.id);
                onDeleted();
              }}
            >
              Delete
            </Button>
            <Button size="xs" variant="ghost" onClick={() => setConfirmDel(false)}>
              Keep
            </Button>
          </span>
        ) : (
          <Button size="icon" variant="ghost" title="Delete dashboard" onClick={() => setConfirmDel(true)}>
            <Trash2 size={13} />
          </Button>
        )}
      </div>}

      {vars.length > 0 && (
        <div className="flex shrink-0 flex-wrap items-center gap-x-3 gap-y-1.5 border-b border-line bg-panel/40 px-3 py-1.5 print:hidden" data-no-export>
          {vars.map((v) => (
            <FilterInput key={v.name} v={v} value={dash.filters?.[v.name] ?? ""} onChange={(x) => setFilter(v.name, x)} />
          ))}
        </div>
      )}

      <div className="min-h-0 flex-1 overflow-y-auto p-3 print:overflow-visible">
        <div ref={shot} className="bg-bg">
        <div className={cx("mb-3 flex-wrap items-baseline gap-2 px-1", exporting || headless ? "flex" : "hidden print:flex")}>
          <span className="text-base font-semibold">{dash.name}</span>
          <span className="text-2xs text-muted">
            {conn?.name ?? ""} · {new Date().toLocaleString()}
            {vars.map((v) => ` · ${v.name} = ${dash.filters?.[v.name] || "—"}`).join("")}
          </span>
        </div>
        {dash.tiles.length === 0 ? (
          <div className="mx-auto mt-16 flex max-w-md flex-col items-center gap-3 rounded-xl border border-dashed border-line p-8 text-center text-xs text-muted">
            <LayoutGrid size={22} className="text-subtle" />
            Add tiles from your saved queries, or run a query, open <b>Chart</b> and use <b>Pin to dashboard</b>.
            <Button variant="primary" onClick={() => setEditing("new")}>
              <Plus size={12} /> Add tile
            </Button>
          </div>
        ) : (
          <div className="grid grid-cols-12 gap-3" style={{ gridAutoRows: ROW_H }}>
            {dash.tiles.map((t) => {
              const st = states[t.id];
              return (
                <div
                  key={t.id}
                  onDragOver={(e) => dragId && (e.preventDefault(), (e.dataTransfer.dropEffect = "move"))}
                  onDrop={() => dragId && moveTile(dragId, t.id)}
                  className={cx("group flex min-w-0 flex-col overflow-hidden rounded-xl border border-line bg-panel/50", dragId === t.id && "opacity-50")}
                  style={{ gridColumn: `span ${Math.min(12, t.w)}`, gridRow: `span ${t.h}` }}
                >
                  <div className="flex h-8 shrink-0 items-center gap-1.5 pr-1 pl-1">
                    <span
                      draggable
                      onDragStart={() => setDragId(t.id)}
                      onDragEnd={() => setDragId(null)}
                      data-no-export
                      className="cursor-grab rounded p-0.5 text-subtle opacity-0 group-hover:opacity-100 active:cursor-grabbing print:hidden"
                      title="Drag to reorder"
                    >
                      <GripVertical size={12} />
                    </span>
                    <span className="truncate text-xs font-medium">{t.title}</span>
                    {t.snippetId != null && <span className="shrink-0 rounded bg-hover px-1 text-[9.5px] text-subtle">saved query</span>}
                    <span className="flex-1" />
                    {st?.loading && <RefreshCw size={11} className="animate-spin text-subtle" />}
                    <button
                      data-no-export
                      onClick={(e) => setMenu({ x: e.clientX, y: e.clientY, tile: t })}
                      className="print:hidden rounded p-0.5 text-subtle opacity-0 group-hover:opacity-100 hover:bg-hover hover:text-fg"
                    >
                      <MoreHorizontal size={14} />
                    </button>
                  </div>
                  <div className="relative min-h-0 flex-1">
                    <TileBody tile={t} state={st} onToast={onToast} mask={connMasks(conn)} />
                  </div>
                </div>
              );
            })}
          </div>
        )}
        </div>
      </div>

      {menu && (
        <ContextMenu
          x={menu.x}
          y={menu.y}
          onClose={closeMenu}
          items={[
            { label: <><Pencil size={12} className="text-subtle" /> Edit</>, onSelect: () => setEditing(menu.tile) },
            { label: <><RefreshCw size={12} className="text-subtle" /> Refresh</>, onSelect: () => runTile(menu.tile) },
            { label: <><SquareTerminal size={12} className="text-subtle" /> Open in query tab</>, onSelect: () => onOpenQuery(sqlOf(menu.tile)) },
            { label: <><Copy size={12} className="text-subtle" /> Duplicate</>, onSelect: () => persist({ ...dash, tiles: [...dash.tiles, { ...menu.tile, id: uid(), title: `${menu.tile.title} (copy)` }] }) },
            "separator",
            ...WIDTHS.map((x) => ({ label: `Width ${x.label}`, hint: menu.tile.w === x.w ? "✓" : undefined, onSelect: () => patchTile(menu.tile.id, { w: x.w }) })),
            { label: menu.tile.h > 1 ? "Shorter" : "Taller", onSelect: () => patchTile(menu.tile.id, { h: menu.tile.h > 1 ? 1 : 2 }) },
            "separator",
            { label: <><Trash2 size={12} /> Remove</>, danger: true, onSelect: () => persist({ ...dash, tiles: dash.tiles.filter((x) => x.id !== menu.tile.id) }) },
          ]}
        />
      )}

      {snapOpen && (
        <SnapshotDialog
          dash={dash}
          onToast={onToast}
          onClose={() => setSnapOpen(false)}
          onSave={async (snapshot, sendNow) => {
            try {
              const saved = await api.dashboards.save({ ...dash, snapshot });
              setDash(saved);
              setSnapOpen(false);
              if (sendNow) {
                await api.dashboards.snapshotNow(saved.id);
                onToast("Taking a snapshot…", "ok");
              }
            } catch (e) {
              onToast(String(e));
            }
          }}
        />
      )}

      {editing && (
        <TileEditor
          filters={dash.filters ?? {}}
          dialect={dialect}
          connId={dash.connectionId}
          initial={editing === "new" ? null : editing}
          snippets={snippets}
          onClose={() => setEditing(null)}
          onSave={(tile) => {
            const exists = dash.tiles.some((x) => x.id === tile.id);
            persist({ ...dash, tiles: exists ? dash.tiles.map((x) => (x.id === tile.id ? tile : x)) : [...dash.tiles, tile] });
            setEditing(null);
            setTimeout(() => runTile(tile), 0);
          }}
        />
      )}
    </div>
  );
}

function TileBody({ tile, state, onToast, mask }: { tile: DashTile; state?: TileState; onToast: (m: string, k?: "ok" | "err") => void; mask?: boolean }) {
  const masking = !!mask || !!useAppearance().maskAll;
  if (!state || (state.loading && !state.result && !state.error)) return <div className="m-3 h-[calc(100%-24px)] animate-pulse rounded-lg bg-hover/60" />;
  if (state.error)
    return (
      <div className="flex h-full items-start gap-2 overflow-auto p-3 text-2xs text-danger">
        <AlertCircle size={13} className="mt-px shrink-0" />
        <span className="selectable">{state.error}</span>
      </div>
    );
  const r = state.result!;
  if (!r.columns.length) return <div className="p-3 text-2xs text-subtle">No result set</div>;
  if (tile.view === "number") {
    const v = r.rows[0]?.[r.columns.findIndex((_, i) => typeof r.rows[0]?.[i] === "number")] ?? r.rows[0]?.[0];
    const n = typeof v === "number" ? v : Number(v);
    return (
      <div className="flex h-full flex-col justify-center px-4 pb-3">
        <div className="text-[34px] leading-none font-semibold tracking-tight tabular-nums">{Number.isFinite(n) ? n.toLocaleString() : display(v ?? null)}</div>
        <div className="mt-1.5 truncate text-2xs text-muted">{r.columns[0]?.name}</div>
      </div>
    );
  }
  if (tile.view === "chart") {
    return (
      <div className="absolute inset-0">
        <ResultsChart compact result={r} spec={tile.chart ?? undefined} onSpec={() => {}} title={tile.title} onToast={onToast} />
      </div>
    );
  }
  return (
    <div className="absolute inset-0 overflow-auto">
      <table className="w-max min-w-full border-collapse font-mono text-[11.5px]">
        <thead className="sticky top-0 bg-panel text-left text-2xs text-muted">
          <tr>
            {r.columns.map((c, i) => (
              <th key={i} className="border-b border-line px-2.5 py-1 font-sans font-medium">
                {c.name}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {r.rows.slice(0, 200).map((row, i) => (
            <tr key={i} className="border-b border-line/50 last:border-0">
              {row.map((v, j) => (
                <td key={j} className={cx("max-w-64 truncate px-2.5 py-1", v === null && "text-subtle italic", typeof v === "number" && "text-right tabular-nums")}>
                  {masking ? (maskCell(columnKind(r.columns[j]?.name ?? ""), v) ?? display(v)) : display(v)}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

/** Add / edit a tile: pick a saved query or write SQL, preview, choose how to show it. */
function TileEditor({
  connId,
  initial,
  snippets,
  onClose,
  onSave,
  filters,
  dialect,
}: {
  filters: Record<string, string>;
  dialect: import("../lib/api").Dialect;
  connId: string;
  initial: DashTile | null;
  snippets: Snippet[];
  onClose: () => void;
  onSave: (t: DashTile) => void;
}) {
  const [t, setT] = useState<DashTile>(initial ?? { id: uid(), title: "", sql: "", snippetId: null, view: "chart", chart: null, w: 6, h: 1 });
  const [preview, setPreview] = useState<QueryResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const sql = t.snippetId != null ? (snippets.find((s) => s.id === t.snippetId)?.sql ?? t.sql) : t.sql;
  const run = async () => {
    if (!sql.trim()) return;
    setBusy(true);
    setError(null);
    try {
      const out = await api.dashboards.query(connId, substitute(sql, filters, dialect));
      const r = [...out.results].reverse().find((x) => x.columns.length > 0) ?? out.results[0];
      setPreview(r);
      if (!t.chart && r) {
        const spec = defaultSpec(r);
        setT((x) => ({ ...x, chart: spec, view: spec ? x.view : r.rows.length === 1 && r.columns.length <= 2 ? "number" : "table" }));
      }
    } catch (e) {
      setError(String(e));
      setPreview(null);
    } finally {
      setBusy(false);
    }
  };
  useEffect(() => {
    if (initial) run();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  const can = !!t.title.trim() && !!sql.trim();
  const viewOpts = useMemo(
    () => [
      { value: "chart" as const, label: <span className="flex items-center gap-1"><BarChart3 size={11} /> Chart</span> },
      { value: "table" as const, label: <span className="flex items-center gap-1"><Table2 size={11} /> Table</span> },
      { value: "number" as const, label: <span className="flex items-center gap-1"><Hash size={11} /> Number</span> },
    ],
    [],
  );

  return (
    <Modal
      open
      onClose={onClose}
      width={820}
      title={initial ? "Edit tile" : "Add tile"}
      footer={
        <>
          <span className="text-2xs text-muted">Width</span>
          <Segmented<string> value={String(t.w)} onChange={(w) => setT({ ...t, w: Number(w) })} options={WIDTHS.map((x) => ({ value: String(x.w), label: x.label }))} />
          <Segmented<string> value={String(t.h)} onChange={(h) => setT({ ...t, h: Number(h) })} options={[{ value: "1", label: "Short" }, { value: "2", label: "Tall" }]} />
          <span className="flex-1" />
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button variant="primary" disabled={!can} onClick={() => onSave({ ...t, title: t.title.trim(), sql })}>
            {initial ? "Save" : "Add to dashboard"}
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-3">
        <div className="grid grid-cols-[1fr_240px] gap-2">
          <label className="flex flex-col gap-1 text-2xs text-muted">
            Title
            <Input autoFocus value={t.title} onChange={(e) => setT({ ...t, title: e.target.value })} placeholder="Revenue this week" />
          </label>
          <label className="flex flex-col gap-1 text-2xs text-muted">
            Source
            <select
              value={t.snippetId ?? ""}
              onChange={(e) => {
                const id = e.target.value ? Number(e.target.value) : null;
                const s = snippets.find((x) => x.id === id);
                setT({ ...t, snippetId: id, sql: s?.sql ?? t.sql, title: t.title || s?.name || "", chart: null });
                setPreview(null);
              }}
              className="h-7 rounded-md border border-line bg-elevated px-1.5 text-xs text-fg outline-none"
            >
              <option value="">Write SQL</option>
              {snippets.map((s) => (
                <option key={s.id} value={s.id}>
                  Saved: {s.name}
                </option>
              ))}
            </select>
          </label>
        </div>
        <div className="relative">
          <textarea
            value={sql}
            readOnly={t.snippetId != null}
            onChange={(e) => setT({ ...t, sql: e.target.value })}
            onKeyDown={(e) => (e.metaKey || e.ctrlKey) && e.key === "Enter" && (e.preventDefault(), run())}
            spellCheck={false}
            placeholder="SELECT date_trunc('day', created_at) AS day, sum(total) FROM orders GROUP BY 1 ORDER BY 1"
            className={cx("h-28 w-full resize-none rounded-lg border border-line bg-panel p-2.5 font-mono text-[12px] outline-none focus:border-accent", t.snippetId != null && "text-muted")}
          />
          <Button size="xs" className="absolute right-2 bottom-2" disabled={!sql.trim() || busy} onClick={run}>
            <RefreshCw size={11} className={cx(busy && "animate-spin")} /> Run preview
          </Button>
        </div>
        {t.snippetId != null && <p className="-mt-2 text-2xs text-subtle">Follows the saved query — edit it in the query tab and the tile updates.</p>}
        <div className="flex items-center gap-2">
          <Segmented value={t.view} onChange={(view) => setT({ ...t, view })} options={viewOpts} />
          <span className="text-2xs text-subtle">Tiles always run read-only.</span>
        </div>
        <div className="h-64 overflow-hidden rounded-lg border border-line">
          {error ? (
            <div className="p-3 text-2xs text-danger">{error}</div>
          ) : !preview ? (
            <div className="flex h-full items-center justify-center text-xs text-subtle">Run the query to preview</div>
          ) : t.view === "chart" ? (
            <ResultsChart result={preview} spec={t.chart ?? undefined} onSpec={(chart: ChartSpec) => setT({ ...t, chart })} title={t.title} onToast={() => {}} />
          ) : (
            <TileBody tile={t} state={{ result: preview, loading: false }} onToast={() => {}} />
          )}
        </div>
      </div>
    </Modal>
  );
}

const DATEISH = /(date|day|from|to|since|until|start|end|_at|_on)$/i;

function iso(d: Date) {
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

/** One dashboard filter: date inputs get quick presets. */
function FilterInput({ v, value, onChange }: { v: VarRef; value: string; onChange: (v: string) => void }) {
  const date = DATEISH.test(v.name);
  const now = new Date();
  const presets: [string, string][] = date
    ? /(to|until|end)$/i.test(v.name)
      ? [["Today", iso(now)]]
      : [
          ["7d", iso(new Date(now.getTime() - 7 * 864e5))],
          ["30d", iso(new Date(now.getTime() - 30 * 864e5))],
          ["Month", iso(new Date(now.getFullYear(), now.getMonth(), 1))],
          ["Year", iso(new Date(now.getFullYear(), 0, 1))],
        ]
    : [];
  return (
    <label className="flex items-center gap-1.5 text-2xs">
      <span className="font-mono text-muted">{v.raw ? `{{${v.name}}}` : `:${v.name}`}</span>
      <input
        type={date ? "date" : "text"}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        spellCheck={false}
        placeholder={v.raw ? "raw SQL" : "value"}
        className={cx("h-6 rounded-md border border-line bg-elevated px-1.5 text-[11.5px] text-fg outline-none focus:border-accent", date ? "w-32" : "w-28")}
      />
      {presets.map(([l, val]) => (
        <button key={l} type="button" onClick={() => onChange(val)} className={cx("rounded px-1 text-[10.5px]", value === val ? "bg-accent-soft text-accent" : "text-subtle hover:text-fg")}>
          {l}
        </button>
      ))}
    </label>
  );
}

/** Save a PNG of the dashboard to a folder on a schedule. */
const DELIVERY: Record<DeliveryKind, { label: string; ph: string; hint: string }> = {
  slack: { label: "Slack", ph: "xoxb-… bot token", hint: "A Slack app's bot token with files:write, invited to the channel. Incoming webhooks can't post images." },
  discord: { label: "Discord", ph: "https://discord.com/api/webhooks/…", hint: "Channel settings → Integrations → Webhooks → Copy URL." },
  webhook: { label: "Webhook", ph: "https://example.com/hooks/rowhouse", hint: "POSTs multipart/form-data: `file` (the PNG) and `payload` (JSON)." },
};

function DeliveryEditor({ list, onChange, onToast }: { list: Delivery[]; onChange: (l: Delivery[]) => void; onToast: (m: string, k?: "ok" | "err") => void }) {
  const [kind, setKind] = useState<DeliveryKind>("slack");
  const [secret, setSecret] = useState("");
  const [channel, setChannel] = useState("");
  const [busy, setBusy] = useState(false);
  const field = "h-7 rounded-md border border-line bg-elevated px-2 text-xs text-fg outline-none focus:border-accent";
  const add = async () => {
    const d: Delivery = { id: uid(), kind, channel: kind === "slack" ? channel.trim() : null, label: "" };
    if (kind === "slack" && !d.channel) return onToast("Enter the Slack channel ID (C0123…)");
    setBusy(true);
    try {
      d.label = await api.dashboards.setDeliverySecret(d, secret);
      onChange([...list, d]);
      setSecret("");
      setChannel("");
    } catch (e) {
      onToast(String(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="flex flex-col gap-2">
      <span className="text-2xs text-muted">Also send each snapshot to</span>
      {list.map((d) => (
        <div key={d.id} className="flex items-center gap-2 rounded-md border border-line bg-panel/40 px-2 py-1">
          <Send size={11} className="text-subtle" />
          <span className="font-medium">{DELIVERY[d.kind].label}</span>
          <span className="min-w-0 flex-1 truncate font-mono text-2xs text-muted">
            {d.kind === "slack" ? `#${d.channel} · ` : ""}
            {d.label}
          </span>
          <button
            type="button"
            title="Remove"
            className="text-subtle hover:text-danger"
            onClick={() => {
              api.dashboards.deleteDeliverySecret(d).catch(() => {});
              onChange(list.filter((x) => x.id !== d.id));
            }}
          >
            <X size={12} />
          </button>
        </div>
      ))}
      <div className="flex flex-wrap items-center gap-2">
        <select value={kind} onChange={(e) => setKind(e.target.value as DeliveryKind)} className={field}>
          {(Object.keys(DELIVERY) as DeliveryKind[]).map((k) => (
            <option key={k} value={k}>
              {DELIVERY[k].label}
            </option>
          ))}
        </select>
        {kind === "slack" && <input value={channel} onChange={(e) => setChannel(e.target.value)} placeholder="Channel ID" className={cx(field, "w-28 font-mono")} />}
        <input type="password" value={secret} onChange={(e) => setSecret(e.target.value)} placeholder={DELIVERY[kind].ph} className={cx(field, "min-w-0 flex-1 font-mono")} />
        <Button size="sm" disabled={!secret.trim() || busy} onClick={add}>
          <Plus size={12} /> Add
        </Button>
      </div>
      <p className="text-2xs text-subtle">{DELIVERY[kind].hint} Secrets stay in your system keychain.</p>
    </div>
  );
}

function SnapshotDialog({
  dash,
  onClose,
  onSave,
  onToast,
}: {
  dash: Dashboard;
  onClose: () => void;
  onSave: (s: import("../lib/api").DashSnapshot | null, sendNow?: boolean) => void;
  onToast: (m: string, k?: "ok" | "err") => void;
}) {
  const cur = dash.snapshot;
  const [enabled, setEnabled] = useState(cur?.enabled ?? true);
  const [kind, setKind] = useState<"daily" | "weekly" | "minutes">((cur?.every.kind as "daily" | "weekly" | "minutes") ?? "daily");
  const [at, setAt] = useState(cur && "at" in cur.every ? cur.every.at : "08:00");
  const [weekday, setWeekday] = useState(cur && "weekday" in cur.every ? cur.every.weekday : 0);
  const [n, setN] = useState(cur && "n" in cur.every ? cur.every.n : 60);
  const [dir, setDir] = useState(cur?.dir ?? "");
  const [notify, setNotify] = useState(cur?.notify ?? true);
  const [deliver, setDeliver] = useState<Delivery[]>(cur?.deliver ?? []);
  const ready = !!dir.trim() || deliver.length > 0;
  const value = () => ({ enabled, every: every as import("../lib/api").DashSnapshot["every"], dir: dir.trim(), deliver, notify, lastRun: cur?.lastRun ?? null, lastError: null });
  const every = kind === "daily" ? { kind, at } : kind === "weekly" ? { kind, weekday, at } : { kind, n: Math.max(5, n) };
  const field = "h-7 rounded-md border border-line bg-elevated px-2 text-xs text-fg outline-none focus:border-accent";
  return (
    <Modal
      open
      onClose={onClose}
      width={480}
      title="Scheduled snapshots"
      footer={
        <>
          {cur && (
            <Button variant="danger" size="sm" onClick={() => (deliver.forEach((d) => api.dashboards.deleteDeliverySecret(d).catch(() => {})), onSave(null))}>
              Remove
            </Button>
          )}
          <span className="flex-1" />
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button disabled={!ready} onClick={() => onSave(value(), true)} title="Save, then take a snapshot right away">
            <Send size={12} /> Save &amp; send now
          </Button>
          <Button variant="primary" disabled={!ready} onClick={() => onSave(value())}>
            Save
          </Button>
        </>
      }
    >
      <div className="flex flex-col gap-3 text-xs">
        <p className="text-muted">Rowhouse refreshes “{dash.name}” and saves a PNG into a folder, posts it to Slack or Discord, or both — a daily report without a BI server. Runs while the app is open (a missed run happens on next launch).</p>
        <div className="flex flex-wrap items-center gap-2">
          <Segmented<string>
            value={kind}
            onChange={(k) => setKind(k as "daily" | "weekly" | "minutes")}
            options={[
              { value: "daily", label: "Daily" },
              { value: "weekly", label: "Weekly" },
              { value: "minutes", label: "Every…" },
            ]}
          />
          {kind === "weekly" && (
            <select value={weekday} onChange={(e) => setWeekday(Number(e.target.value))} className={field}>
              {["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"].map((d, i) => (
                <option key={d} value={i}>
                  {d}
                </option>
              ))}
            </select>
          )}
          {kind !== "minutes" ? (
            <input type="time" value={at} onChange={(e) => setAt(e.target.value)} className={field} />
          ) : (
            <span className="flex items-center gap-1.5">
              <input type="number" min={5} value={n} onChange={(e) => setN(Number(e.target.value))} className={cx(field, "w-20")} /> minutes
            </span>
          )}
        </div>
        <div className="flex gap-2">
          <Input value={dir} onChange={(e) => setDir(e.target.value)} placeholder="Folder for the PNGs (optional when sending)" className="font-mono" />
          <Button onClick={() => dialogs.folder().then((f) => f && setDir(f))}>Browse…</Button>
        </div>
        <DeliveryEditor list={deliver} onChange={setDeliver} onToast={onToast} />
        <label className="flex items-center gap-2">
          <input type="checkbox" checked={notify} onChange={(e) => setNotify(e.target.checked)} className="accent-[var(--accent)]" /> Notify me when a snapshot is saved
        </label>
        <label className="flex items-center gap-2">
          <input type="checkbox" checked={enabled} onChange={(e) => setEnabled(e.target.checked)} className="accent-[var(--accent)]" /> Enabled
        </label>
        {cur?.lastRun && (
          <p className={cx("text-2xs", cur.lastError ? "text-danger" : "text-subtle")}>
            Last run {new Date(cur.lastRun * 1000).toLocaleString()}
            {cur.lastError ? ` — ${cur.lastError}` : ""}
          </p>
        )}
      </div>
    </Modal>
  );
}
