import { AlertCircle, Bookmark, CalendarClock, ChevronLeft, ChevronRight, Filter as FilterIcon, BookOpen, Check, Download, GanttChart, History, KeyRound, Sparkles, Wand2, Lock, Play, RefreshCw, Square, Upload } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { api, dialogs, kindInfo, type ConnectionConfig, type ExportFormat, type TableInfo } from "../lib/api";
import { countPending, editBlocker, toChangeSet } from "../lib/pending";
import { mod, type Tab } from "../lib/types";
import { AiPanel, type AiState } from "./AiPanel";
import { FilterBar } from "./FilterBar";
import { PlanView } from "./PlanView";
import { StructureEditor } from "./StructureEditor";
import { ResultsGrid } from "./ResultsGrid";
import { SqlEditor } from "./SqlEditor";
import { Button, Field, Input, Kbd, Menu, Modal, Segmented, Switch, cx } from "./ui";

export function Workspace({
  tab,
  conn,
  tables,
  onChange,
  onRun,
  onView,
  onToast,
  onHistory,
  onSchedule,
  onCancel,
  aiEnabled,
  inlineAi = false,
  onOpenSettings,
  active = true,
  onBrowse,
}: {
  tab: Tab;
  conn: ConnectionConfig | undefined;
  tables: TableInfo[];
  onChange: (patch: Partial<Tab>) => void;
  onRun: (sql?: string) => void;
  onView: (v: "data" | "structure") => void;
  onToast: (msg: string, kind?: "ok" | "err") => void;
  onHistory: () => void;
  onSchedule?: (sql: string) => void;
  /** stop the running query */
  onCancel?: () => void;
  aiEnabled: boolean;
  /** ghost-text suggestions in the editor (AI settings) */
  inlineAi?: boolean;
  onOpenSettings: () => void;
  /** focused pane: only it reacts to global shortcuts */
  active?: boolean;
  /** table tabs: change filters / sort / page and reload from the server */
  onBrowse?: (patch: Pick<Tab, "filters" | "sort" | "offset">) => void;
}) {
  const [showFilters, setShowFilters] = useState((tab.filters?.length ?? 0) > 0);
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

  // ---- AI ----------------------------------------------------------------
  const [ai, setAi] = useState<AiState | null>(null);
  const needAi = () => {
    if (!aiEnabled) {
      onOpenSettings();
      return false;
    }
    return true;
  };
  const askAi = () => needAi() && setAi({ mode: "ask", prompt: "" });
  const runAi = async (state: AiState) => {
    setAi({ ...state, loading: true, error: undefined, answer: undefined, streamed: "" });
    const onDelta = (d: string) => setAi((cur) => cur && { ...cur, streamed: (cur.streamed ?? "") + d });
    try {
      const answer =
        state.mode === "ask"
          ? await api.ai.generate(tab.connId, state.prompt, tab.sql, onDelta)
          : state.mode === "fix"
            ? await api.ai.fix(tab.connId, tab.sql, tab.error ?? "", onDelta)
            : await api.ai.explain(tab.connId, tab.sql, onDelta);
      setAi((cur) => cur && { ...cur, loading: false, answer });
    } catch (e) {
      setAi((cur) => cur && { ...cur, loading: false, error: String(e) });
    }
  };
  const fixWithAi = () => needAi() && runAi({ mode: "fix" });
  const explainWithAi = () => needAi() && runAi({ mode: "explain" });
  const acceptAi = (run: boolean) => {
    const sql = ai?.answer?.sql;
    if (!sql) return;
    onChange({ sql });
    setAi(null);
    if (run) onRun(sql);
  };

  useEffect(() => {
    if (tab.kind !== "query" || !active) return;
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === "." && tab.running) {
        e.preventDefault();
        onCancel?.();
        return;
      }
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "i") {
        // inside the editor, Mod+I is the inline edit box (handled by SqlEditor)
        if (aiEnabled && (e.target as HTMLElement | null)?.closest?.(".cm-editor")) return;
        e.preventDefault();
        askAi();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  // ---- query plan -----------------------------------------------------------
  const [analyze, setAnalyze] = useState(false);
  const isRedis = !!conn && kindInfo(conn.kind).dialect === "redis";
  const isCql = !!conn && kindInfo(conn.kind).dialect === "cql";
  const canAnalyze = conn?.kind === "postgres" || conn?.kind === "greenplum";
  const explainPlan = async (withAnalyze = analyze) => {
    onChange({ showPlan: true, planLoading: true, planError: undefined });
    try {
      onChange({ plan: await api.explainPlan(tab.connId, tab.sql, withAnalyze && canAnalyze), planLoading: false });
    } catch (e) {
      onChange({ planError: String(e), planLoading: false, plan: undefined });
    }
  };

  // ---- saved queries --------------------------------------------------------
  const [saveDlg, setSaveDlg] = useState<{ name: string; global: boolean } | null>(null);
  const saveSnippet = async () => {
    if (!saveDlg?.name.trim()) return;
    try {
      const saved = await api.saveSnippet({
        id: tab.snippetId ?? 0,
        name: saveDlg.name.trim(),
        sql: tab.sql,
        connectionId: saveDlg.global ? null : tab.connId,
        updatedAt: 0,
      });
      onChange({ snippetId: saved.id, title: saved.name });
      setSaveDlg(null);
      onToast(`Saved “${saved.name}”`, "ok");
    } catch (e) {
      onToast(String(e));
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
    if (!active) return;
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "s") {
        e.preventDefault();
        if (pendingCount > 0 && !review) openReview();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [pendingCount, review, openReview, active]);

  return (
    <div ref={boxRef} className="flex min-h-0 flex-1 flex-col">
      {tab.kind === "query" ? (
        <>
          <div style={{ height: editorH }} className="min-h-0 shrink-0 overflow-hidden">
            <SqlEditor
              value={tab.sql}
              onChange={onSql}
              onRun={onRun}
              kind={conn?.kind ?? "postgres"}
              tables={tables}
              suggest={inlineAi ? (before, after) => api.ai.completeInline(tab.connId, before, after) : undefined}
              inlineEdit={aiEnabled ? (ins, sel, before, after, onDelta) => api.ai.edit(tab.connId, ins, sel, before, after, onDelta) : undefined}
            />
          </div>
          <div onMouseDown={startDrag} className="group relative h-px shrink-0 cursor-row-resize bg-line">
            <div className="absolute inset-x-0 -top-1 -bottom-1 group-hover:bg-accent/30" />
          </div>
          {ai && (
            <AiPanel
              state={ai}
              currentSql={tab.sql}
              onChange={setAi}
              onSubmit={() => runAi(ai)}
              onAccept={acceptAi}
              onClose={() => setAi(null)}
            />
          )}
          <Toolbar>
            {tab.running && onCancel ? (
              <Button variant="danger" size="xs" className="bg-danger/10" onClick={onCancel} title={`Cancel the running query (${mod}+.)`}>
                <Square size={9} fill="currentColor" /> Cancel
              </Button>
            ) : (
              <Button variant="primary" size="xs" onClick={() => onRun()} disabled={tab.running} title={`Run selection or all (${mod}+Enter)`}>
                <Play size={11} fill="currentColor" /> Run
              </Button>
            )}
            <Button variant="ghost" size="xs" onClick={askAi} title={`Ask AI (${mod}+I)`}>
              <Sparkles size={11} className="text-accent" /> Ask AI
            </Button>
            <Button variant="ghost" size="xs" onClick={explainWithAi} disabled={!tab.sql.trim()} title="Explain this query with AI">
              <BookOpen size={11} /> Explain
            </Button>
            <Button
              variant="ghost"
              size="xs"
              onClick={() => (tab.showPlan ? onChange({ showPlan: false }) : explainPlan())}
              disabled={!tab.sql.trim() || isRedis || isCql || conn?.kind === "mongodb"}
              title="Show the query plan (EXPLAIN)"
              className={cx(tab.showPlan && "bg-selected text-fg")}
            >
              <GanttChart size={11} /> Plan
            </Button>
            <Button
              variant="ghost"
              size="xs"
              onClick={() => setSaveDlg({ name: tab.snippetId ? tab.title : "", global: false })}
              disabled={!tab.sql.trim()}
              title="Save this query"
            >
              <Bookmark size={11} className={cx(tab.snippetId && "fill-current text-accent")} /> Save
            </Button>
            {onSchedule && (
              <Button variant="ghost" size="xs" onClick={() => onSchedule(tab.sql)} disabled={!tab.sql.trim()} title="Run this query on a schedule">
                <CalendarClock size={11} /> Schedule
              </Button>
            )}
            <ResultSwitcher count={results.length} idx={idx} onPick={(i) => onChange({ resultIndex: i })} />
            <span className={cx(results.length < 2 && "ml-auto")}>{exportMenu}</span>
            <Button variant="ghost" size="xs" onClick={onHistory} title={`Query history (${mod}+Y)`}>
              <History size={11} /> History
            </Button>
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
          <Button
            variant="ghost"
            size="xs"
            onClick={() => setShowFilters((f) => !f)}
            disabled={tab.view === "structure" || isRedis}
            className={cx((showFilters || (tab.filters?.length ?? 0) > 0) && "bg-selected text-fg")}
          >
            <FilterIcon size={11} /> Filter
            {(tab.filters?.length ?? 0) > 0 && (
              <span className="rounded bg-accent px-1 text-[10px] text-accent-fg">{tab.filters!.length}</span>
            )}
          </Button>
          <span className="truncate font-mono text-2xs text-subtle" title={tab.sql}>{tab.sql}</span>
          <span className="flex-1" />
          {tab.view !== "structure" && tab.total != null && (
            <Pager
              offset={tab.offset ?? 0}
              size={tab.pageSize ?? 300}
              total={tab.total}
              disabled={tab.running || pendingCount > 0}
              onPage={(offset) => onBrowse?.({ offset })}
            />
          )}
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

      {tab.kind === "table" && tab.view !== "structure" && showFilters && tab.columns && (
        <FilterBar columns={tab.columns} filters={tab.filters ?? []} onApply={(filters) => onBrowse?.({ filters, offset: 0 })} />
      )}

      <div className="relative min-h-0 flex-1">
        {tab.kind === "table" && tab.view === "structure" ? (
          conn && !conn.readOnly && !isRedis && !isCql && conn.kind !== "mongodb" && tab.columns && tab.schema && tab.table ? (
            <StructureEditor
              conn={conn}
              schema={tab.schema}
              table={tab.table}
              columns={tab.columns}
              onToast={onToast}
              onApplied={async () => {
                onChange({ columns: await api.describeTable(tab.connId, tab.schema!, tab.table!) });
                onRun();
              }}
            />
          ) : (
            <Structure tab={tab} />
          )
        ) : tab.kind === "query" && tab.showPlan ? (
          <PlanView
            plan={tab.plan}
            loading={tab.planLoading}
            error={tab.planError}
            canAnalyze={canAnalyze}
            analyze={analyze}
            onAnalyze={(v) => (setAnalyze(v), explainPlan(v))}
          />
        ) : tab.error && /cancelled$/i.test(tab.error.trim()) ? (
          <div className="flex h-full items-center justify-center text-xs text-muted">Query cancelled</div>
        ) : tab.error ? (
          <div className="selectable m-3 flex gap-2 rounded-lg border border-danger/30 bg-danger/5 p-3 font-mono text-xs text-danger">
            <AlertCircle size={14} className="mt-px shrink-0" />
            <pre className="flex-1 whitespace-pre-wrap">{tab.error}</pre>
            {tab.kind === "query" && (
              <Button size="xs" className="shrink-0 font-sans" onClick={fixWithAi}>
                <Wand2 size={11} className="text-accent" /> Fix with AI
              </Button>
            )}
          </div>
        ) : result ? (
          <ResultsGrid
            result={result}
            table={tab.kind === "table" && tab.table ? (tab.schema && tab.schema !== "main" ? `${tab.schema}.${tab.table}` : tab.table) : undefined}
            pkColumns={(tab.columns ?? [])
              .filter((c) => c.primary_key)
              .map((c) => result.columns.findIndex((x) => x.name === c.name))
              .filter((i) => i >= 0)}
            serverSort={tab.kind === "table" ? (tab.sort ?? null) : undefined}
            onServerSort={tab.kind === "table" && onBrowse ? (sort) => onBrowse({ sort, offset: 0 }) : undefined}
            edit={editable ? { pending: tab.pending ?? { edits: {}, deletes: [], inserts: [] }, onChange: (p) => onChange({ pending: p }) } : undefined}
          />
        ) : (
          <div className="flex h-full flex-col items-center justify-center gap-2 text-xs text-subtle">
            {tab.running ? (
              <>
                <span className="flex items-center gap-2 text-muted">
                  <span className="size-3 animate-spin rounded-full border-2 border-accent border-t-transparent" />
                  Running… <Elapsed since={tab.runStarted} />
                </span>
                {onCancel && (
                  <Button size="xs" onClick={onCancel}>
                    Cancel <Kbd>{mod}</Kbd>
                    <Kbd>.</Kbd>
                  </Button>
                )}
              </>
            ) : (
              "Run a query to see results"
            )}
          </div>
        )}
        {tab.running && <div className="absolute inset-x-0 top-0 h-0.5 animate-pulse bg-accent" />}
      </div>

      <Modal
        open={!!saveDlg}
        onClose={() => setSaveDlg(null)}
        width={420}
        title={tab.snippetId ? "Update saved query" : "Save query"}
        footer={
          <>
            <span className="flex-1" />
            <Button onClick={() => setSaveDlg(null)}>Cancel</Button>
            <Button variant="primary" onClick={saveSnippet} disabled={!saveDlg?.name.trim()}>
              Save
            </Button>
          </>
        }
      >
        {saveDlg && (
          <form className="flex flex-col gap-3" onSubmit={(e) => (e.preventDefault(), saveSnippet())}>
            <Field label="Name">
              <Input autoFocus value={saveDlg.name} placeholder="e.g. Revenue by month" onChange={(e) => setSaveDlg({ ...saveDlg, name: e.target.value })} />
            </Field>
            <Switch
              checked={saveDlg.global}
              onChange={(g) => setSaveDlg({ ...saveDlg, global: g })}
              label={<span className="text-xs">Show for all connections</span>}
            />
            <pre className="max-h-32 overflow-auto rounded-md bg-panel p-2 font-mono text-[11px] text-muted">{tab.sql}</pre>
          </form>
        )}
      </Modal>

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

function Pager({
  offset,
  size,
  total,
  disabled,
  onPage,
}: {
  offset: number;
  size: number;
  total: number;
  disabled?: boolean;
  onPage: (offset: number) => void;
}) {
  const from = total === 0 ? 0 : offset + 1;
  const to = Math.min(offset + size, total);
  return (
    <span className="flex items-center gap-0.5 text-2xs whitespace-nowrap text-muted tabular-nums">
      <Button variant="ghost" size="icon" disabled={disabled || offset === 0} onClick={() => onPage(Math.max(0, offset - size))} title="Previous page">
        <ChevronLeft size={13} />
      </Button>
      <span>
        {from.toLocaleString()}–{to.toLocaleString()} of {total.toLocaleString()}
      </span>
      <Button variant="ghost" size="icon" disabled={disabled || to >= total} onClick={() => onPage(offset + size)} title="Next page">
        <ChevronRight size={13} />
      </Button>
    </span>
  );
}

function Toolbar({ children }: { children: React.ReactNode }) {
  return <div className="flex h-9 shrink-0 items-center gap-2 overflow-x-auto border-b border-line px-2 whitespace-nowrap">{children}</div>;
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

/** Live "3.2 s" counter while a query runs. */
function Elapsed({ since }: { since?: number }) {
  const [, tick] = useState(0);
  useEffect(() => {
    const t = setInterval(() => tick((n) => n + 1), 200);
    return () => clearInterval(t);
  }, []);
  if (!since) return null;
  const s = (Date.now() - since) / 1000;
  return <span className="tabular-nums">{s < 60 ? `${s.toFixed(1)} s` : `${Math.floor(s / 60)}m ${Math.floor(s % 60)}s`}</span>;
}
