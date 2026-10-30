import type { Tab } from "../lib/types";
import { cx } from "./ui";

export function StatusBar({ tab, connected }: { tab: Tab | null; connected: boolean }) {
  const r = tab?.output?.results[tab.resultIndex ?? 0];
  return (
    <footer className="flex h-6 shrink-0 items-center gap-3 border-t border-line bg-panel px-3 text-2xs text-muted">
      <span className="flex items-center gap-1.5">
        <span className={cx("size-1.5 rounded-full", connected ? "bg-success" : "bg-subtle")} />
        {connected ? "Connected" : "Not connected"}
      </span>
      <span className="flex-1" />
      {tab?.error && (/cancelled$/i.test(tab.error.trim()) ? <span>Cancelled</span> : <span className="text-danger">Error</span>)}
      {r && (
        <span className="tabular-nums">
          {r.columns.length > 0
            ? `${r.rows.length.toLocaleString()} row${r.rows.length === 1 ? "" : "s"}${r.truncated ? " (limited)" : ""}`
            : `${r.rows_affected} affected`}
        </span>
      )}
      {tab?.output && <span className="tabular-nums">{tab.output.elapsed_ms} ms</span>}
      <span>UTF-8</span>
    </footer>
  );
}
