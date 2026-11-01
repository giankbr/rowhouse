import { EyeOff } from "lucide-react";
import { saveAppearance, useAppearance } from "../lib/appearance";
import type { Tab } from "../lib/types";
import { cx } from "./ui";

export function StatusBar({ tab, connected }: { tab: Tab | null; connected: boolean }) {
  const r = tab?.output?.results[tab.resultIndex ?? 0];
  const appearance = useAppearance();
  return (
    <footer className="flex h-6 shrink-0 items-center gap-3 border-t border-line bg-panel px-3 text-2xs text-muted">
      <span className="flex items-center gap-1.5">
        <span className={cx("size-1.5 rounded-full", connected ? "bg-success" : "bg-subtle")} />
        {connected ? "Connected" : "Not connected"}
      </span>
      <span className="flex-1" />
      {appearance.maskAll && (
        <button
          type="button"
          onClick={() => saveAppearance({ ...appearance, maskAll: false })}
          className="flex items-center gap-1 rounded bg-accent/12 px-1.5 font-medium text-accent hover:bg-accent/20"
          title="Personal data is masked in every grid. Click (or Ctrl+Shift+M) to turn off."
        >
          <EyeOff size={11} /> Screenshot mode
        </button>
      )}
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
