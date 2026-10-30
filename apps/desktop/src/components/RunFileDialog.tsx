import { AlertCircle, CheckCircle2, FileCode2, Square } from "lucide-react";
import { useEffect, useState } from "react";
import { api, dialogs, type ConnectionConfig, type ScriptProgress } from "../lib/api";
import { uid } from "../lib/types";
import { Button, Modal, Switch, cx } from "./ui";

function dur(ms: number) {
  if (ms < 1000) return `${ms} ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)} s`;
  return `${Math.floor(ms / 60_000)}m ${Math.floor((ms % 60_000) / 1000)}s`;
}

/** Run a .sql file statement by statement with progress, stop / continue on errors. */
export function RunFileDialog({
  conn,
  open,
  onClose,
  onDone,
}: {
  conn: ConnectionConfig | null;
  open: boolean;
  onClose: () => void;
  /** after a run that changed something (reload the table list) */
  onDone: (p: ScriptProgress) => void;
}) {
  const [path, setPath] = useState<string | null>(null);
  const [cont, setCont] = useState(false);
  const [p, setP] = useState<ScriptProgress | null>(null);
  const [runId, setRunId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    if (!open) return;
    setP(null);
    setError(null);
    setRunId(null);
  }, [open]);

  const pick = async () => {
    const f = await dialogs.openAny("SQL script", ["sql", "txt", "ddl"]);
    if (f) (setPath(f), setP(null), setError(null));
  };
  const run = async () => {
    if (!conn || !path) return;
    const id = uid();
    setRunId(id);
    setError(null);
    setP({ total: 0, done: 0, failed: 0, rowsAffected: 0, elapsedMs: 0, finished: false, cancelled: false, errors: [], current: "Reading the file…" });
    try {
      const r = await api.runSqlFile(conn.id, path, cont, id, setP);
      setP(r);
      onDone(r);
    } catch (e) {
      setError(String(e));
    } finally {
      setRunId(null);
    }
  };

  const running = !!runId;
  const pct = p && p.total ? Math.round((p.done / p.total) * 100) : 0;
  const name = path?.split(/[\\/]/).pop();

  return (
    <Modal
      open={open}
      onClose={() => !running && onClose()}
      width={640}
      title={
        <span className="flex items-center gap-2">
          <FileCode2 size={14} className="text-accent" /> Run SQL file{conn ? <span className="font-normal text-muted">· {conn.name}</span> : null}
        </span>
      }
      footer={
        <>
          <Switch checked={cont} onChange={setCont} label={<span className="text-2xs">Keep going after errors</span>} />
          <span className="flex-1" />
          {running ? (
            <Button onClick={() => runId && api.cancelSqlFile(runId)}>
              <Square size={11} /> Stop
            </Button>
          ) : (
            <>
              <Button variant="ghost" onClick={onClose}>
                Close
              </Button>
              <Button variant="primary" disabled={!path || conn?.readOnly} onClick={run}>
                {p?.finished ? "Run again" : "Run"}
              </Button>
            </>
          )}
        </>
      }
    >
      <div className="flex flex-col gap-3">
        <div className="flex items-center gap-2">
          <div className="min-w-0 flex-1 truncate rounded-md border border-line bg-panel px-2.5 py-1.5 font-mono text-[11.5px]" title={path ?? undefined}>
            {path ?? <span className="text-subtle">No file chosen</span>}
          </div>
          <Button disabled={running} onClick={pick}>
            Choose…
          </Button>
        </div>
        {conn?.readOnly && <p className="text-2xs text-warning">This connection is read-only — scripts that change data are blocked.</p>}
        {!p && !error && (
          <p className="text-2xs text-subtle">
            Statements run one at a time, so big seed files and migrations work without loading everything into the editor. MySQL <code className="font-mono">DELIMITER</code> blocks
            and Postgres <code className="font-mono">$$</code> bodies are handled; use Tools → Restore for <code className="font-mono">pg_dump</code> files with COPY data.
          </p>
        )}
        {error && <div className="selectable rounded-lg border border-danger/30 bg-danger/5 p-2.5 font-mono text-2xs text-danger">{error}</div>}
        {p && (
          <>
            <div>
              <div className="mb-1 flex items-baseline gap-2 text-xs">
                {p.finished ? (
                  p.failed ? (
                    <AlertCircle size={13} className="self-center text-warning" />
                  ) : (
                    <CheckCircle2 size={13} className="self-center text-success" />
                  )
                ) : null}
                <span className="font-medium">
                  {p.finished
                    ? p.cancelled
                      ? `Stopped after ${p.done.toLocaleString()} of ${p.total.toLocaleString()} statements`
                      : p.failed && p.done < p.total
                        ? `Stopped at an error — ${p.done.toLocaleString()} of ${p.total.toLocaleString()} statements`
                        : `Ran ${p.done.toLocaleString()} statements from ${name}`
                    : p.total
                      ? `${p.done.toLocaleString()} / ${p.total.toLocaleString()} statements`
                      : "Starting…"}
                </span>
                <span className="ml-auto text-2xs text-muted tabular-nums">
                  {p.rowsAffected.toLocaleString()} rows · {dur(p.elapsedMs)}
                  {p.failed ? ` · ${p.failed} failed` : ""}
                </span>
              </div>
              <div className="h-1.5 overflow-hidden rounded-full bg-hover">
                <div className={cx("h-full rounded-full transition-[width] duration-150", p.failed ? "bg-warning" : "bg-accent")} style={{ width: `${p.finished && !p.failed && !p.cancelled ? 100 : pct}%` }} />
              </div>
              {p.current && !p.finished && <div className="mt-1.5 truncate font-mono text-[11px] text-subtle">{p.current}</div>}
            </div>
            {p.errors.length > 0 && (
              <div className="max-h-56 overflow-y-auto rounded-lg border border-line">
                {p.errors.map((e) => (
                  <div key={e.index} className="border-b border-line/60 px-3 py-2 last:border-0">
                    <div className="flex gap-2 text-2xs text-muted">
                      <span>Statement {e.index + 1}</span>
                      {e.line > 0 && <span>· line {e.line}</span>}
                    </div>
                    <div className="selectable truncate font-mono text-[11px] text-fg/80" title={e.statement}>
                      {e.statement}
                    </div>
                    <div className="selectable mt-0.5 text-2xs text-danger">{e.message}</div>
                  </div>
                ))}
              </div>
            )}
          </>
        )}
      </div>
    </Modal>
  );
}
