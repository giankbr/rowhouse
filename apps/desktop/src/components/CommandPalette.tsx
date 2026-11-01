import { Command } from "cmdk";
import { Activity, Blocks, Combine, EyeOff, NotebookPen, History as HistoryIcon, FileCode2, LayoutGrid, Puzzle, UsersRound, Bot, HardDriveDownload, HardDriveUpload, CalendarClock, Columns2, Database, GitCompareArrows, Eye, FileInput, History, Moon, Network, Plus, Settings, SquareTerminal, Table2 } from "lucide-react";
import type { ConnectionConfig, TableInfo } from "../lib/api";
import { Dot } from "./ui";

export function CommandPalette({
  open,
  onClose,
  connections,
  tables,
  actions,
  extensions = [],
  dashboards = [],
  notebooks = [],
}: {
  dashboards?: { label: string; run: () => void }[];
  notebooks?: { label: string; run: () => void }[];
  /** panels of enabled extensions */
  extensions?: { label: string; run: () => void }[];
  open: boolean;
  onClose: () => void;
  connections: ConnectionConfig[];
  tables: TableInfo[];
  actions: {
    newQuery: () => void;
    showHistory: () => void;
    openErd: () => void;
    openCompare: () => void;
    importConnections: () => void;
    splitRight: () => void;
    showSettings: () => void;
    newConnection: () => void;
    openTable: (t: TableInfo) => void;
    selectConnection: (id: string) => void;
    toggleTheme: () => void;
    showMcp: () => void;
    showSchedules: () => void;
    openBuilder: () => void;
    showActivity: () => void;
    showRoles: () => void;
    runFile: () => void;
    newWorkspace: () => void;
    newNotebook: () => void;
    toggleMask: () => void;
    schemaTimeline: () => void;
    showExtensions: () => void;
    showDashboards: () => void;
    backup: () => void;
    restore: () => void;
  };
}) {
  if (!open) return null;
  const run = (fn: () => void) => () => {
    onClose();
    fn();
  };
  const item =
    "flex h-8 cursor-default items-center gap-2.5 rounded-md px-2 text-xs text-fg data-[selected=true]:bg-selected";
  const group = "[&_[cmdk-group-heading]]:px-2 [&_[cmdk-group-heading]]:pt-2 [&_[cmdk-group-heading]]:pb-1 [&_[cmdk-group-heading]]:text-[10.5px] [&_[cmdk-group-heading]]:font-medium [&_[cmdk-group-heading]]:text-subtle";

  return (
    <div
      className="animate-fade fixed inset-0 z-50 flex items-start justify-center bg-black/20 pt-[14vh]"
      onMouseDown={(e) => e.target === e.currentTarget && onClose()}
    >
      <Command
        loop
        className="animate-pop w-[560px] overflow-hidden rounded-xl bg-elevated shadow-pop"
        onKeyDown={(e) => e.key === "Escape" && onClose()}
      >
        <Command.Input
          autoFocus
          placeholder="Type a table, connection or command…"
          className="h-11 w-full border-b border-line bg-transparent px-4 text-sm outline-none placeholder:text-subtle focus-visible:outline-none"
        />
        <Command.List className="max-h-[360px] overflow-y-auto p-1.5">
          <Command.Empty className="py-6 text-center text-xs text-subtle">No results</Command.Empty>
          {tables.length > 0 && (
            <Command.Group heading="Tables" className={group}>
              {tables.slice(0, 200).map((t) => (
                <Command.Item key={t.schema + t.name} value={`table ${t.schema}.${t.name}`} onSelect={run(() => actions.openTable(t))} className={item}>
                  {t.kind === "view" ? <Eye size={13} className="text-subtle" /> : <Table2 size={13} className="text-subtle" />}
                  {t.name}
                  <span className="ml-auto text-2xs text-subtle">{t.schema}</span>
                </Command.Item>
              ))}
            </Command.Group>
          )}
          <Command.Group heading="Connections" className={group}>
            {connections.map((c) => (
              <Command.Item key={c.id} value={`connection ${c.name}`} onSelect={run(() => actions.selectConnection(c.id))} className={item}>
                <Dot color={c.color} />
                {c.name}
                <span className="ml-auto text-2xs text-subtle">{c.kind}</span>
              </Command.Item>
            ))}
          </Command.Group>
          <Command.Group heading="Commands" className={group}>
            <Command.Item onSelect={run(actions.newQuery)} className={item}>
              <SquareTerminal size={13} className="text-subtle" /> New query tab
            </Command.Item>
            <Command.Item onSelect={run(actions.openErd)} className={item}>
              <Network size={13} className="text-subtle" /> Open diagram (ERD)
            </Command.Item>
            <Command.Item onSelect={run(actions.openCompare)} className={item}>
              <GitCompareArrows size={13} className="text-subtle" /> Compare schemas
            </Command.Item>
            <Command.Item onSelect={run(actions.splitRight)} className={item}>
              <Columns2 size={13} className="text-subtle" /> Split right
            </Command.Item>
            <Command.Item onSelect={run(actions.importConnections)} className={item}>
              <FileInput size={13} className="text-subtle" /> Import connections (DBeaver, TablePlus, Navicat)
            </Command.Item>
            <Command.Item onSelect={run(actions.showHistory)} className={item}>
              <History size={13} className="text-subtle" /> Query history
            </Command.Item>
            <Command.Item onSelect={run(actions.openBuilder)} className={item}>
              <Blocks size={13} className="text-subtle" /> Visual query builder
            </Command.Item>
            <Command.Item onSelect={run(actions.showActivity)} className={item}>
              <Activity size={13} className="text-subtle" /> Server activity (processes)
            </Command.Item>
            <Command.Item onSelect={run(actions.showRoles)} className={item}>
              <UsersRound size={13} className="text-subtle" /> Users & roles (privileges)
            </Command.Item>
            <Command.Item onSelect={run(actions.schemaTimeline)} className={item} keywords={["history", "migration", "changes", "ddl"]}>
              <HistoryIcon size={13} className="text-subtle" /> Schema timeline
            </Command.Item>
            <Command.Item onSelect={run(actions.toggleMask)} className={item} keywords={["pii", "privacy", "mask", "demo", "share"]}>
              <EyeOff size={13} className="text-subtle" /> Toggle screenshot mode (mask personal data)
            </Command.Item>
            <Command.Item onSelect={run(actions.newWorkspace)} className={item} keywords={["join", "federated", "duckdb", "csv", "parquet"]}>
              <Combine size={13} className="text-subtle" /> New cross-database workspace…
            </Command.Item>
            <Command.Item onSelect={run(actions.runFile)} className={item}>
              <FileCode2 size={13} className="text-subtle" /> Run SQL file…
            </Command.Item>
            <Command.Item onSelect={run(actions.backup)} className={item}>
              <HardDriveDownload size={13} className="text-subtle" /> Back up database…
            </Command.Item>
            <Command.Item onSelect={run(actions.restore)} className={item}>
              <HardDriveUpload size={13} className="text-subtle" /> Restore from backup…
            </Command.Item>
            <Command.Item onSelect={run(actions.showSchedules)} className={item}>
              <CalendarClock size={13} className="text-subtle" /> Scheduled queries
            </Command.Item>
            <Command.Item onSelect={run(actions.newConnection)} className={item}>
              <Plus size={13} className="text-subtle" /> New connection
            </Command.Item>
            <Command.Item onSelect={run(actions.showMcp)} className={item}>
              <Bot size={13} className="text-subtle" /> Connect an AI assistant (MCP)
            </Command.Item>
            <Command.Item onSelect={run(actions.showSettings)} className={item}>
              <Settings size={13} className="text-subtle" /> Settings · AI model
            </Command.Item>
            <Command.Item onSelect={run(actions.showDashboards)} className={item}>
              <LayoutGrid size={13} className="text-subtle" /> Dashboards…
            </Command.Item>
            {dashboards.map((x) => (
              <Command.Item key={x.label} onSelect={run(x.run)} className={item}>
                <LayoutGrid size={13} className="text-accent" /> {x.label}
              </Command.Item>
            ))}
            <Command.Item onSelect={run(actions.newNotebook)} className={item} keywords={["notes", "report", "markdown"]}>
              <NotebookPen size={13} className="text-subtle" /> New notebook
            </Command.Item>
            {notebooks.map((x) => (
              <Command.Item key={x.label} onSelect={run(x.run)} className={item}>
                <NotebookPen size={13} className="text-accent" /> {x.label}
              </Command.Item>
            ))}
            <Command.Item onSelect={run(actions.showExtensions)} className={item}>
              <Puzzle size={13} className="text-subtle" /> Extensions…
            </Command.Item>
            {extensions.map((x) => (
              <Command.Item key={x.label} onSelect={run(x.run)} className={item}>
                <Puzzle size={13} className="text-accent" /> {x.label}
              </Command.Item>
            ))}
            <Command.Item onSelect={run(actions.toggleTheme)} className={item}>
              <Moon size={13} className="text-subtle" /> Toggle light / dark
            </Command.Item>
            <Command.Item onSelect={run(() => {})} className={item} disabled>
              <Database size={13} className="text-subtle" /> Import / export — coming soon
            </Command.Item>
          </Command.Group>
        </Command.List>
      </Command>
    </div>
  );
}
