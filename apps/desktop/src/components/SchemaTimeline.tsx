import { Camera, Copy, History, Loader2, Minus, PencilLine, Plus, SquareTerminal } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { api, type ConnectionConfig, type SchemaChange, type TimelineDiff, type TimelineEntry } from "../lib/api";
import { describeChange } from "./CompareView";
import { Button, cx } from "./ui";

const META: Record<SchemaChange["kind"], { icon: typeof Plus; cls: string; text: string }> = {
  tableAdded: { icon: Plus, cls: "text-success", text: "table added" },
  columnAdded: { icon: Plus, cls: "text-success", text: "column added" },
  columnChanged: { icon: PencilLine, cls: "text-warning", text: "changed" },
  columnRemoved: { icon: Minus, cls: "text-danger", text: "column dropped" },
  tableRemoved: { icon: Minus, cls: "text-danger", text: "table dropped" },
};

const when = (at: number) => new Date(at * 1000).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });

function summary(e: TimelineEntry) {
  if (e.initial) return `First snapshot · ${e.tables} tables`;
  const p = [
    e.tablesAdded && `+${e.tablesAdded} table${e.tablesAdded > 1 ? "s" : ""}`,
    e.tablesRemoved && `−${e.tablesRemoved} table${e.tablesRemoved > 1 ? "s" : ""}`,
    e.columnsAdded && `+${e.columnsAdded} column${e.columnsAdded > 1 ? "s" : ""}`,
    e.columnsRemoved && `−${e.columnsRemoved} column${e.columnsRemoved > 1 ? "s" : ""}`,
    e.columnsChanged && `${e.columnsChanged} changed`,
  ].filter(Boolean);
  return p.join(" · ") || "no column changes";
}

