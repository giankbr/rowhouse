import { Bot, Check, ChevronDown, Command, Database, Moon, PanelLeft, Search, Settings, Sun, Wrench } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { api, ENVIRONMENTS, kindInfo, type ConnectionConfig } from "../lib/api";
import { isMac, mod } from "../lib/types";
import { Button, Dot, Kbd, Menu, cx } from "./ui";

export function Titlebar({
  conn,
  dark,
  onToggleTheme,
  onToggleSidebar,
  onPalette,
  onMcp,
  onSettings,
  onHome,
  database,
  onSwitchDb,
  onTool,
  onSwitch,
}: {
  onSwitch?: () => void;
  /** database picked this session (overrides the profile's) */
  database?: string | null;
  onSwitchDb: (db: string) => void;
  onTool: (tool: "builder" | "erd" | "compare" | "schedules" | "backup" | "restore" | "activity" | "roles" | "runfile" | "extensions" | "dashboards") => void;
  conn: ConnectionConfig | null;
  dark: boolean;
  onToggleTheme: () => void;
  onToggleSidebar: () => void;
  onPalette: () => void;
  onMcp: () => void;
  onSettings: () => void;
  onHome: () => void;
}) {
  return (
    <header
      data-tauri-drag-region
      className={cx("flex h-10 shrink-0 items-center gap-2 border-b border-line bg-panel pr-2", isMac ? "pl-[78px]" : "pl-2")}
    >
      {conn && (
        <Button variant="ghost" size="icon" onClick={onToggleSidebar} title="Toggle sidebar">
          <PanelLeft size={14} />
        </Button>
      )}

      <div data-tauri-drag-region className="flex min-w-0 flex-1 items-center gap-2 text-xs">
        <button onClick={onHome} title="All connections" className="rounded px-1 font-semibold tracking-tight hover:bg-hover">
          Rowhouse
        </button>
        {conn && (
          <>
            <span className="text-subtle">/</span>
            <button onClick={onSwitch} title={`Switch connection (${mod}+O)`} className="flex min-w-0 items-center gap-1.5 rounded px-1 hover:bg-hover">
              <Dot color={conn.color} />
              <span className="truncate font-medium">{conn.name}</span>
              <ChevronDown size={11} className="shrink-0 text-subtle" />
            </button>
            <span className="truncate text-muted">
              {kindInfo(conn.kind).file ? conn.file : `${conn.user ?? ""}@${conn.host ?? ""}`}
            </span>
            {!kindInfo(conn.kind).file && !kindInfo(conn.kind).api && (
              <DatabasePicker connId={conn.id} current={database ?? conn.database ?? null} onSwitch={onSwitchDb} />
            )}
            {conn.env && conn.env !== "none" && (
              <span
                className={cx(
                  "rounded px-1.5 py-px text-[10px] font-semibold uppercase",
                  conn.env === "production" ? "bg-danger text-white" : conn.env === "staging" ? "bg-warning/20 text-warning" : "bg-hover text-muted",
                )}
              >
                {ENVIRONMENTS.find((e) => e.value === conn.env)?.label}
              </span>
            )}
            {conn.readOnly && (
              <span className="rounded bg-warning/15 px-1.5 py-px text-[10px] font-semibold text-warning">READ-ONLY</span>
            )}
          </>
        )}
      </div>

      {conn && (
        <Menu
          label={
            <>
              <Wrench size={12} /> Tools
            </>
          }
          items={[
            { label: "Dashboards", onSelect: () => onTool("dashboards") },
            { label: "Query builder", onSelect: () => onTool("builder") },
            { label: "Diagram (ERD)", onSelect: () => onTool("erd") },
            { label: "Compare schema / data", onSelect: () => onTool("compare") },
            { label: "Scheduled queries", onSelect: () => onTool("schedules") },
            { label: "Server activity", hint: "processes", onSelect: () => onTool("activity") },
            { label: "Users & roles", onSelect: () => onTool("roles") },
            { label: "Run SQL file…", onSelect: () => onTool("runfile") },
            { label: "Back up database…", onSelect: () => onTool("backup") },
            { label: "Restore from backup…", onSelect: () => onTool("restore") },
            { label: "Extensions…", onSelect: () => onTool("extensions") },
          ]}
        />
      )}
      <button
        onClick={onPalette}
        className="flex h-6 w-60 items-center gap-2 rounded-md border border-line bg-elevated px-2 text-2xs text-subtle hover:border-line-strong"
      >
        <Command size={11} />
        <span className="flex-1 truncate text-left whitespace-nowrap">Jump to table or command…</span>
        <Kbd>{mod}</Kbd>
        <Kbd>K</Kbd>
      </button>
      <Button variant="ghost" size="icon" onClick={onMcp} title="Connect an AI assistant (MCP)">
        <Bot size={14} />
      </Button>
      <Button variant="ghost" size="icon" onClick={onSettings} title="Settings (AI)">
        <Settings size={14} />
      </Button>
      <Button variant="ghost" size="icon" onClick={onToggleTheme} title="Toggle theme">
        {dark ? <Sun size={14} /> : <Moon size={14} />}
      </Button>
    </header>
  );
}

