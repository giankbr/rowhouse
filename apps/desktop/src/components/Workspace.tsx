import { AlertCircle, Check, Download, KeyRound, Lock, Play, RefreshCw, Upload } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { api, dialogs, type ConnectionConfig, type ExportFormat, type TableInfo } from "../lib/api";
import { countPending, editBlocker, toChangeSet } from "../lib/pending";
import { mod, type Tab } from "../lib/types";
import { ResultsGrid } from "./ResultsGrid";
import { SqlEditor } from "./SqlEditor";
import { Button, Kbd, Menu, Modal, Segmented, cx } from "./ui";

export function Workspace({
  tab,
  conn,
  tables,
  onChange,
  onRun,
  onView,
  onToast,
}: {
  tab: Tab;
  conn: ConnectionConfig | undefined;
  tables: TableInfo[];
  onChange: (patch: Partial<Tab>) => void;
  onRun: (sql?: string) => void;
  onView: (v: "data" | "structure") => void;
  onToast: (msg: string, kind?: "ok" | "err") => void;
}) {
  const [editorH, setEditorH] = useState(220);
  const boxRef = useRef<HTMLDivElement>(null);
  const onSql = useCallback((sql: string) => onChange({ sql }), [onChange]);

  const startDrag = (e: React.MouseEvent) => {
    e.preventDefault();
    const top = boxRef.current!.getBoundingClientRect().top;
    const max = boxRef.current!.clientHeight - 80;
    const move = (ev: MouseEvent) => setEditorH(Math.max(80, Math.min(max, ev.clientY - top)));
    const up = () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  };

  const results = tab.output?.results ?? [];
  const idx = Math.min(tab.resultIndex ?? 0, Math.max(0, results.length - 1));
  const result = results[idx];

  // ---- inline editing (table tabs) ----------------------------------------
  const isTable = tab.kind === "table";
  const blocker = !isTable ? null : conn?.readOnly ? "Read-only connection" : editBlocker(result, tab.columns);
  const editable = isTable && !blocker && !tab.running;
  const pendingCount = countPending(tab.pending);
  const [review, setReview] = useState<{ sql: string[]; error?: string; busy?: boolean } | null>(null);

  const openReview = useCallback(async () => {
    if (!result || !tab.columns || !tab.schema || !tab.table || !tab.pending) return;
    const cs = toChangeSet(tab.pending, result, tab.columns, tab.schema, tab.table);
    try {
      setReview({ sql: await api.previewChanges(tab.connId, cs) });
    } catch (e) {
      setReview({ sql: [], error: String(e) });
    }
  }, [result, tab]);

  const commit = async () => {
    if (!result || !tab.columns || !tab.schema || !tab.table || !tab.pending || !review) return;
    setReview({ ...review, busy: true, error: undefined });
    try {
      await api.applyChanges(tab.connId, toChangeSet(tab.pending, result, tab.columns, tab.schema, tab.table));
      setReview(null);
      onChange({ pending: undefined });
      onRun();
    } catch (e) {
      setReview({ ...review, busy: false, error: String(e) });
    }
  };

  // ---- import / export ----------------------------------------------------
  const exportAs = async (format: ExportFormat) => {
    const base = isTable ? tab.table! : "query";
    const path = await dialogs.save(`${base}.${format}`, format);
    if (!path) return;
    try {
      const sql = isTable ? await api.tableSqlAll(tab.connId, tab.schema!, tab.table!) : tab.sql;
      const n = await api.exportQuery(tab.connId, sql, format, path, tab.schema, tab.table);
      onToast(`Exported ${n.toLocaleString()} rows → ${path.split(/[\\/]/).pop()}`, "ok");
    } catch (e) {
      onToast(String(e));
    }
  };

  const importCsv = async () => {
    const path = await dialogs.openCsv();
    if (!path || !tab.schema || !tab.table) return;
    try {
      const n = await api.importCsv(tab.connId, tab.schema, tab.table, path);
      onToast(`Imported ${n.toLocaleString()} rows into ${tab.table}`, "ok");
      onRun();
    } catch (e) {
      onToast(String(e));
    }
  };

  const exportMenu = (
    <Menu
      label={
        <>
          <Download size={11} /> Export
        </>
      }
      disabled={tab.running}
      items={[
        { label: "CSV", hint: ".csv", onSelect: () => exportAs("csv") },
        { label: "JSON", hint: ".json", onSelect: () => exportAs("json") },
        { label: "SQL INSERTs", hint: ".sql", onSelect: () => exportAs("sql") },
      ]}
    />
  );

  // Mod+S opens the review for pending edits.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "s") {
        e.preventDefault();
        if (pendingCount > 0 && !review) openReview();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [pendingCount, review, openReview]);

  return (
    <div ref={boxRef} className="flex min-h-0 flex-1 flex-col">
      {tab.kind === "query" ? (
        <>
          <div style={{ height: editorH }} className="min-h-0 shrink-0 overflow-hidden">
            <SqlEditor value={tab.sql} onChange={onSql} onRun={onRun} kind={conn?.kind ?? "postgres"} tables={tables} />
          </div>
          <div onMouseDown={startDrag} className="group relative h-px shrink-0 cursor-row-resize bg-line">
            <div className="absolute inset-x-0 -top-1 -bottom-1 group-hover:bg-accent/30" />
          </div>
          <Toolbar>
            <Button variant="primary" size="xs" onClick={() => onRun()} disabled={tab.running}>
              <Play size={11} fill="currentColor" /> Run
            </Button>
            <span className="flex items-center gap-0.5 text-2xs text-subtle">
              <Kbd>{mod}</Kbd>
              <Kbd>↵</Kbd>
              <span className="ml-1">selection or all</span>
            </span>
            <ResultSwitcher count={results.length} idx={idx} onPick={(i) => onChange({ resultIndex: i })} />
            <span className={cx(results.length < 2 && "ml-auto")}>{exportMenu}</span>
          </Toolbar>
        </>
      ) : (
        <Toolbar>
          <Segmented
            value={tab.view ?? "data"}
            onChange={onView}
            options={[
              { value: "data", label: "Data" },
              { value: "structure", label: "Structure" },
            ]}
          />
          <span className="truncate font-mono text-2xs text-subtle">{tab.sql}</span>
          <span className="flex-1" />
          {blocker && blocker !== "loading" && (
            <span className="flex items-center gap-1 text-2xs whitespace-nowrap text-subtle">
              <Lock size={11} /> {blocker}
            </span>
          )}
          {pendingCount > 0 ? (
            <>
              <span className="text-2xs whitespace-nowrap text-warning">
                {pendingCount} change{pendingCount === 1 ? "" : "s"}
              </span>
              <Button variant="ghost" size="xs" onClick={() => onChange({ pending: undefined })}>
                Discard
              </Button>
              <Button variant="primary" size="xs" onClick={openReview}>
                <Check size={12} /> Review & commit
              </Button>
            </>
          ) : (
            <>
              {!conn?.readOnly && (
                <Button variant="ghost" size="xs" onClick={importCsv} disabled={tab.running}>
                  <Upload size={11} /> Import
                </Button>
              )}
              {exportMenu}
              <Button variant="ghost" size="xs" onClick={() => onRun()} disabled={tab.running}>
                <RefreshCw size={11} className={cx(tab.running && "animate-spin")} /> Refresh
              </Button>
            </>
          )}
        </Toolbar>
      )}

      <div className="relative min-h-0 flex-1">
        {tab.kind === "table" && tab.view === "structure" ? (
          <Structure tab={tab} />
        ) : tab.error ? (
          <div className="selectable m-3 flex gap-2 rounded-lg border border-danger/30 bg-danger/5 p-3 font-mono text-xs text-danger">
            <AlertCircle size={14} className="mt-px shrink-0" />
            <pre className="whitespace-pre-wrap">{tab.error}</pre>
          </div>
        ) : result ? (
          <ResultsGrid
            result={result}
            edit={editable ? { pending: tab.pending ?? { edits: {}, deletes: [], inserts: [] }, onChange: (p) => onChange({ pending: p }) } : undefined}
          />
        ) : (
          <div className="flex h-full items-center justify-center text-xs text-subtle">
            {tab.running ? "Running…" : "Run a query to see results"}
          </div>
        )}
        {tab.running && <div className="absolute inset-x-0 top-0 h-0.5 animate-pulse bg-accent" />}
      </div>

      <Modal
        open={!!review}
        onClose={() => !review?.busy && setReview(null)}
        width={640}
        title={`Review ${pendingCount} change${pendingCount === 1 ? "" : "s"} · ${tab.table}`}
        footer={
          <>
            <span className="flex-1 text-2xs text-subtle">Runs in one transaction · each UPDATE/DELETE must hit exactly 1 row</span>
            <Button onClick={() => setReview(null)} disabled={review?.busy}>
              Cancel
            </Button>
            <Button variant="primary" onClick={commit} disabled={review?.busy || !review?.sql.length}>
              {review?.busy ? "Committing…" : "Commit"}
            </Button>
          </>
        }
      >
        <div className="flex flex-col gap-2">
          <pre className="selectable max-h-72 overflow-auto rounded-lg border border-line bg-panel p-3 font-mono text-2xs leading-relaxed whitespace-pre-wrap">
            {review?.sql.map((s) => s + ";").join("\n")}
          </pre>
          {review?.error && (
            <div className="selectable flex gap-2 rounded-lg border border-danger/30 bg-danger/5 p-2.5 font-mono text-2xs text-danger">
              <AlertCircle size={13} className="shrink-0" />
              <pre className="whitespace-pre-wrap">{review.error}</pre>
            </div>
          )}
        </div>
      </Modal>
    </div>
  );
}

