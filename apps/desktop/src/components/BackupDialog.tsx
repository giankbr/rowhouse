import { AlertCircle, CheckCircle2, FolderOpen, Loader2 } from "lucide-react";
import { useEffect, useState } from "react";
import { api, dialogs, kindInfo, type ConnectionConfig, type DumpOptions, type DumpReport, type TableInfo } from "../lib/api";
import { Button, Modal, Segmented, cx } from "./ui";

function size(n: number) {
  if (n < 1024) return `${n} B`;
  if (n < 1024 ** 2) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 ** 3) return `${(n / 1024 ** 2).toFixed(1)} MB`;
  return `${(n / 1024 ** 3).toFixed(2)} GB`;
}

/** Backup (pg_dump / mysqldump / VACUUM INTO) or restore the connection's database. */
export function BackupDialog({
  mode,
  conn,
  tables,
  onClose,
  onRestored,
}: {
  mode: "backup" | "restore" | null;
  conn: ConnectionConfig | null;
  tables: TableInfo[];
  onClose: () => void;
  onRestored: () => void;
}) {
  const [opts, setOpts] = useState<DumpOptions>({ format: "sql", content: "all", tables: [] });
  const [path, setPath] = useState("");
  const [tools, setTools] = useState<[string, boolean][]>([]);
  const [state, setState] = useState<{ busy?: boolean; done?: DumpReport; error?: string }>({});
  const [confirmText, setConfirmText] = useState("");
  const d = conn ? kindInfo(conn.kind).dialect : "postgres";
  const isPg = d === "postgres";
  const db = conn?.database || conn?.file?.split(/[\\/]/).pop() || "database";

  useEffect(() => {
    if (!mode || !conn) return;
    setState({});
    setPath("");
    setConfirmText("");
    setOpts({ format: "sql", content: "all", tables: [] });
    api.dumpTools(conn.id).then(setTools).catch(() => setTools([]));
  }, [mode, conn]);

  if (!conn) return null;
  const missing = tools.filter(([, ok]) => !ok).map(([n]) => n);
  const stamp = new Date().toISOString().slice(0, 16).replace(/[-:T]/g, "").replace(/(\d{8})(\d{4})/, "$1-$2");
  const ext = d === "sqlite" ? "db" : opts.format === "custom" ? "dump" : "sql";

  const pickPath = async () => {
    if (mode === "backup") {
      const p = await dialogs.save(`${db}-${stamp}.${ext}`, ext);
      if (p) setPath(p);
    } else {
      const p = await dialogs.openAny("Backup", ["sql", "dump", "backup"]);
      if (p) setPath(p);
    }
  };

  const run = async () => {
    if (!path) return;
    setState({ busy: true });
    try {
      const done = mode === "backup" ? await api.backupDatabase(conn.id, path, opts) : await api.restoreDatabase(conn.id, path);
      setState({ done });
      if (mode === "restore") onRestored();
    } catch (e) {
      setState({ error: String(e) });
    }
  };

  const prod = conn.env === "production";
  const needsConfirm = mode === "restore" && prod;

  return (
    <Modal
      open={!!mode}
      onClose={() => !state.busy && onClose()}
      width={520}
      title={mode === "backup" ? `Back up ${db}` : `Restore into ${db}`}
      footer={
        <>
          <span className="flex-1 text-2xs text-subtle">
            {tools.length ? `Uses ${tools.map(([n]) => n).join(" / ")} on this computer` : d === "sqlite" ? "Copies the file with VACUUM INTO" : ""}
          </span>
          <Button onClick={onClose} disabled={state.busy}>
            {state.done ? "Done" : "Cancel"}
          </Button>
          {!state.done && (
            <Button
              variant={mode === "restore" ? "danger" : "primary"}
              className={cx(mode === "restore" && "bg-danger text-white hover:bg-danger/90")}
              onClick={run}
              disabled={!path || state.busy || (needsConfirm && confirmText !== db) || (mode === "restore" && conn.readOnly)}
            >
              {state.busy ? <Loader2 size={12} className="animate-spin" /> : null}
              {mode === "backup" ? "Back up" : "Restore"}
            </Button>
          )}
        </>
      }
    >
      <div className="flex flex-col gap-3 text-xs">
        {missing.length > 0 && (
          <div className="flex gap-2 rounded-lg border border-warning/30 bg-warning/8 p-2.5 text-2xs text-warning">
            <AlertCircle size={13} className="shrink-0" />
            <span>
              {missing.join(", ")} not found on this computer.{" "}
              {isPg ? "Install the PostgreSQL client tools (macOS: brew install libpq)." : "Install the MySQL client tools (macOS: brew install mysql-client)."}
            </span>
          </div>
        )}
        {mode === "backup" && (
          <>
            {isPg && (
              <Row label="Format">
                <Segmented
                  value={opts.format}
                  onChange={(format) => setOpts({ ...opts, format })}
                  options={[
                    { value: "sql", label: "SQL script" },
                    { value: "custom", label: "Custom archive (pg_restore)" },
                  ]}
                />
              </Row>
            )}
            {d !== "sqlite" && d !== "duckdb" && (
              <Row label="Include">
                <Segmented
                  value={opts.content}
                  onChange={(content) => setOpts({ ...opts, content })}
                  options={[
                    { value: "all", label: "Schema + data" },
                    { value: "schema-only", label: "Schema only" },
                    { value: "data-only", label: "Data only" },
                  ]}
                />
              </Row>
            )}
            {d !== "sqlite" && d !== "duckdb" && (
              <Row label="Tables">
                <select
                  multiple
                  value={opts.tables}
                  onChange={(e) => setOpts({ ...opts, tables: [...e.target.selectedOptions].map((o) => o.value) })}
                  className="h-24 w-full rounded-md border border-line bg-elevated px-1 font-mono text-2xs outline-none focus:border-accent"
                >
                  {tables
                    .filter((t) => t.kind === "table")
                    .map((t) => (
                      <option key={`${t.schema}.${t.name}`} value={`${t.schema}.${t.name}`}>
                        {t.schema}.{t.name}
                      </option>
                    ))}
                </select>
              </Row>
            )}
            {d !== "sqlite" && d !== "duckdb" && <span className="-mt-2 pl-[88px] text-2xs text-subtle">{opts.tables.length ? `${opts.tables.length} selected` : "None selected = whole database"}</span>}
          </>
        )}
        {mode === "restore" && (
          <div className="flex gap-2 rounded-lg border border-danger/30 bg-danger/5 p-2.5 text-2xs text-danger">
            <AlertCircle size={13} className="shrink-0" />
            <span>
              Runs the backup against <b>{db}</b>{conn.host ? ` on ${conn.host}` : ""}. Existing objects with the same names may make it stop, and statements that already ran
              are not rolled back. {conn.readOnly && "This connection is read-only."}
            </span>
          </div>
        )}
        <Row label={mode === "backup" ? "Save to" : "File"}>
          <div className="flex flex-1 gap-1.5">
            <input
              value={path}
              onChange={(e) => setPath(e.target.value)}
              placeholder={mode === "backup" ? `${db}-${stamp}.${ext}` : "backup .sql / .dump"}
              className="h-7 flex-1 rounded-md border border-line bg-elevated px-2 font-mono text-2xs outline-none focus:border-accent"
            />
            <Button size="sm" onClick={pickPath}>
              <FolderOpen size={12} />
            </Button>
          </div>
        </Row>
        {needsConfirm && (
          <Row label="Confirm">
            <input
              value={confirmText}
              onChange={(e) => setConfirmText(e.target.value)}
              placeholder={`type ${db} to restore into production`}
              className="h-7 flex-1 rounded-md border border-danger/40 bg-elevated px-2 font-mono text-2xs outline-none focus:border-danger"
            />
          </Row>
        )}
        {state.done && (
          <div className="flex items-center gap-2 rounded-lg bg-success/10 px-3 py-2 text-2xs text-success">
            <CheckCircle2 size={13} />
            {mode === "backup" ? "Backed up" : "Restored"} · {size(state.done.bytes)} · {(state.done.elapsedMs / 1000).toFixed(1)} s · {state.done.tool}
          </div>
        )}
        {state.done?.log && <pre className="selectable max-h-28 overflow-auto rounded-md bg-panel p-2 font-mono text-[10.5px] text-muted">{state.done.log}</pre>}
        {state.error && <pre className="selectable max-h-40 overflow-auto rounded-lg border border-danger/30 bg-danger/5 p-2.5 font-mono text-2xs whitespace-pre-wrap text-danger">{state.error}</pre>}
      </div>
    </Modal>
  );
}

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex items-start gap-3">
      <span className="w-20 shrink-0 pt-1.5 text-muted">{label}</span>
      <div className="flex min-w-0 flex-1">{children}</div>
    </div>
  );
}
