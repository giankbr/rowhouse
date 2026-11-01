import { AlertTriangle, Eye, Loader2 } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { api, type Impact } from "../lib/api";
import { diffResults, type RowDiff } from "../lib/rowdiff";
import { DiffView } from "./DiffView";
import { Button, Modal } from "./ui";

/** Statements the preview understands (the backend parses them properly). */
export const previewable = (sql: string) => {
  const s = sql.trim().replace(/;\s*$/, "");
  return /^(update|delete)\b/i.test(s) && !/;\s*\S/.test(s);
};

/**
 * "Preview changes": the statement ran inside a transaction that was rolled
 * back; here are the rows it touched. Commit re-runs it and rolls back unless
 * the same number of rows changes.
 */
export function ImpactDialog({
  connId,
  sql,
  onClose,
  onCommitted,
  onToast,
}: {
  connId: string;
  sql: string;
  onClose: () => void;
  onCommitted: (rows: number) => void;
  onToast: (m: string, k?: "ok" | "err") => void;
}) {
  const [impact, setImpact] = useState<Impact | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    api.previewDml(connId, sql).then(setImpact, (e) => setError(String(e)));
  }, [connId, sql]);

  const diff: RowDiff | null = useMemo(() => {
    if (!impact) return null;
    if (impact.kind === "delete" || !impact.after) {
      // deleted rows, or an update on a table without a primary key: show the matching rows
      const none = { ...impact.before, rows: [] };
      const d = diffResults(impact.before, none, impact.key);
      return impact.kind === "delete" ? d : { ...d, removed: [], added: impact.before.rows, unchanged: 0 };
    }
    return diffResults(impact.before, impact.after, impact.key);
  }, [impact]);

  const commit = async () => {
    if (!impact) return;
    setBusy(true);
    try {
      const n = await api.commitDml(connId, sql, impact.affected);
      onToast(`${n.toLocaleString()} row${n === 1 ? "" : "s"} ${impact.kind === "delete" ? "deleted" : "updated"}`, "ok");
      onCommitted(n);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const noun = impact ? `${impact.affected.toLocaleString()} row${impact.affected === 1 ? "" : "s"}` : "";
  return (
    <Modal
      open
      onClose={onClose}
      width={920}
      title={
        <span className="flex items-center gap-2">
          <Eye size={14} className="text-accent" /> Preview changes
        </span>
      }
      footer={
        <>
          <span className="flex-1 text-2xs text-subtle">Nothing has changed yet — the preview ran in a transaction that was rolled back.</span>
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button variant="danger" disabled={!impact || busy || impact.affected === 0} onClick={commit}>
            {busy && <Loader2 size={12} className="animate-spin" />}
            {impact ? `${impact.kind === "delete" ? "Delete" : "Update"} ${noun}` : "Commit"}
          </Button>
        </>
      }
    >
      <div className="-mx-4 -my-4 flex h-[460px] flex-col">
        {error ? (
          <div className="m-4 flex gap-2 rounded-lg border border-danger/30 bg-danger/5 p-3 text-xs text-danger">
            <AlertTriangle size={14} className="mt-px shrink-0" />
            <span className="whitespace-pre-wrap">{error}</span>
          </div>
        ) : !impact || !diff ? (
          <div className="flex flex-1 items-center justify-center gap-2 text-xs text-subtle">
            <Loader2 size={14} className="animate-spin" /> Running in a rolled-back transaction…
          </div>
        ) : (
          <>
            <div className="border-b border-line px-4 py-2 text-xs">
              This {impact.kind.toUpperCase()} changes <b>{noun}</b> in <span className="font-mono">{impact.schema}.{impact.table}</span>.
              {impact.before.truncated && <span className="text-subtle"> Showing the first {impact.before.rows.length.toLocaleString()}.</span>}
              {impact.kind === "update" && !impact.key.length && <span className="text-warning"> The table has no primary key, so only the matching rows are shown.</span>}
            </div>
            <div className="min-h-0 flex-1">
              <DiffView diff={diff} empty="No rows match" />
            </div>
          </>
        )}
      </div>
    </Modal>
  );
}
