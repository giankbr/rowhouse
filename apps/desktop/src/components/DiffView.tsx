import { KeyRound } from "lucide-react";
import { useMemo, useState } from "react";
import type { Cell } from "../lib/api";
import type { RowDiff } from "../lib/rowdiff";
import { display } from "./ResultsGrid";
import { cx } from "./ui";

const MAX_ROWS = 2000;

type Line = { kind: "add" | "del" | "chg"; row: Cell[]; before?: Cell[]; cells?: Set<number> };

/** Rows added / removed / changed, with changed cells as `old → new`. */
export function DiffView({
  diff,
  keyOptions,
  onKey,
  title,
  empty = "No row changes",
}: {
  diff: RowDiff;
  /** columns the user can pick as the match key (none = whole rows) */
  keyOptions?: string[];
  onKey?: (key: string[]) => void;
  title?: React.ReactNode;
  empty?: string;
}) {
  const [show, setShow] = useState<{ add: boolean; del: boolean; chg: boolean }>({ add: true, del: true, chg: true });
  const lines = useMemo(() => {
    const out: Line[] = [];
    if (show.chg) for (const c of diff.changed) out.push({ kind: "chg", row: c.after, before: c.before, cells: new Set(c.cells) });
    if (show.add) for (const r of diff.added) out.push({ kind: "add", row: r });
    if (show.del) for (const r of diff.removed) out.push({ kind: "del", row: r });
    return out;
  }, [diff, show]);
  const chip = (k: "add" | "del" | "chg", n: number, label: string, tone: string) => (
    <button
      type="button"
      onClick={() => setShow((s) => ({ ...s, [k]: !s[k] }))}
      className={cx("rounded-full px-2 py-0.5 text-2xs font-medium tabular-nums transition-opacity", tone, !show[k] && "opacity-40")}
    >
      {n.toLocaleString()} {label}
    </button>
  );
  const nothing = !diff.added.length && !diff.removed.length && !diff.changed.length;
  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex shrink-0 flex-wrap items-center gap-2 border-b border-line px-3 py-1.5 text-xs">
        {title}
        {chip("add", diff.added.length, "added", "bg-success/12 text-success")}
        {chip("del", diff.removed.length, "removed", "bg-danger/12 text-danger")}
        {chip("chg", diff.changed.length, "changed", "bg-warning/15 text-warning")}
        <span className="text-2xs text-subtle">{diff.unchanged.toLocaleString()} unchanged</span>
        {(diff.columnsAdded.length > 0 || diff.columnsRemoved.length > 0) && (
          <span className="text-2xs text-muted">
            {diff.columnsAdded.length > 0 && `+ columns ${diff.columnsAdded.join(", ")} `}
            {diff.columnsRemoved.length > 0 && `− columns ${diff.columnsRemoved.join(", ")}`}
          </span>
        )}
        <span className="flex-1" />
        {onKey && keyOptions && (
          <label className="flex items-center gap-1 text-2xs text-muted" title="Rows with the same key are compared cell by cell">
            <KeyRound size={11} /> match by
            <select
              value={diff.key[0] ?? ""}
              onChange={(e) => onKey(e.target.value ? [e.target.value] : [])}
              className="h-6 rounded border border-line bg-elevated px-1 text-2xs text-fg outline-none"
            >
              <option value="">whole row</option>
              {keyOptions.map((k) => (
                <option key={k} value={k}>
                  {k}
                </option>
              ))}
            </select>
          </label>
        )}
      </div>
      {nothing ? (
        <div className="flex flex-1 items-center justify-center text-xs text-subtle">{empty}</div>
      ) : (
        <div className="min-h-0 flex-1 overflow-auto font-mono text-xs">
          <table className="border-separate border-spacing-0">
            <thead className="sticky top-0 z-[1] bg-panel">
              <tr>
                <th className="w-6 border-r border-b border-line" />
                {diff.columns.map((c) => (
                  <th key={c} className="border-r border-b border-line px-2 py-1 text-left font-sans font-medium whitespace-nowrap">
                    {c}
                    {diff.key.includes(c) && <KeyRound size={10} className="ml-1 inline text-accent" />}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {lines.slice(0, MAX_ROWS).map((l, i) => (
                <tr key={i} className={cx(l.kind === "add" && "bg-success/8", l.kind === "del" && "bg-danger/8 text-danger/90")}>
                  <td className={cx("border-r border-b border-line/60 text-center font-sans font-semibold", l.kind === "add" ? "text-success" : l.kind === "del" ? "text-danger" : "text-warning")}>
                    {l.kind === "add" ? "+" : l.kind === "del" ? "−" : "~"}
                  </td>
                  {l.row.map((v, ci) => {
                    const changed = l.cells?.has(ci);
                    return (
                      <td key={ci} className={cx("max-w-[320px] truncate border-r border-b border-line/60 px-2 py-0.5 whitespace-nowrap", changed && "bg-warning/15")} title={changed ? `${display(l.before![ci])} → ${display(v)}` : display(v)}>
                        {changed ? (
                          <>
                            <span className="text-danger line-through decoration-danger/50">{display(l.before![ci])}</span>
                            <span className="text-subtle"> → </span>
                            <span className="text-fg">{display(v)}</span>
                          </>
                        ) : (
                          display(v)
                        )}
                      </td>
                    );
                  })}
                </tr>
              ))}
            </tbody>
          </table>
          {lines.length > MAX_ROWS && <div className="p-2 text-2xs text-subtle">Showing the first {MAX_ROWS.toLocaleString()} of {lines.length.toLocaleString()} differences.</div>}
        </div>
      )}
    </div>
  );
}
