import { Ban, Pause, Play, Power } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { api, type ConnectionConfig, type ServerSession } from "../lib/api";
import { Button, Modal, Switch, cx } from "./ui";

function dur(s: number | null) {
  if (s == null) return "";
  if (s < 1) return `${Math.round(s * 1000)} ms`;
  if (s < 60) return `${s.toFixed(1)} s`;
  if (s < 3600) return `${Math.floor(s / 60)}m ${Math.floor(s % 60)}s`;
  return `${Math.floor(s / 3600)}h ${Math.floor((s % 3600) / 60)}m`;
}

/** What's running on the server right now, refreshed every 2 s, with cancel / kill. */
export function ActivityDialog({ conn, open, onClose, onToast }: { conn: ConnectionConfig | null; open: boolean; onClose: () => void; onToast: (m: string, k?: "ok" | "err") => void }) {
  const [rows, setRows] = useState<ServerSession[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [live, setLive] = useState(true);
  const [hideIdle, setHideIdle] = useState(true);
  const [confirm, setConfirm] = useState<string | null>(null);

  const load = useCallback(() => {
    if (!conn) return;
    api
      .serverActivity(conn.id)
      .then((r) => (setRows(r), setError(null)))
      .catch((e) => setError(String(e)));
  }, [conn]);

  useEffect(() => {
    if (!open) return;
    setRows(null);
    setConfirm(null);
    load();
  }, [open, load]);
  useEffect(() => {
    if (!open || !live) return;
    const t = setInterval(load, 2000);
    return () => clearInterval(t);
  }, [open, live, load]);

  const kill = async (s: ServerSession, terminate: boolean) => {
    if (!conn) return;
    try {
      await api.killSession(conn.id, s.id, terminate);
      onToast(terminate ? `Session ${s.id} terminated` : `Query in ${s.id} cancelled`, "ok");
      setConfirm(null);
      load();
    } catch (e) {
      onToast(String(e));
    }
  };

  const idle = (s: ServerSession) => /^(idle|sleep)$/i.test(s.state.trim());
  const shown = (rows ?? []).filter((s) => !hideIdle || !idle(s));
  const longRunning = (s: ServerSession) => !idle(s) && (s.seconds ?? 0) > 30;

  return (
    <Modal
      open={open}
      onClose={onClose}
      width={980}
      title={`Server activity${conn ? ` · ${conn.name}` : ""}`}
      footer={
        <>
          <Switch checked={hideIdle} onChange={setHideIdle} label={<span className="text-2xs">Hide idle sessions</span>} />
          <span className="flex-1 text-2xs text-subtle">
            {rows ? `${rows.length} session${rows.length === 1 ? "" : "s"} · ${rows.filter((r) => !idle(r)).length} active` : ""}
          </span>
          <Button size="sm" variant="ghost" onClick={() => setLive((l) => !l)} title={live ? "Pause refresh" : "Resume refresh"}>
            {live ? <Pause size={12} /> : <Play size={12} />} {live ? "Live" : "Paused"}
          </Button>
          <Button onClick={onClose}>Close</Button>
        </>
      }
    >
      {error ? (
        <div className="selectable rounded-lg border border-danger/30 bg-danger/5 p-3 font-mono text-2xs text-danger">{error}</div>
      ) : (
        <div className="-mx-4 -my-3 max-h-[60vh] overflow-auto">
          <table className="w-full table-fixed border-collapse text-xs">
            <thead className="sticky top-0 bg-panel text-left text-2xs text-muted">
              <tr className="h-7 border-b border-line">
                <th className="w-20 px-3 font-medium">ID</th>
                <th className="w-24 px-2 font-medium">User</th>
                <th className="w-24 px-2 font-medium">Database</th>
                <th className="w-24 px-2 font-medium">Client</th>
                <th className="w-36 px-2 font-medium">State</th>
                <th className="w-20 px-2 text-right font-medium">Time</th>
                <th className="px-2 font-medium">Query</th>
                <th className="w-24" />
              </tr>
            </thead>
            <tbody>
              {rows === null && (
                <tr>
                  <td colSpan={8} className="p-6 text-center text-subtle">
                    Loading…
                  </td>
                </tr>
              )}
              {rows && shown.length === 0 && (
                <tr>
                  <td colSpan={8} className="p-6 text-center text-subtle">
                    Nothing running
                  </td>
                </tr>
              )}
              {shown.map((s) => (
                <tr key={s.id} className={cx("h-8 border-b border-line/60 align-middle", s.own && "text-muted")}>
                  <td className="px-3 font-mono text-2xs">{s.id}</td>
                  <td className="truncate px-2">{s.user}</td>
                  <td className="truncate px-2">{s.database}</td>
                  <td className="max-w-28 truncate px-2 text-muted">{s.client}</td>
                  <td className="px-2">
                    <span
                      className={cx(
                        "rounded px-1.5 py-px text-[10.5px]",
                        /active|running|query|execute/i.test(s.state) ? "bg-success/12 text-success" : /transaction/i.test(s.state) ? "bg-warning/15 text-warning" : "bg-hover text-muted",
                      )}
                    >
                      {s.state || "—"}
                    </span>
                  </td>
                  <td className={cx("px-2 text-right tabular-nums", longRunning(s) && "font-medium text-warning")}>{dur(s.seconds)}</td>
                  <td className="max-w-0 px-2">
                    <div className="truncate font-mono text-[11px]" title={s.query}>
                      {s.query}
                    </div>
                  </td>
                  <td className="px-2 text-right whitespace-nowrap">
                    {s.own ? (
                      <span className="text-2xs text-subtle">this app</span>
                    ) : confirm === s.id ? (
                      <span className="flex justify-end gap-1">
                        <Button size="xs" variant="danger" onClick={() => kill(s, true)}>
                          Kill session
                        </Button>
                        <Button size="xs" variant="ghost" onClick={() => setConfirm(null)}>
                          No
                        </Button>
                      </span>
                    ) : (
                      <span className="flex justify-end gap-0.5">
                        {!idle(s) && (
                          <Button size="icon" variant="ghost" title="Cancel the running query" onClick={() => kill(s, false)} disabled={conn?.readOnly}>
                            <Ban size={12} />
                          </Button>
                        )}
                        <Button size="icon" variant="ghost" title="Terminate the session" onClick={() => setConfirm(s.id)} disabled={conn?.readOnly}>
                          <Power size={12} className="text-danger" />
                        </Button>
                      </span>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </Modal>
  );
}
