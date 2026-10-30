import { CalendarClock, FolderOpen, Play, Plus, Trash2 } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import {
  api,
  describeEvery,
  dialogs,
  nextRun,
  type ConnectionConfig,
  type Every,
  type ExportFormat,
  type Notify,
  type Schedule,
} from "../lib/api";
import { KindBadge } from "./KindBadge";
import { Button, Field, Input, Modal, Segmented, Switch, cx } from "./ui";

const DAYS = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

function blank(connectionId: string, sql = ""): Schedule {
  return {
    id: "",
    name: "",
    connectionId,
    sql,
    every: { kind: "daily", at: "08:00" },
    enabled: true,
    export: null,
    notify: "on-failure",
    createdAt: 0,
    runs: [],
  };
}

function ago(ts: number) {
  const d = Math.floor(Date.now() / 1000) - ts;
  if (d < 60) return "just now";
  if (d < 3600) return `${Math.floor(d / 60)} min ago`;
  if (d < 86400) return `${Math.floor(d / 3600)} h ago`;
  return `${Math.floor(d / 86400)} d ago`;
}

function when(ts: number | null) {
  if (ts == null) return "paused";
  const d = new Date(ts * 1000);
  const today = new Date();
  const same = d.toDateString() === today.toDateString();
  const t = d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  if (ts * 1000 <= Date.now()) return "due now";
  return same ? `today ${t}` : `${d.toLocaleDateString([], { weekday: "short", day: "numeric", month: "short" })} ${t}`;
}