/** What changed in the schema, when — and the SQL between any two points. */
export function SchemaTimeline({
  conn,
  onOpenSql,
  onToast,
}: {
  conn: ConnectionConfig;
  onOpenSql: (sql: string) => void;
  onToast: (m: string, k?: "ok" | "err") => void;
}) {
  const [list, setList] = useState<TimelineEntry[] | null>(null);
  const [sel, setSel] = useState<number | null>(null);
  /** compare the selected point with: the one before it ("prev"), or a chosen one */
  const [base, setBase] = useState<number | "prev">("prev");
  const [diff, setDiff] = useState<TimelineDiff | null>(null);
  const [busy, setBusy] = useState(false);

  const load = useCallback(() => api.timeline.list(conn.id).then((l) => (setList(l), setSel((s) => s ?? l[0]?.at ?? null))), [conn.id]);
  useEffect(() => {
    load().catch((e) => onToast(String(e)));
  }, [load, onToast]);

  useEffect(() => {
    if (!list || sel == null) return setDiff(null);
    const i = list.findIndex((e) => e.at === sel);
    const from = base === "prev" ? list[i + 1]?.at : base;
    if (from == null) return setDiff(null);
    api.timeline
      .diff(conn.id, from, sel)
      .then(setDiff)
      .catch((e) => onToast(String(e)));
  }, [list, sel, base, conn.id, onToast]);

  const snapshot = async () => {
    setBusy(true);
    try {
      const e = await api.timeline.record(conn.id, true);
      onToast(e ? "Schema change recorded" : "No changes since the last snapshot", "ok");
      await load();
      if (e) setSel(e.at);
    } catch (err) {
      onToast(String(err));
    } finally {
      setBusy(false);
    }
  };

  const selected = list?.find((e) => e.at === sel);
  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex h-10 shrink-0 items-center gap-2 border-b border-line px-3">
        <History size={14} className="text-accent" />
        <span className="text-xs font-semibold">Schema timeline</span>
        <span className="text-2xs text-subtle">{conn.name} · recorded when the schema changes (on connect and after DDL)</span>
        <span className="flex-1" />
        <Button size="xs" onClick={snapshot} disabled={busy}>
          {busy ? <Loader2 size={12} className="animate-spin" /> : <Camera size={12} />} Snapshot now
        </Button>
      </div>
      {list && list.length === 0 ? (
        <div className="flex flex-1 flex-col items-center justify-center gap-3 text-xs text-muted">
          No snapshots yet.
          <Button variant="primary" onClick={snapshot} disabled={busy}>
            <Camera size={12} /> Take the first one
          </Button>
        </div>
      ) : (
        <div className="flex min-h-0 flex-1">
          <div className="w-72 shrink-0 overflow-y-auto border-r border-line py-2">
            {list?.map((e, i) => (
              <button
                key={e.at}
                type="button"
                onClick={() => (setSel(e.at), setBase("prev"))}
                className={cx("relative flex w-full gap-3 py-2 pr-3 pl-4 text-left hover:bg-hover", sel === e.at && "bg-selected")}
              >
                <span className="relative flex w-3 shrink-0 justify-center">
                  {i < list.length - 1 && <span className="absolute top-3 -bottom-5 w-px bg-line" />}
                  <span className={cx("relative mt-1 size-2.5 rounded-full border-2", e.initial ? "border-subtle bg-bg" : e.tablesRemoved || e.columnsRemoved ? "border-danger bg-danger/30" : "border-accent bg-accent/30")} />
                </span>
                <span className="min-w-0">
                  <span className="block text-xs font-medium">{when(e.at)}</span>
                  <span className="block truncate text-2xs text-muted">{summary(e)}</span>
                  <span className="block text-2xs text-subtle">
                    {e.tables} tables · {e.columns} columns
                  </span>
                </span>
              </button>
            ))}
          </div>
          <div className="flex min-w-0 flex-1 flex-col">
            {selected && (
              <div className="flex shrink-0 flex-wrap items-center gap-2 border-b border-line px-3 py-1.5 text-2xs text-muted">
                Changes from
                <select value={String(base)} onChange={(e) => setBase(e.target.value === "prev" ? "prev" : Number(e.target.value))} className="h-6 rounded border border-line bg-elevated px-1 text-2xs text-fg outline-none">
                  <option value="prev">the snapshot before</option>
                  {list!
                    .filter((e) => e.at < selected.at)
                    .map((e) => (
                      <option key={e.at} value={e.at}>
                        {when(e.at)}
                      </option>
                    ))}
                </select>
                to <b className="text-fg">{when(selected.at)}</b>
              </div>
            )}
            {!diff ? (
              <div className="flex flex-1 items-center justify-center text-xs text-subtle">{selected?.initial ? "This is the first snapshot — later changes show up here." : "Pick a point on the timeline"}</div>
            ) : (
              <div className="flex min-h-0 flex-1 flex-col">
                <div className="min-h-0 flex-1 overflow-y-auto p-3">
                  {diff.schemas.length === 0 && <p className="text-xs text-subtle">No schema changes between these points.</p>}
                  {diff.schemas.map((s) => (
                    <div key={s.schema} className="mb-3">
                      <div className="mb-1 text-2xs font-semibold tracking-wide text-subtle uppercase">{s.schema}</div>
                      <div className="overflow-hidden rounded-lg border border-line">
                        {s.changes.map((c, i) => {
                          const m = META[c.kind];
                          const Icon = m.icon;
                          return (
                            <div key={i} className="flex items-baseline gap-2 border-b border-line px-3 py-1.5 text-xs last:border-b-0">
                              <Icon size={11} className={cx("shrink-0 self-center", m.cls)} />
                              <span className="w-28 shrink-0 text-2xs text-muted">{m.text}</span>
                              <span className="w-40 shrink-0 truncate font-mono text-[11.5px]">{c.table}</span>
                              <span className="min-w-0 font-mono text-[11px] break-all text-muted">{describeChange(c)}</span>
                            </div>
                          );
                        })}
                      </div>
                    </div>
                  ))}
                </div>
                <div className="flex h-56 shrink-0 flex-col border-t border-line">
                  <div className="flex items-center gap-2 px-3 py-1 text-2xs text-muted">
                    Migration SQL <span className="text-subtle">(drops stay commented out)</span>
                    <span className="flex-1" />
                    <Button size="xs" variant="ghost" onClick={() => (navigator.clipboard.writeText(diff.sql), onToast("Copied", "ok"))}>
                      <Copy size={11} /> Copy
                    </Button>
                    <Button size="xs" variant="ghost" onClick={() => onOpenSql(diff.sql)}>
                      <SquareTerminal size={11} /> Open in a query tab
                    </Button>
                  </div>
                  <pre className="selectable min-h-0 flex-1 overflow-auto bg-panel/50 px-3 py-2 font-mono text-[11.5px] leading-relaxed">{diff.sql}</pre>
                </div>
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
