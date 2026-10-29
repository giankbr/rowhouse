import { Plus, SquareTerminal, Table2, X } from "lucide-react";
import { countPending } from "../lib/pending";
import type { Tab } from "../lib/types";
import { Button, cx } from "./ui";

export function TabBar({
  tabs,
  activeId,
  onSelect,
  onClose,
  onNew,
}: {
  tabs: Tab[];
  activeId: string | null;
  onSelect: (id: string) => void;
  onClose: (id: string) => void;
  onNew: () => void;
}) {
  return (
    <div className="flex h-8 shrink-0 items-end gap-px overflow-x-auto border-b border-line bg-panel px-1.5">
      {tabs.map((t) => {
        const active = t.id === activeId;
        return (
          <div
            key={t.id}
            onMouseDown={(e) => (e.button === 1 ? onClose(t.id) : onSelect(t.id))}
            className={cx(
              "group relative flex h-7 max-w-48 min-w-28 cursor-default items-center gap-1.5 rounded-t-md pr-1 pl-2.5 text-xs",
              active ? "bg-bg text-fg shadow-[0_0_0_1px_var(--border)] [clip-path:inset(-1px_-1px_0_-1px)]" : "text-muted hover:bg-hover hover:text-fg",
            )}
          >
            {t.kind === "table" ? <Table2 size={12} className="shrink-0 text-subtle" /> : <SquareTerminal size={12} className="shrink-0 text-subtle" />}
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
      <Button variant="ghost" size="icon" className="mb-0.5" onClick={onNew} title="New query">
        <Plus size={13} />
      </Button>
    </div>
  );
}
