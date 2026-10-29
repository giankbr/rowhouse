import { AlertCircle, KeyRound, Play, RefreshCw } from "lucide-react";
import { useCallback, useRef, useState } from "react";
import type { ConnectionConfig, TableInfo } from "../lib/api";
import { mod, type Tab } from "../lib/types";
import { ResultsGrid } from "./ResultsGrid";
import { SqlEditor } from "./SqlEditor";
import { Button, Kbd, Segmented, cx } from "./ui";

export function Workspace({
  tab,
  conn,
  tables,
  onChange,
  onRun,
  onView,
}: {
  tab: Tab;
  conn: ConnectionConfig | undefined;
  tables: TableInfo[];
  onChange: (patch: Partial<Tab>) => void;
  onRun: (sql?: string) => void;
  onView: (v: "data" | "structure") => void;
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
          <Button variant="ghost" size="xs" onClick={() => onRun()} disabled={tab.running}>
            <RefreshCw size={11} className={cx(tab.running && "animate-spin")} /> Refresh
          </Button>
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
          <ResultsGrid result={result} />
        ) : (
          <div className="flex h-full items-center justify-center text-xs text-subtle">
            {tab.running ? "Running…" : "Run a query to see results"}
          </div>
        )}
        {tab.running && <div className="absolute inset-x-0 top-0 h-0.5 animate-pulse bg-accent" />}
      </div>
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
