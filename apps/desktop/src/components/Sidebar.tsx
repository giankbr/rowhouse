import { ChevronRight, Database, Eye, Lock, Plus, RefreshCw, Search, Settings2, Table2 } from "lucide-react";
import { useMemo, useState } from "react";
import type { ConnectionConfig, TableInfo } from "../lib/api";
import { KindBadge } from "./KindBadge";
import { Button, Dot, cx } from "./ui";


export function Sidebar({
  connections,
  activeId,
  tables,
  loadingTables,
  onSelect,
  onEdit,
  onNew,
  onOpenTable,
  onRefresh,
}: {
  connections: ConnectionConfig[];
  activeId: string | null;
  tables: TableInfo[];
  loadingTables: boolean;
  onSelect: (id: string) => void;
  onEdit: (c: ConnectionConfig) => void;
  onNew: () => void;
  onOpenTable: (t: TableInfo) => void;
  onRefresh: () => void;
}) {
  const [filter, setFilter] = useState("");
  const [collapsed, setCollapsed] = useState<Record<string, boolean>>({});

  const grouped = useMemo(() => {
    const f = filter.toLowerCase();
    const g = new Map<string, TableInfo[]>();
    for (const t of tables) {
      if (f && !t.name.toLowerCase().includes(f)) continue;
      g.set(t.schema, [...(g.get(t.schema) ?? []), t]);
    }
    return [...g.entries()];
  }, [tables, filter]);

  return (
    <aside className="flex h-full w-full flex-col bg-panel">
      <div className="flex h-8 items-center justify-between pr-1.5 pl-3">
        <span className="text-2xs font-semibold tracking-wide text-subtle uppercase">Connections</span>
        <Button variant="ghost" size="icon" onClick={onNew} title="New connection">
          <Plus size={14} />
        </Button>
      </div>

      <div className="flex flex-col gap-px px-1.5">
        {connections.length === 0 && (
          <button
            onClick={onNew}
            className="m-1 rounded-lg border border-dashed border-line-strong px-3 py-4 text-center text-2xs text-muted hover:bg-hover"
          >
            Add your first connection
          </button>
        )}
        {connections.map((c) => (
          <div
            key={c.id}
            onClick={() => onSelect(c.id)}
            onDoubleClick={() => onEdit(c)}
            className={cx(
              "group flex h-7 cursor-default items-center gap-2 rounded-md px-2 text-xs",
              c.id === activeId ? "bg-selected text-fg" : "text-muted hover:bg-hover hover:text-fg",
            )}
          >
            <Dot color={c.color} />
            <span className="flex-1 truncate font-medium">{c.name || "Untitled"}</span>
            {c.readOnly && <Lock size={11} className="text-subtle" />}
            <KindBadge kind={c.kind} size={15} className="opacity-80" />
            <button
              onClick={(e) => {
                e.stopPropagation();
                onEdit(c);
              }}
              className="hidden text-subtle group-hover:block hover:text-fg"
              title="Edit connection"
            >
              <Settings2 size={12} />
            </button>
          </div>
        ))}
      </div>

      {activeId && (
        <>
          <div className="mx-3 mt-3 mb-1.5 h-px bg-line" />
          <div className="flex items-center gap-1 px-1.5">
            <div className="relative flex-1">
              <Search size={12} className="pointer-events-none absolute top-1/2 left-2 -translate-y-1/2 text-subtle" />
              <input
                value={filter}
                onChange={(e) => setFilter(e.target.value)}
                placeholder="Filter tables"
                spellCheck={false}
                className="h-6 w-full rounded-md bg-transparent pr-2 pl-6.5 text-xs outline-none placeholder:text-subtle hover:bg-hover focus:bg-elevated focus:ring-1 focus:ring-line-strong"
              />
            </div>
            <Button variant="ghost" size="icon" onClick={onRefresh} title="Refresh">
              <RefreshCw size={12} className={cx(loadingTables && "animate-spin")} />
            </Button>
          </div>

          <div className="mt-1 flex-1 overflow-y-auto px-1.5 pb-2">
            {loadingTables && tables.length === 0 && (
              <div className="space-y-1.5 px-2 pt-2">
                {Array.from({ length: 7 }).map((_, i) => (
                  <div key={i} className="h-3 animate-pulse rounded bg-hover" style={{ width: `${50 + ((i * 37) % 45)}%` }} />
                ))}
              </div>
            )}
            {grouped.map(([schema, list]) => (
              <div key={schema}>
                {grouped.length > 1 || schema !== "main" ? (
                  <button
                    onClick={() => setCollapsed((s) => ({ ...s, [schema]: !s[schema] }))}
                    className="flex h-6 w-full items-center gap-1 rounded px-1 text-2xs font-medium text-muted hover:text-fg"
                  >
                    <ChevronRight size={12} className={cx("transition-transform", !collapsed[schema] && "rotate-90")} />
                    <Database size={11} />
                    {schema}
                    <span className="ml-auto pr-1 text-subtle">{list.length}</span>
                  </button>
                ) : null}
                {!collapsed[schema] &&
                  list.map((t) => (
                    <button
                      key={t.schema + "." + t.name}
                      onClick={() => onOpenTable(t)}
                      className="flex h-6 w-full items-center gap-2 rounded-md pr-2 pl-5 text-xs text-fg/85 hover:bg-hover"
                    >
                      {t.kind === "view" ? (
                        <Eye size={12} className="shrink-0 text-subtle" />
                      ) : (
                        <Table2 size={12} className="shrink-0 text-subtle" />
                      )}
                      <span className="truncate">{t.name}</span>
                    </button>
                  ))}
              </div>
            ))}
          </div>
        </>
      )}
    </aside>
  );
}