export function SchedulesDialog({
  open,
  onClose,
  connections,
  draft,
}: {
  open: boolean;
  onClose: () => void;
  connections: ConnectionConfig[];
  /** pre-filled new schedule (from "Schedule…" in a query tab) */
  draft?: { connectionId: string; sql: string; name?: string } | null;
}) {
  const [list, setList] = useState<Schedule[]>([]);
  const [sel, setSel] = useState<Schedule | null>(null);
  const [busy, setBusy] = useState<"save" | "run" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [, tick] = useState(0);

  const reload = useCallback(() => api.schedules.list().then(setList), []);
  const [pro, setPro] = useState(true);
  useEffect(() => {
    if (open) api.license.status().then((s) => setPro(s.pro)).catch(() => {});
  }, [open]);

  useEffect(() => {
    if (!open) return;
    setError(null);
    reload().then(() => {
      if (draft) setSel({ ...blank(draft.connectionId, draft.sql), name: draft.name ?? "" });
    });
    const t = setInterval(() => tick((n) => n + 1), 30_000);
    let off: (() => void) | undefined;
    api.schedules
      .onRun(({ schedule }) => {
        setList((l) => l.map((x) => (x.id === schedule.id ? schedule : x)));
        setSel((s) => (s && s.id === schedule.id ? { ...s, runs: schedule.runs } : s));
      })
      .then((f) => (off = f));
    return () => {
      clearInterval(t);
      off?.();
    };
  }, [open, draft, reload]);

  useEffect(() => {
    if (open && !draft && !sel && list.length) setSel(list[0]);
  }, [open, draft, sel, list]);

  const conn = (id: string) => connections.find((c) => c.id === id);
  const dirty = useMemo(() => {
    if (!sel) return false;
    const cur = list.find((x) => x.id === sel.id);
    if (!cur) return true;
    const strip = (s: Schedule) => JSON.stringify({ ...s, runs: [], createdAt: 0 });
    return strip(cur) !== strip(sel);
  }, [sel, list]);

  const set = (patch: Partial<Schedule>) => setSel((s) => (s ? { ...s, ...patch } : s));
  const setEvery = (every: Every) => set({ every });

  const save = async () => {
    if (!sel) return;
    setBusy("save");
    setError(null);
    try {
      const saved = await api.schedules.save(sel);
      await reload();
      setSel(saved);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  };

  const runNow = async () => {
    if (!sel?.id) return;
    setBusy("run");
    setError(null);
    try {
      await api.schedules.runNow(sel.id);
      const fresh = await api.schedules.list();
      setList(fresh);
      const s = fresh.find((x) => x.id === sel.id);
      if (s) setSel((cur) => (cur ? { ...cur, runs: s.runs } : cur));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  };

  const remove = async () => {
    if (!sel?.id) return setSel(null);
    await api.schedules.remove(sel.id);
    setSel(null);
    reload();
  };

  const e = sel?.every;
  const firstConn = connections[0]?.id ?? "";

  return (
    <Modal
      open={open}
      onClose={onClose}
      width={900}
      title="Scheduled queries"
      footer={
        <>
          <span className="flex-1 text-2xs text-subtle">Schedules run while Rowhouse is open · results and failures can notify you</span>
          {sel && (
            <>
              <Button variant="ghost" onClick={remove} title="Delete schedule">
                <Trash2 size={12} />
              </Button>
              <Button onClick={runNow} disabled={!sel.id || dirty || busy !== null} title={dirty ? "Save first" : "Run now"}>
                <Play size={11} /> {busy === "run" ? "Running…" : "Run now"}
              </Button>
              <Button variant="primary" onClick={save} disabled={!dirty || busy !== null || !sel.name.trim() || !sel.sql.trim()}>
                {busy === "save" ? "Saving…" : sel.id ? "Save" : "Create"}
              </Button>
            </>
          )}
          {!sel && <Button onClick={onClose}>Done</Button>}
        </>
      }
    >
      <div className="-m-4 flex h-[560px] min-h-0">
        {/* list */}
        <div className="flex w-64 shrink-0 flex-col border-r border-line">
          <div className="flex items-center justify-between px-3 pt-3 pb-2">
            <span className="text-2xs font-medium tracking-wide text-subtle uppercase">{list.length} schedules</span>
            <Button size="xs" variant="ghost" onClick={() => setSel(blank(firstConn))} title="New schedule">
              <Plus size={12} /> New
            </Button>
          </div>
          <div className="flex-1 overflow-y-auto px-2 pb-2">
            {list.length === 0 && !sel && (
              <div className="flex flex-col items-center gap-2 px-4 pt-16 text-center text-xs text-muted">
                <CalendarClock size={22} className="text-subtle" />
                No schedules yet.
                <span className="text-2xs text-subtle">Run a report every morning, watch a table for new rows, or export a CSV on a timer.</span>
              </div>
            )}
            {list.map((s) => {
              const last = s.runs[0];
              const c = conn(s.connectionId);
              return (
                <button
                  key={s.id}
                  onClick={() => setSel(s)}
                  className={cx(
                    "flex w-full flex-col gap-0.5 rounded-lg px-2.5 py-2 text-left",
                    sel?.id === s.id ? "bg-selected" : "hover:bg-hover",
                  )}
                >
                  <span className="flex items-center gap-1.5">
                    <span
                      className={cx(
                        "size-1.5 shrink-0 rounded-full",
                        !s.enabled ? "bg-subtle" : !last ? "bg-line-strong" : last.ok ? "bg-success" : "bg-danger",
                      )}
                    />
                    <span className={cx("truncate text-xs font-medium", !s.enabled && "text-muted")}>{s.name}</span>
                  </span>
                  <span className="truncate pl-3 text-2xs text-muted">
                    {describeEvery(s.every)} · {c?.name ?? "missing connection"}
                  </span>
                  <span className="pl-3 text-2xs text-subtle">next {when(nextRun(s))}</span>
                </button>
              );
            })}
          </div>
        </div>

        {/* editor */}
        {sel ? (
          <div className="flex min-w-0 flex-1 flex-col overflow-y-auto p-4">
            <div className="flex flex-col gap-3">
              <div className="grid grid-cols-[1fr_220px] gap-2">
                <Field label="Name">
                  <Input value={sel.name} placeholder="e.g. Daily revenue" onChange={(ev) => set({ name: ev.target.value })} autoFocus={!sel.id} />
                </Field>
                <Field label="Connection">
                  <div className="relative">
                    <span className="pointer-events-none absolute top-1/2 left-2 -translate-y-1/2">
                      {conn(sel.connectionId) && <KindBadge kind={conn(sel.connectionId)!.kind} size={16} />}
                    </span>
                    <select
                      value={sel.connectionId}
                      onChange={(ev) => set({ connectionId: ev.target.value })}
                      className="h-7 w-full appearance-none rounded-md border border-line bg-elevated pr-2 pl-7 text-xs outline-none focus:border-accent"
                    >
                      {connections.map((c) => (
                        <option key={c.id} value={c.id}>
                          {c.name}
                        </option>
                      ))}
                    </select>
                  </div>
                </Field>
              </div>

              <Field label="SQL" hint="Runs with the connection's settings (read-only connections stay read-only). DROP / TRUNCATE / unfiltered DELETE are refused.">
                <textarea
                  value={sel.sql}
                  onChange={(ev) => set({ sql: ev.target.value })}
                  spellCheck={false}
                  rows={5}
                  className="w-full resize-y rounded-md border border-line bg-elevated px-2.5 py-2 font-mono text-[11.5px] leading-relaxed outline-none focus:border-accent"
                />
              </Field>

              <div className="flex flex-col gap-2 rounded-lg border border-line p-3">
                <div className="flex items-center gap-3">
                  <span className="w-16 text-xs text-muted">Runs</span>
                  <Segmented
                    value={e!.kind}
                    onChange={(k) =>
                      setEvery(
                        k === "minutes"
                          ? { kind: "minutes", n: 60 }
                          : k === "daily"
                            ? { kind: "daily", at: "at" in e! ? e.at : "08:00" }
                            : { kind: "weekly", weekday: 0, at: "at" in e! ? e.at : "08:00" },
                      )
                    }
                    options={[
                      { value: "minutes", label: "Interval" },
                      { value: "daily", label: "Daily" },
                      { value: "weekly", label: "Weekly" },
                    ]}
                  />
                  {e!.kind === "minutes" && (
                    <span className="flex items-center gap-1.5 text-xs text-muted">
                      every
                      <Input
                        className="w-16"
                        inputMode="numeric"
                        value={String(e!.n)}
                        onChange={(ev) => setEvery({ kind: "minutes", n: Math.max(1, Number(ev.target.value.replace(/\D/g, "")) || 1) })}
                      />
                      min
                    </span>
                  )}
                  {e!.kind !== "minutes" && (
                    <Input
                      type="time"
                      className="w-28"
                      value={e!.at}
                      onChange={(ev) => setEvery({ ...(e as Exclude<Every, { kind: "minutes" }>), at: ev.target.value || "08:00" })}
                    />
                  )}
                </div>
                {e!.kind === "weekly" && (
                  <div className="flex items-center gap-3">
                    <span className="w-16 text-xs text-muted">On</span>
                    <Segmented
                      value={String(e!.weekday)}
                      onChange={(d) => setEvery({ kind: "weekly", weekday: Number(d), at: e!.at })}
                      options={DAYS.map((d, i) => ({ value: String(i), label: d }))}
                    />
                  </div>
                )}
                <div className="flex items-center gap-3">
                  <span className="w-16 text-xs text-muted">Notify</span>
                  <Segmented<Notify>
                    value={sel.notify}
                    onChange={(notify) => set({ notify })}
                    options={[
                      { value: "never", label: "Never" },
                      { value: "on-failure", label: "On failure" },
                      { value: "on-change", label: "When result changes" },
                      { value: "always", label: "Every run" },
                    ]}
                  />
                </div>
                <div className="flex items-center gap-3">
                  <span className="w-16 text-xs text-muted">Export</span>
                  <Switch
                    checked={!!sel.export}
                    onChange={(on) => set({ export: on ? { dir: sel.export?.dir ?? "", format: "csv" } : null })}
                    label={<span className="text-xs">Save each result to a file</span>}
                  />
                </div>
                {sel.export && (
                  <div className="flex items-center gap-2 pl-[76px]">
                    <Input
                      className="flex-1"
                      value={sel.export.dir}
                      placeholder="Folder"
                      onChange={(ev) => set({ export: { ...sel.export!, dir: ev.target.value } })}
                    />
                    <Button
                      size="sm"
                      onClick={async () => {
                        const d = await dialogs.folder();
                        if (d) set({ export: { ...sel.export!, dir: d } });
                      }}
                    >
                      <FolderOpen size={12} />
                    </Button>
                    <Segmented<ExportFormat>
                      value={sel.export.format}
                      onChange={(format) => set({ export: { ...sel.export!, format } })}
                      options={[
                        { value: "csv", label: "CSV" },
                        { value: "json", label: "JSON" },
                      ]}
                    />
                  </div>
                )}
                <div className="flex items-center gap-3">
                  <span className="w-16 text-xs text-muted">Status</span>
                  <Switch checked={sel.enabled} onChange={(enabled) => set({ enabled })} label={<span className="text-xs">{sel.enabled ? "Active" : "Paused"}</span>} />
                  {sel.id && !dirty && <span className="ml-auto text-2xs text-subtle">Next run {when(nextRun(sel))}</span>}
                </div>
              </div>

              {!pro && (
                <div className="rounded-md bg-accent-soft px-3 py-2 text-xs">
                  Scheduled queries are part of <b>Rowhouse Pro</b>. You can set them up now; they run once a license is added in Settings → License.
                </div>
              )}
              {error && <div className="rounded-md bg-danger/10 px-3 py-2 text-xs text-danger">{error}</div>}

              {sel.runs.length > 0 && (
                <div className="flex flex-col">
                  <span className="mb-1.5 text-2xs font-medium tracking-wide text-subtle uppercase">Recent runs</span>
                  <div className="overflow-hidden rounded-lg border border-line">
                    {sel.runs.map((r, i) => (
                      <div key={i} className="flex items-center gap-3 border-b border-line px-3 py-1.5 text-xs last:border-b-0">
                        <span className={cx("size-1.5 shrink-0 rounded-full", r.ok ? "bg-success" : "bg-danger")} />
                        <span className="w-20 shrink-0 text-muted" title={new Date(r.at * 1000).toLocaleString()}>
                          {ago(r.at)}
                        </span>
                        {r.ok ? (
                          <>
                            <span className="w-16 shrink-0 tabular-nums">{r.rows} rows</span>
                            <span className="w-14 shrink-0 text-subtle tabular-nums">{r.elapsedMs} ms</span>
                            {r.changed && <span className="rounded bg-accent/12 px-1.5 py-px text-2xs font-medium text-accent">changed</span>}
                            {r.file && <span className="min-w-0 truncate font-mono text-2xs text-subtle" title={r.file}>{r.file.split(/[\\/]/).pop()}</span>}
                          </>
                        ) : (
                          <span className="min-w-0 truncate text-danger" title={r.error ?? ""}>
                            {r.error}
                          </span>
                        )}
                      </div>
                    ))}
                  </div>
                </div>
              )}
            </div>
          </div>
        ) : (
          <div className="flex flex-1 items-center justify-center text-xs text-subtle">Pick a schedule or create one</div>
        )}
      </div>
    </Modal>
  );
}
