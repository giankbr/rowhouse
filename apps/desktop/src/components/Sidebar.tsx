import { Braces, ChevronRight, Database, Eye, FileInput, Hash, Layers, ListOrdered, SquareFunction, Zap, GitCompareArrows, Lock, Network, Plus, RefreshCw, Search, Settings2, Table2 } from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import type { ConnectionConfig, DbObject, TableInfo } from "../lib/api";
import { KindBadge } from "./KindBadge";
import { ContextMenu, type MenuItem } from "./TableActions";
import { Button, Dot, cx } from "./ui";


const OBJECT_GROUP: Record<string, { label: string; icon: typeof Braces }> = {
  matview: { label: "Materialized views", icon: Layers },
  function: { label: "Functions", icon: SquareFunction },
  procedure: { label: "Functions", icon: SquareFunction },
  macro: { label: "Macros", icon: SquareFunction },
  trigger: { label: "Triggers", icon: Zap },
  sequence: { label: "Sequences", icon: ListOrdered },
  type: { label: "Types", icon: Hash },
};
const GROUP_ORDER = ["Materialized views", "Functions", "Macros", "Triggers", "Sequences", "Types"];

export type TableMenuAction =
  | "open"
  | "structure"
  | "ddl"
  | "query"
  | "copy-name"
  | "copy-select"
  | "export"
  | "generate"
  | "rename"
  | "duplicate"
  | "truncate"
  | "drop";

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
  onImport,
  onErd,
  onCompare,
  onTableMenu,
  onNewTable,
  canEdit = true,
  objects = [],
  onOpenObject,
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
  onImport: () => void;
  onErd: () => void;
  onCompare: () => void;
  onTableMenu?: (t: TableInfo, action: TableMenuAction) => void;
  onNewTable?: () => void;
  /** SQL database with DDL (not Redis) */
  canEdit?: boolean;
  objects?: DbObject[];
  onOpenObject?: (o: DbObject) => void;
}) {
  const [filter, setFilter] = useState("");
  const [openGroups, setOpenGroups] = useState<Record<string, boolean>>({});
  const [menu, setMenu] = useState<{ x: number; y: number; t: TableInfo } | null>(null);
  const closeMenu = useCallback(() => setMenu(null), []);

  const menuItems = (t: TableInfo): MenuItem[] => {
    const go = (a: TableMenuAction) => () => onTableMenu?.(t, a);
    const view = t.kind === "view";
    return [
      { label: "Open", hint: "↵", onSelect: go("open") },
      { label: "Structure", onSelect: go("structure") },
      { label: "Show DDL", onSelect: go("ddl") },
      { label: "New query", onSelect: go("query") },
      "separator",
      { label: "Copy name", onSelect: go("copy-name") },
      { label: "Copy SELECT", onSelect: go("copy-select") },
      { label: "Export…", onSelect: go("export") },
      { label: "Generate data…", disabled: !canEdit || view, onSelect: go("generate") },
      "separator",
      { label: "Rename…", disabled: !canEdit, onSelect: go("rename") },
      { label: "Duplicate…", disabled: !canEdit || view, onSelect: go("duplicate") },
      { label: "Truncate…", danger: true, disabled: !canEdit || view, onSelect: go("truncate") },
      { label: view ? "Drop view…" : "Drop table…", danger: true, disabled: !canEdit, onSelect: go("drop") },
    ];
  };
  const [collapsed, setCollapsed] = useState<Record<string, boolean>>({});

  const grouped = useMemo(() => {
    const f = filter.toLowerCase();
    const g = new Map<string, TableInfo[]>();
    for (const t of tables) {
      if (f && !t.name.toLowerCase().includes(f)) continue;
      g.set(t.schema, [...(g.get(t.schema) ?? []), t]);
    }
    for (const o of objects) if (!g.has(o.schema) && (!f || o.name.toLowerCase().includes(f))) g.set(o.schema, []);
    return [...g.entries()];
  }, [tables, objects, filter]);
  const objectsIn = (schema: string) => {
    const f = filter.toLowerCase();
    const m = new Map<string, DbObject[]>();
    for (const o of objects) {
      if (o.schema !== schema || (f && !o.name.toLowerCase().includes(f))) continue;
      const k = OBJECT_GROUP[o.kind]?.label ?? o.kind;
      m.set(k, [...(m.get(k) ?? []), o]);
    }
    return [...m.entries()].sort((a, b) => GROUP_ORDER.indexOf(a[0]) - GROUP_ORDER.indexOf(b[0]));
  };

  return (
    <aside className="flex h-full w-full flex-col bg-panel">
      <div className="flex h-8 items-center justify-between pr-1.5 pl-3">
        <span className="text-2xs font-semibold tracking-wide text-subtle uppercase">Connections</span>
        <span className="flex">
          <Button variant="ghost" size="icon" onClick={onImport} title="Import from DBeaver / TablePlus / Navicat">
            <FileInput size={13} />
          </Button>
          <Button variant="ghost" size="icon" onClick={onNew} title="New connection">
            <Plus size={14} />
          </Button>
        </span>
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
            {onNewTable && canEdit && (
              <Button variant="ghost" size="icon" onClick={onNewTable} title="New table">
                <Plus size={13} />
              </Button>
            )}
            <Button variant="ghost" size="icon" onClick={onCompare} title="Compare schemas">
              <GitCompareArrows size={12} />
            </Button>
            <Button variant="ghost" size="icon" onClick={onErd} title="Diagram (ERD)">
              <Network size={12} />
            </Button>
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
                      onContextMenu={(e) => {
                        if (!onTableMenu) return;
                        e.preventDefault();
                        setMenu({ x: e.clientX, y: e.clientY, t });
                      }}
                      className={cx(
                        "flex h-6 w-full items-center gap-2 rounded-md pr-2 pl-5 text-xs text-fg/85 hover:bg-hover",
                        menu?.t === t && "bg-hover",
                      )}
                    >
                      {t.kind === "view" ? (
                        <Eye size={12} className="shrink-0 text-subtle" />
                      ) : (
                        <Table2 size={12} className="shrink-0 text-subtle" />
                      )}
                      <span className="truncate">{t.name}</span>
                    </button>
                  ))}
                {!collapsed[schema] &&
                  objectsIn(schema).map(([group, list]) => {
                    const key = `${schema}\u0000${group}`;
                    const Icon = Object.values(OBJECT_GROUP).find((g) => g.label === group)?.icon ?? Braces;
                    const open = !!openGroups[key] || !!filter;
                    return (
                      <div key={key}>
                        <button
                          onClick={() => setOpenGroups((s) => ({ ...s, [key]: !s[key] }))}
                          className="flex h-6 w-full items-center gap-1.5 rounded-md pr-2 pl-3 text-2xs text-muted hover:bg-hover hover:text-fg"
                        >
                          <ChevronRight size={11} className={cx("transition-transform", open && "rotate-90")} />
                          <Icon size={11} className="text-subtle" />
                          {group}
                          <span className="ml-auto pr-1 text-subtle">{list.length}</span>
                        </button>
                        {open &&
                          list.map((o) => (
                            <button
                              key={o.kind + o.name + (o.detail ?? "")}
                              onClick={() => onOpenObject?.(o)}
                              title={o.kind === "trigger" ? `${o.name} on ${o.detail ?? ""}` : o.detail ? `${o.name}(${o.detail})` : o.name}
                              className="flex h-6 w-full items-center gap-2 rounded-md pr-2 pl-8 text-xs text-fg/85 hover:bg-hover"
                            >
                              <span className="truncate">{o.name}</span>
                              {o.kind === "trigger" && o.detail && <span className="ml-auto truncate text-2xs text-subtle">{o.detail}</span>}
                              {o.kind === "procedure" && <span className="ml-auto text-2xs text-subtle">proc</span>}
                            </button>
                          ))}
                      </div>
                    );
                  })}
              </div>
            ))}
          </div>
        </>
      )}
      {menu && <ContextMenu x={menu.x} y={menu.y} items={menuItems(menu.t)} onClose={closeMenu} />}
    </aside>
  );
}
