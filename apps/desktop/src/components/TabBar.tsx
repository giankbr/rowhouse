import { Columns2, Network, Plus, SquareTerminal, Table2, X } from "lucide-react";
import { useState } from "react";
import { countPending } from "../lib/pending";
import type { Tab } from "../lib/types";
import { Button, cx } from "./ui";

export const TAB_MIME = "application/x-rowhouse-tab";

const ICON = { table: Table2, query: SquareTerminal, erd: Network } as const;

/**
 * Tabs for one pane. Tabs can be dragged to reorder, or dropped on the other
 * pane's tab bar to move them across.
 */
export function TabBar({
  tabs,
  activeId,
  focused,
  onSelect,
  onClose,
  onNew,
  onDropTab,
  onSplit,
  onDragState,
}: {
  tabs: Tab[];
  activeId: string | null;
  focused: boolean;
  onSelect: (id: string) => void;
  onClose: (id: string) => void;
  onNew: () => void;
  /** move `id` into this pane, before `beforeId` (or at the end) */
  onDropTab: (id: string, beforeId: string | null) => void;
  onSplit?: () => void;
  onDragState: (dragging: string | null) => void;
}) {
  const [over, setOver] = useState<string | "end" | null>(null);

  const drop = (e: React.DragEvent, beforeId: string | null) => {
    const id = e.dataTransfer.getData(TAB_MIME);
    setOver(null);
    onDragState(null);
    if (id && id !== beforeId) onDropTab(id, beforeId);
  };

  return (
    <div
      className={cx("flex h-8 shrink-0 items-end gap-px overflow-x-auto border-b border-line px-1.5", focused ? "bg-panel" : "bg-panel/60")}
      onDragOver={(e) => e.dataTransfer.types.includes(TAB_MIME) && (e.preventDefault(), setOver("end"))}
      onDragLeave={() => setOver(null)}
      onDrop={(e) => drop(e, null)}
    >
      {tabs.map((t) => {
        const active = t.id === activeId;
        const Icon = ICON[t.kind];
        return (
          <div
            key={t.id}
            draggable
            onDragStart={(e) => {
              e.dataTransfer.setData(TAB_MIME, t.id);
              e.dataTransfer.effectAllowed = "move";
              onDragState(t.id);
            }}
            onDragEnd={() => (onDragState(null), setOver(null))}
            onDragOver={(e) => {
              if (!e.dataTransfer.types.includes(TAB_MIME)) return;
              e.preventDefault();
              e.stopPropagation();
              setOver(t.id);
            }}
            onDrop={(e) => (e.stopPropagation(), drop(e, t.id))}
            onMouseDown={(e) => (e.button === 1 ? onClose(t.id) : onSelect(t.id))}
            className={cx(
              "group relative flex h-7 max-w-48 min-w-28 cursor-default items-center gap-1.5 rounded-t-md pr-1 pl-2.5 text-xs",
              active
                ? "bg-bg text-fg shadow-[0_0_0_1px_var(--border)] [clip-path:inset(-1px_-1px_0_-1px)]"
                : "text-muted hover:bg-hover hover:text-fg",
              active && focused && "before:absolute before:inset-x-2 before:top-0 before:h-0.5 before:rounded-full before:bg-accent",
            )}
          >
            {over === t.id && <span className="absolute top-1 bottom-1 -left-px w-0.5 rounded bg-accent" />}
            <Icon size={12} className="shrink-0 text-subtle" />
            <span className="flex-1 truncate">{t.title}</span>
            {t.running && <span className="size-1.5 animate-pulse rounded-full bg-accent" />}
            {countPending(t.pending) > 0 && <span title="Unsaved changes" className="size-1.5 rounded-full bg-warning" />}
            <button
              onMouseDown={(e) => e.stopPropagation()}
              onClick={() => onClose(t.id)}
              className={cx("rounded p-0.5 text-subtle hover:bg-hover hover:text-fg", !active && "opacity-0 group-hover:opacity-100")}
            >
              <X size={11} />
            </button>
          </div>
        );
      })}
      {over === "end" && <span className="mb-1.5 h-5 w-0.5 rounded bg-accent" />}
      <Button variant="ghost" size="icon" className="mb-0.5" onClick={onNew} title="New query">
        <Plus size={13} />
      </Button>
      {onSplit && (
        <Button variant="ghost" size="icon" className="mb-0.5 ml-auto" onClick={onSplit} title="Split right (drag a tab to the right edge works too)">
          <Columns2 size={13} />
        </Button>
      )}
    </div>
  );
}
