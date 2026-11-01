import { ArrowDown, ArrowUp, BarChart3, Download, FileText, Loader2, NotebookPen, Play, Plus, SquareTerminal, Table2, Trash2 } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api, dialogs, kindInfo, type ConnectionConfig, type NbCell, type Notebook, type QueryResult, type TableInfo } from "../lib/api";
import { renderMarkdown, resultToMarkdown } from "../lib/markdown";
import { connMasks } from "../lib/mask";
import { uid } from "../lib/types";
import { findVars, substitute, type VarRef } from "../lib/vars";
import { ResultsChart, defaultSpec } from "./ResultsChart";
import { ResultsGrid } from "./ResultsGrid";
import { SqlEditor } from "./SqlEditor";
import { Button, Input, cx } from "./ui";

type Out = { result?: QueryResult; error?: string; loading?: boolean; ms?: number };

/** SQL + Markdown cells on one connection; saved as you type, synced like dashboards. */
export function NotebookView({
  notebookId,
  conn,
  tables,
  onToast,
  onDeleted,
}: {
  notebookId: string;
  conn: ConnectionConfig | undefined;
  tables: TableInfo[];
  onToast: (m: string, k?: "ok" | "err") => void;
  onDeleted: () => void;
}) {
  const [nb, setNb] = useState<Notebook | null>(null);
  const [out, setOut] = useState<Record<string, Out>>({});
  const [editing, setEditing] = useState<string | null>(null);
  const [confirmDel, setConfirmDel] = useState(false);
  const saveTimer = useRef<ReturnType<typeof setTimeout>>(undefined);
  const nbRef = useRef<Notebook | null>(null);
  nbRef.current = nb;

  useEffect(() => {
    api.notebooks
      .list()
      .then((l) => setNb(l.find((n) => n.id === notebookId) ?? null))
      .catch((e) => onToast(String(e)));
  }, [notebookId, onToast]);

  const update = useCallback(
    (fn: (n: Notebook) => Notebook) => {
      setNb((cur) => {
        if (!cur) return cur;
        const next = fn(cur);
        clearTimeout(saveTimer.current);
        saveTimer.current = setTimeout(() => api.notebooks.save(next).catch((e) => onToast(String(e))), 600);
        return next;
      });
    },
    [onToast],
  );
  useEffect(() => () => clearTimeout(saveTimer.current), []);

  const dialect = conn ? kindInfo(conn.kind).dialect : "postgres";
  const vars = useMemo(() => {
    const seen = new Map<string, VarRef>();
    for (const c of nb?.cells ?? []) if (c.kind === "sql") for (const v of findVars(c.source, dialect)) if (!seen.has(v.name)) seen.set(v.name, v);
    return [...seen.values()];
  }, [nb?.cells, dialect]);

  const runCell = useCallback(
    async (c: NbCell) => {
      const n = nbRef.current;
      if (!n || c.kind !== "sql" || !c.source.trim()) return;
      setOut((o) => ({ ...o, [c.id]: { ...o[c.id], loading: true, error: undefined } }));
      const started = performance.now();
      try {
        const sql = substitute(c.source, n.params ?? {}, dialect);
        const res = await api.runQuery(n.connectionId, sql);
        const result = [...res.results].reverse().find((r) => r.columns.length > 0) ?? res.results[res.results.length - 1];
        setOut((o) => ({ ...o, [c.id]: { result, ms: Math.round(performance.now() - started) } }));
      } catch (e) {
        setOut((o) => ({ ...o, [c.id]: { error: String(e) } }));
      }
    },
    [dialect],
  );
  const runAll = useCallback(async () => {
    for (const c of nbRef.current?.cells ?? []) if (c.kind === "sql") await runCell(c);
  }, [runCell]);
  const ran = useRef(false);
  useEffect(() => {
    if (nb && !ran.current) {
      ran.current = true;
      runAll();
    }
  }, [nb, runAll]);

  const patchCell = (id: string, p: Partial<NbCell>) => update((n) => ({ ...n, cells: n.cells.map((c) => (c.id === id ? { ...c, ...p } : c)) }));
  const addCell = (kind: NbCell["kind"], after?: string) => {
    const cell: NbCell = { id: uid(), kind, source: "", view: kind === "sql" ? "table" : null };
    update((n) => {
      const i = after ? n.cells.findIndex((c) => c.id === after) + 1 : n.cells.length;
      const cells = [...n.cells];
      cells.splice(i, 0, cell);
      return { ...n, cells };
    });
    if (kind === "markdown") setEditing(cell.id);
  };
  const move = (id: string, d: -1 | 1) =>
    update((n) => {
      const i = n.cells.findIndex((c) => c.id === id);
      const j = i + d;
      if (i < 0 || j < 0 || j >= n.cells.length) return n;
      const cells = [...n.cells];
      [cells[i], cells[j]] = [cells[j], cells[i]];
      return { ...n, cells };
    });

  const exportMd = async () => {
    if (!nb) return;
    const parts = [`<!-- ${nb.name} · ${conn?.name ?? ""} · ${new Date().toLocaleString()} -->`];
    for (const c of nb.cells) {
      if (c.kind === "markdown") parts.push(c.source);
      else {
        parts.push("```sql\n" + c.source.trim() + "\n```");
        const o = out[c.id];
        if (o?.result) parts.push(resultToMarkdown(o.result));
        else if (o?.error) parts.push(`> Error: ${o.error}`);
      }
    }
    const path = await dialogs.save(`${nb.name.replace(/[^\w.-]+/g, "-")}.md`, "md");
    if (!path) return;
    await api.writeTextFile(path, parts.join("\n\n") + "\n");
    onToast(`Saved ${path.split(/[\\/]/).pop()}`, "ok");
  };

  if (!nb) return <div className="flex h-full items-center justify-center text-xs text-subtle">Loading notebook…</div>;
  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex h-10 shrink-0 items-center gap-2 border-b border-line px-3">
        <NotebookPen size={14} className="text-accent" />
        <input
          value={nb.name}
          onChange={(e) => update((n) => ({ ...n, name: e.target.value }))}
          className="min-w-0 flex-1 bg-transparent text-xs font-semibold outline-none"
          aria-label="Notebook name"
        />
        <span className="text-2xs text-subtle">{conn?.name}</span>
        <Button size="xs" variant="ghost" onClick={exportMd} title="Save as Markdown, with the results as tables">
          <Download size={12} /> Markdown
        </Button>
        <Button size="xs" variant="primary" onClick={runAll}>
          <Play size={11} /> Run all
        </Button>
        {confirmDel ? (
          <span className="flex items-center gap-1 text-2xs">
            <span className="text-danger">Delete notebook?</span>
            <Button size="xs" variant="danger" onClick={async () => (await api.notebooks.remove(nb.id), onDeleted())}>
              Delete
            </Button>
            <Button size="xs" variant="ghost" onClick={() => setConfirmDel(false)}>
              Keep
            </Button>
          </span>
        ) : (
          <Button size="icon" variant="ghost" title="Delete notebook" onClick={() => setConfirmDel(true)}>
            <Trash2 size={13} />
          </Button>
        )}
      </div>
      {vars.length > 0 && (
        <div className="flex shrink-0 flex-wrap items-center gap-x-3 gap-y-1.5 border-b border-line bg-panel/40 px-3 py-1.5">
          {vars.map((v) => (
            <label key={v.name} className="flex items-center gap-1.5 text-2xs">
              <span className="font-mono text-muted">{v.raw ? `{{${v.name}}}` : `:${v.name}`}</span>
              <Input
                value={nb.params?.[v.name] ?? ""}
                onChange={(e) => update((n) => ({ ...n, params: { ...(n.params ?? {}), [v.name]: e.target.value } }))}
                onKeyDown={(e) => e.key === "Enter" && runAll()}
                placeholder="value"
                className="h-6 w-36 font-mono"
              />
            </label>
          ))}
          <span className="text-2xs text-subtle">Enter runs all cells</span>
        </div>
      )}
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="mx-auto flex max-w-5xl flex-col gap-3 px-4 py-4">
          {nb.cells.length === 0 && (
            <div className="rounded-xl border border-dashed border-line p-8 text-center text-xs text-muted">
              Mix queries and notes: add a SQL cell, a text cell, and share the result as Markdown.
            </div>
          )}
          {nb.cells.map((c, i) => {
            const o = out[c.id];
            const lines = Math.min(16, Math.max(3, c.source.split("\n").length + 1));
            return (
              <div key={c.id} className="group rounded-xl border border-line bg-bg">
                <div className="flex h-8 items-center gap-1 border-b border-line/70 px-2 text-2xs text-subtle">
                  {c.kind === "sql" ? <SquareTerminal size={12} /> : <FileText size={12} />}
                  <span className="mr-auto">{c.kind === "sql" ? "SQL" : "Text"}</span>
                  {c.kind === "sql" && o?.result && (
                    <>
                      <span className="mr-1 tabular-nums">
                        {o.result.rows.length.toLocaleString()} rows · {o.ms} ms
                      </span>
                      <Button size="icon" variant="ghost" title="Table" className={cx(c.view !== "chart" && "text-fg")} onClick={() => patchCell(c.id, { view: "table" })}>
                        <Table2 size={12} />
                      </Button>
                      <Button size="icon" variant="ghost" title="Chart" className={cx(c.view === "chart" && "text-fg")} onClick={() => patchCell(c.id, { view: "chart", chart: c.chart ?? defaultSpec(o.result!) })}>
                        <BarChart3 size={12} />
                      </Button>
                    </>
                  )}
                  <span className="flex opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100">
                    <Button size="icon" variant="ghost" title="Move up" disabled={i === 0} onClick={() => move(c.id, -1)}>
                      <ArrowUp size={12} />
                    </Button>
                    <Button size="icon" variant="ghost" title="Move down" disabled={i === nb.cells.length - 1} onClick={() => move(c.id, 1)}>
                      <ArrowDown size={12} />
                    </Button>
                    <Button size="icon" variant="ghost" title="Delete cell" onClick={() => update((n) => ({ ...n, cells: n.cells.filter((x) => x.id !== c.id) }))}>
                      <Trash2 size={12} />
                    </Button>
                  </span>
                  {c.kind === "sql" && (
                    <Button size="xs" variant="ghost" onClick={() => runCell(c)} title="Run cell (Ctrl+Enter)">
                      {o?.loading ? <Loader2 size={11} className="animate-spin" /> : <Play size={11} />} Run
                    </Button>
                  )}
                </div>
                {c.kind === "sql" ? (
                  <>
                    <div style={{ height: lines * 19 + 18 }}>
                      <SqlEditor value={c.source} onChange={(v) => patchCell(c.id, { source: v })} onRun={() => runCell({ ...c, source: nbRef.current?.cells.find((x) => x.id === c.id)?.source ?? c.source })} kind={conn?.kind ?? "postgres"} tables={tables} />
                    </div>
                    {o?.error && <pre className="m-2 rounded-md bg-danger/8 px-2 py-1.5 font-mono text-2xs whitespace-pre-wrap text-danger">{o.error}</pre>}
                    {o?.result &&
                      (o.result.columns.length === 0 ? (
                        <div className="border-t border-line/70 px-3 py-1.5 text-2xs text-muted">Query OK · {o.result.rows_affected} rows affected</div>
                      ) : c.view === "chart" ? (
                        <div className="h-72 border-t border-line/70">
                          <ResultsChart compact result={o.result} spec={c.chart ?? undefined} onSpec={(chart) => patchCell(c.id, { chart })} title={nb.name} onToast={onToast} />
                        </div>
                      ) : (
                        <div className="flex border-t border-line/70" style={{ height: Math.min(320, 34 + Math.max(1, o.result.rows.length) * 26) }}>
                          <ResultsGrid result={o.result} mask={connMasks(conn)} />
                        </div>
                      ))}
                  </>
                ) : editing === c.id || !c.source.trim() ? (
                  <textarea
                    autoFocus={editing === c.id}
                    value={c.source}
                    onChange={(e) => patchCell(c.id, { source: e.target.value })}
                    onBlur={() => setEditing(null)}
                    onKeyDown={(e) => (e.key === "Escape" || (e.key === "Enter" && (e.metaKey || e.ctrlKey))) && (e.preventDefault(), setEditing(null))}
                    placeholder="Write notes in Markdown — # heading, **bold**, - lists, | tables |"
                    rows={Math.max(3, c.source.split("\n").length + 1)}
                    className="block w-full resize-y bg-transparent px-3 py-2 font-mono text-xs outline-none"
                  />
                ) : (
                  <div className="rh-md cursor-text px-4 py-3 text-xs" onDoubleClick={() => setEditing(c.id)} title="Double-click to edit" dangerouslySetInnerHTML={{ __html: renderMarkdown(c.source) }} />
                )}
                <div className="flex h-0 justify-center gap-1 opacity-0 transition-opacity group-hover:opacity-100">
                  <span className="-mt-3 flex gap-1">
                    <Button size="xs" className="h-6 bg-elevated shadow-sm" onClick={() => addCell("sql", c.id)}>
                      <Plus size={11} /> SQL
                    </Button>
                    <Button size="xs" className="h-6 bg-elevated shadow-sm" onClick={() => addCell("markdown", c.id)}>
                      <Plus size={11} /> Text
                    </Button>
                  </span>
                </div>
              </div>
            );
          })}
          <div className="flex justify-center gap-2 pt-1 pb-8">
            <Button size="sm" onClick={() => addCell("sql")}>
              <SquareTerminal size={12} /> SQL cell
            </Button>
            <Button size="sm" onClick={() => addCell("markdown")}>
              <FileText size={12} /> Text cell
            </Button>
          </div>
        </div>
      </div>
    </div>
  );
}