/** Current database with a dropdown of the others on the same server. */
function DatabasePicker({ connId, current, onSwitch }: { connId: string; current: string | null; onSwitch: (db: string) => void }) {
  const [open, setOpen] = useState(false);
  const [list, setList] = useState<string[] | null>(null);
  const [q, setQ] = useState("");
  const [err, setErr] = useState<string | null>(null);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    setQ("");
    setErr(null);
    api.listDatabases(connId).then(setList).catch((e) => (setErr(String(e)), setList([])));
    const close = (e: MouseEvent) => !ref.current?.contains(e.target as Node) && setOpen(false);
    window.addEventListener("mousedown", close);
    return () => window.removeEventListener("mousedown", close);
  }, [open, connId]);
  const shown = (list ?? []).filter((d) => d.toLowerCase().includes(q.toLowerCase()));
  return (
    <div ref={ref} className="relative">
      <button
        onClick={() => setOpen((o) => !o)}
        title="Switch database"
        className="flex h-6 items-center gap-1 rounded-md px-1.5 text-muted hover:bg-hover hover:text-fg"
      >
        <Database size={11} />
        <span className="max-w-40 truncate">{current || "default"}</span>
        <ChevronDown size={11} />
      </button>
      {open && (
        <div className="animate-pop absolute top-7 left-0 z-50 w-60 rounded-lg border border-line bg-elevated p-1 shadow-pop">
          <div className="relative mb-1">
            <Search size={11} className="pointer-events-none absolute top-1/2 left-2 -translate-y-1/2 text-subtle" />
            <input
              autoFocus
              value={q}
              onChange={(e) => setQ(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Escape") setOpen(false);
                if (e.key === "Enter" && shown[0]) (onSwitch(shown[0]), setOpen(false));
              }}
              placeholder="Find database…"
              className="h-7 w-full rounded-md bg-panel pr-2 pl-6 text-xs outline-none placeholder:text-subtle"
            />
          </div>
          <div className="max-h-64 overflow-y-auto">
            {list === null && <div className="px-2 py-1.5 text-2xs text-subtle">Loading…</div>}
            {err && <div className="px-2 py-1.5 text-2xs text-danger">{err}</div>}
            {list && !err && shown.length === 0 && <div className="px-2 py-1.5 text-2xs text-subtle">No database matches</div>}
            {shown.map((d) => (
              <button
                key={d}
                onClick={() => (d !== current && onSwitch(d), setOpen(false))}
                className="flex h-7 w-full items-center gap-2 rounded-md px-2 text-left text-xs hover:bg-hover"
              >
                <Check size={11} className={cx("text-accent", d !== current && "invisible")} />
                <span className="truncate font-mono text-[11.5px]">{d}</span>
              </button>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
