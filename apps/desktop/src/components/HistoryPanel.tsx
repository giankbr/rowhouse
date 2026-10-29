import { Bot, Search, Trash2, X } from "lucide-react";
import { useEffect, useState } from "react";
import { api, type HistoryEntry } from "../lib/api";
import { Button, cx } from "./ui";

function ago(at: number) {
  const s = Math.max(0, Math.floor(Date.now() / 1000) - at);
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  if (s < 86400) return `${Math.floor(s / 3600)}h`;
  return `${Math.floor(s / 86400)}d`;
}

/** Right-hand drawer with this connection's query history (app + MCP). */
export function HistoryPanel({
  connId,
  open,
  onClose,
  onPick,
}: {
  connId: string | null;
  open: boolean;
  onClose: () => void;
  onPick: (sql: string) => void;
}) {
  const [items, setItems] = useState<HistoryEntry[]>([]);
  const [q, setQ] = useState("");
  const [source, setSource] = useState<"all" | "app" | "mcp">("all");

  useEffect(() => {
    if (!open || !connId) return;
    const t = setTimeout(() => api.listHistory(connId, q).then(setItems).catch(() => setItems([])), 120);
    return () => clearTimeout(t);
  }, [open, connId, q]);

  if (!open) return null;
  const shown = items.filter((i) => source === "all" || i.source === source);

  return (
    <aside className="animate-fade flex w-80 shrink-0 flex-col border-l border-line bg-panel">
      <div className="flex h-8 items-center gap-2 border-b border-line pr-1.5 pl-3">
        <span className="text-2xs font-semibold tracking-wide text-subtle uppercase">History</span>
        <span className="flex-1" />
        {(["all", "app", "mcp"] as const).map((s) => (
          <button
            key={s}
            onClick={() => setSource(s)}
            className={cx("h-5 rounded px-1.5 text-2xs", source === s ? "bg-selected text-fg" : "text-muted hover:text-fg")}
          >
            {s === "all" ? "All" : s === "app" ? "App" : "AI"}
          </button>
        ))}
        <Button variant="ghost" size="icon" onClick={onClose} title="Close">
          <X size={13} />
        </Button>
      </div>
      <div className="relative border-b border-line p-1.5">
        <Search size={12} className="pointer-events-none absolute top-1/2 left-3.5 -translate-y-1/2 text-subtle" />
        <input
          autoFocus
          value={q}
          onChange={(e) => setQ(e.target.value)}
          placeholder="Search queries"
          spellCheck={false}
          className="h-6 w-full rounded-md bg-elevated pr-2 pl-7 text-xs outline-none ring-1 ring-line focus:ring-accent"
        />
      </div>
      <div className="flex-1 overflow-y-auto">
        {shown.length === 0 && <div className="p-6 text-center text-2xs text-subtle">No queries yet</div>}
        {shown.map((h) => (
          <button
            key={h.id}
            onClick={() => onPick(h.sql)}
            title={h.error ?? "Open in editor"}
            className="group block w-full border-b border-line/60 px-3 py-2 text-left hover:bg-hover"
          >
            <pre className="line-clamp-3 font-mono text-[11.5px] leading-snug whitespace-pre-wrap text-fg/90">{h.sql}</pre>
            <div className="mt-1 flex items-center gap-2 text-[10.5px] text-subtle">
              <span className={cx("size-1.5 rounded-full", h.ok ? "bg-success" : "bg-danger")} />
              {h.source === "mcp" && (
                <span className="flex items-center gap-0.5 text-accent">
                  <Bot size={10} /> AI
                </span>
              )}
              <span>{h.ok ? `${h.rows.toLocaleString()} row${h.rows === 1 ? "" : "s"}` : "error"}</span>
              <span>{h.elapsedMs} ms</span>
              <span className="ml-auto">{ago(h.at)} ago</span>
            </div>
          </button>
        ))}
      </div>
      {items.length > 0 && (
        <div className="border-t border-line p-1.5">
          <Button
            variant="ghost"
            size="xs"
            className="w-full text-muted"
            onClick={async () => connId && (await api.clearHistory(connId), setItems([]))}
          >
            <Trash2 size={11} /> Clear history for this connection
          </Button>
        </div>
      )}
    </aside>
  );
}