function Toolbar({ children }: { children: React.ReactNode }) {
  return <div className="flex h-9 shrink-0 items-center gap-2 border-b border-line px-2">{children}</div>;
}

function ResultSwitcher({ count, idx, onPick }: { count: number; idx: number; onPick: (i: number) => void }) {
  if (count < 2) return null;
  return (
    <div className="ml-auto flex items-center gap-0.5">
      {Array.from({ length: count }).map((_, i) => (
        <button
          key={i}
          onClick={() => onPick(i)}
          className={cx("h-5 rounded px-1.5 text-2xs", i === idx ? "bg-selected text-fg" : "text-muted hover:bg-hover")}
        >
          Result {i + 1}
        </button>
      ))}
    </div>
  );
}

function Structure({ tab }: { tab: Tab }) {
  if (!tab.columns) return <div className="p-4 text-xs text-subtle">Loading…</div>;
  return (
    <div className="h-full overflow-auto">
      <table className="w-full border-collapse text-xs">
        <thead className="sticky top-0 bg-panel text-left text-2xs text-muted">
          <tr className="h-7 border-b border-line">
            <th className="w-8" />
            <th className="px-2 font-medium">Column</th>
            <th className="px-2 font-medium">Type</th>
            <th className="px-2 font-medium">Nullable</th>
            <th className="px-2 font-medium">Default</th>
          </tr>
        </thead>
        <tbody className="font-mono">
          {tab.columns.map((c) => (
            <tr key={c.name} className="h-7 border-b border-line/60 hover:bg-hover">
              <td className="pl-3 text-warning">{c.primary_key && <KeyRound size={11} />}</td>
              <td className="px-2 font-medium">{c.name}</td>
              <td className="px-2 text-accent">{c.data_type}</td>
              <td className="px-2 text-muted">{c.nullable ? "YES" : "NO"}</td>
              <td className="px-2 text-muted">{c.default ?? <span className="text-subtle italic">—</span>}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
