import { ArrowRight, Bot, FileInput, FolderOpen, Link2, Lock, Pencil, Plus, Search, Settings } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { api, ENVIRONMENTS, isTauri, kindInfo, type ConnectionConfig, type DbKind, type DetectedImport, type Environment } from "../lib/api";
import { mod } from "../lib/types";
import { KindBadge } from "./KindBadge";
import { Kbd, cx } from "./ui";

const RECENT_KEY = "rh-recent-connections";

export function readRecent(): string[] {
  try {
    return JSON.parse(localStorage.getItem(RECENT_KEY) ?? "[]");
  } catch {
    return [];
  }
}

export function touchRecent(id: string) {
  try {
    localStorage.setItem(RECENT_KEY, JSON.stringify([id, ...readRecent().filter((x) => x !== id)].slice(0, 50)));
  } catch {
    /* storage unavailable */
  }
}

const ENV_STYLE: Record<Environment, string> = {
  none: "",
  local: "bg-success/15 text-success",
  development: "bg-[#14b8a6]/15 text-[#0d9488] dark:text-[#2dd4bf]",
  staging: "bg-warning/15 text-warning",
  production: "bg-danger/15 text-danger",
};

/** The house mark from the landing page. */
export function RowhouseMark({ size = 96 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 32 32" aria-hidden>
      <path d="M4 13.5 L16 4 L28 13.5" fill="none" stroke="var(--accent)" strokeWidth="2.4" strokeLinejoin="round" strokeLinecap="round" />
      <rect x="6.5" y="13" width="19" height="15" rx="2.5" fill="none" stroke="currentColor" strokeWidth="2.1" />
      <line x1="6.5" y1="18.2" x2="25.5" y2="18.2" stroke="currentColor" strokeWidth="1.6" opacity="0.5" />
      <line x1="6.5" y1="23" x2="25.5" y2="23" stroke="currentColor" strokeWidth="1.6" opacity="0.5" />
      <rect x="6.5" y="23" width="19" height="5" fill="var(--accent)" opacity="0.18" />
      <rect x="6.5" y="13" width="3.6" height="15" fill="var(--accent)" opacity="0.14" />
    </svg>
  );
}

function subtitle(c: ConnectionConfig) {
  const k = kindInfo(c.kind);
  if (k.file) return c.file?.split(/[\\/]/).pop() ?? "no file";
  if (c.kind === "turso") return c.host?.replace(/^\w+:\/\//, "") ?? "Turso";
  if (c.kind === "d1") return `Cloudflare D1 · ${c.database ?? "?"}`;
  const parts: string[] = [];
  if (c.ssh) parts.push(`SSH · ${c.ssh.host || "?"}`);
  else parts.push(c.host ?? "localhost");
  if (c.database) parts.push(c.database);
  return parts.join(" : ");
}

/**
 * Start screen: brand + actions on the left, every saved connection on the right
 * (search, ↑/↓, Enter to open, most-recent first).
 */
export function Launcher({
  connections,
  version,
  onOpen,
  onEdit,
  onNew,
  onImport,
  onOpenFile,
  onMcp,
  onSettings,
}: {
  connections: ConnectionConfig[];
  version: string;
  onOpen: (id: string) => void;
  onEdit: (c: ConnectionConfig) => void;
  onNew: (kind?: DbKind) => void;
  /** with a path: open that detected file's import preview */
  onImport: (path?: string) => void;
  onOpenFile: () => void;
  onMcp: () => void;
  onSettings: () => void;
}) {
  const [q, setQ] = useState("");
  const [sel, setSel] = useState(0);
  const listRef = useRef<HTMLDivElement>(null);
  const [detected, setDetected] = useState<DetectedImport[]>([]);
  useEffect(() => {
    api.importDetect().then(setDetected).catch(() => setDetected([]));
  }, []);
  const APP_LABEL = { tableplus: "TablePlus", dbeaver: "DBeaver", navicat: "Navicat" } as const;

  const list = useMemo(() => {
    const recent = readRecent();
    const rank = (c: ConnectionConfig) => {
      const i = recent.indexOf(c.id);
      return i < 0 ? 1e6 : i;
    };
    const needle = q.trim().toLowerCase();
    return connections
      .filter((c) =>
        !needle
          ? true
          : [c.name, c.host, c.database, c.file, kindInfo(c.kind).label, c.env, c.ssh?.host]
              .filter(Boolean)
              .some((x) => String(x).toLowerCase().includes(needle)),
      )
      .sort((a, b) => rank(a) - rank(b) || a.name.localeCompare(b.name));
  }, [connections, q]);

  useEffect(() => setSel(0), [q]);
  useEffect(() => {
    listRef.current?.querySelector(`[data-i="${sel}"]`)?.scrollIntoView({ block: "nearest" });
  }, [sel]);

  const onKey = (e: React.KeyboardEvent) => {
    if (e.key === "ArrowDown") (e.preventDefault(), setSel((s) => Math.min(list.length - 1, s + 1)));
    else if (e.key === "ArrowUp") (e.preventDefault(), setSel((s) => Math.max(0, s - 1)));
    else if (e.key === "Enter" && list[sel]) (e.preventDefault(), onOpen(list[sel].id));
  };

  const action = "flex h-9 w-full items-center gap-2.5 whitespace-nowrap rounded-lg border border-line bg-elevated/60 px-3 text-[13px] text-fg/90 transition-colors hover:border-line-strong hover:bg-elevated";

  return (
    <div className="flex min-h-0 flex-1 overflow-hidden bg-elevated">
      <div className="flex min-h-0 w-full overflow-hidden">
        {/* brand + actions */}
        <div data-tauri-drag-region className="flex w-[34%] min-w-60 flex-col items-center border-r border-line bg-panel px-6 pt-12 pb-5">
          <div className="pointer-events-none text-fg">
            <RowhouseMark size={72} />
          </div>
          <h1 className="pointer-events-none mt-2.5 text-[24px] font-semibold tracking-tight">Rowhouse</h1>
          <span className="mt-0.5 text-xs text-muted">Version {version}</span>
          <span className="mt-3 rounded-full bg-success/12 px-2.5 py-0.5 text-[11px] font-medium text-success">Free &amp; open source</span>
          <div className="mt-auto flex w-full flex-col gap-1.5">
            <button className={action} onClick={() => onNew()}>
              <Plus size={16} className="text-accent" /> Create connection
            </button>
            <button className={action} onClick={() => onImport()}>
              <FileInput size={16} className="shrink-0 text-muted" /> <span className="truncate">Import connections</span>
              {detected.length > 0 && (
                <span className="ml-auto shrink-0 rounded-full bg-accent/12 px-1.5 text-2xs font-medium text-accent" title="Found in other apps">
                  {detected.reduce((n, d) => n + d.count, 0)}
                </span>
              )}
            </button>
            <button className={action} onClick={onOpenFile}>
              <FolderOpen size={16} className="text-muted" /> Open SQLite / DuckDB file
            </button>
            <button className={action} onClick={onMcp}>
              <Bot size={16} className="text-muted" /> Connect an AI assistant
            </button>
          </div>
        </div>

        {/* connections */}
        <div className="flex min-w-0 flex-1 flex-col" onKeyDown={onKey}>
          <div data-tauri-drag-region className="flex items-center gap-2.5 px-4 pt-4 pb-2.5">
            <button
              onClick={() => onNew()}
              title="New connection"
              className="flex size-9 shrink-0 items-center justify-center rounded-full border border-line text-muted hover:border-line-strong hover:text-fg"
            >
              <Plus size={17} />
            </button>
            <div className="relative flex-1">
              <Search size={15} className="pointer-events-none absolute top-1/2 left-3.5 -translate-y-1/2 text-subtle" />
              <input
                autoFocus
                value={q}
                onChange={(e) => setQ(e.target.value)}
                placeholder="Search for connection…"
                spellCheck={false}
                className="h-9 w-full rounded-full border border-line bg-panel pr-4 pl-10 text-[13px] outline-none transition-colors placeholder:text-subtle focus:border-accent/70 focus:bg-elevated"
              />
            </div>
            <button
              onClick={onSettings}
              title="Settings"
              className="flex size-9 shrink-0 items-center justify-center rounded-full text-muted hover:bg-hover hover:text-fg"
            >
              <Settings size={16} />
            </button>
          </div>

          <div ref={listRef} className="flex-1 overflow-y-auto px-3 pb-4">
            {connections.length === 0 && (
              <div className="flex flex-col gap-4 px-2 pt-2">
                {detected.length > 0 && (
                  <section className="flex flex-col gap-2">
                    <h3 className="text-[11px] font-medium tracking-wide text-subtle uppercase">Found on this computer</h3>
                    {detected.map((d) => (
                      <button
                        key={d.path}
                        onClick={() => onImport(d.path)}
                        className="group flex items-center gap-3 rounded-xl border border-line px-3 py-2.5 text-left transition-colors hover:border-accent/60 hover:bg-accent-soft/30"
                      >
                        <span className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-accent/12 text-xs font-semibold text-accent">
                          {APP_LABEL[d.source].slice(0, 2)}
                        </span>
                        <span className="min-w-0 flex-1">
                          <span className="block text-[13px] font-medium">
                            {d.count} connection{d.count === 1 ? "" : "s"} in {APP_LABEL[d.source]}
                          </span>
                          <span className="block truncate text-[11.5px] text-muted">One click · passwords are asked on first connect</span>
                        </span>
                        <span className="flex items-center gap-1 text-xs font-medium text-accent opacity-80 group-hover:opacity-100">
                          Import <ArrowRight size={13} />
                        </span>
                      </button>
                    ))}
                  </section>
                )}
                <section className="flex flex-col gap-2">
                  <h3 className="text-[11px] font-medium tracking-wide text-subtle uppercase">Start with</h3>
                  <div className="grid grid-cols-3 gap-2">
                    {(["postgres", "mysql", "sqlite", "supabase", "redis", "mongodb"] as DbKind[]).map((k) => (
                      <button
                        key={k}
                        onClick={() => (kindInfo(k).file ? onOpenFile() : onNew(k))}
                        className="flex items-center gap-2 rounded-lg border border-line px-2.5 py-2 text-left text-[12.5px] transition-colors hover:border-line-strong hover:bg-hover"
                      >
                        <KindBadge kind={k} size={22} />
                        {kindInfo(k).label}
                      </button>
                    ))}
                  </div>
                  <button
                    onClick={() => onNew()}
                    className="flex items-center gap-2 rounded-lg border border-dashed border-line px-3 py-2 text-left text-[12.5px] text-muted transition-colors hover:border-line-strong hover:text-fg"
                  >
                    <Link2 size={14} /> Paste a connection URL, or pick from {"20+"} databases…
                  </button>
                </section>
              </div>
            )}
            {connections.length > 0 && list.length === 0 && (
              <div className="p-8 text-center text-xs text-subtle">No connection matches “{q}”</div>
            )}
            {list.map((c, i) => {
              const env = c.env && c.env !== "none" ? ENVIRONMENTS.find((e) => e.value === c.env) : undefined;
              return (
                <div
                  key={c.id}
                  data-i={i}
                  onMouseEnter={() => setSel(i)}
                  onClick={() => onOpen(c.id)}
                  className={cx(
                    "group flex cursor-default items-center gap-3 rounded-lg px-2.5 py-2",
                    i === sel ? "bg-selected" : "hover:bg-hover",
                  )}
                >
                  <KindBadge kind={c.kind} size={32} />
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2">
                      <span className="truncate text-[13.5px] font-medium">{c.name || "Untitled"}</span>
                      {env && <span className={cx("shrink-0 rounded-md px-1.5 py-px text-[11.5px] font-medium", ENV_STYLE[c.env!])}>{env.label.toLowerCase()}</span>}
                      {c.readOnly && <Lock size={12} className="shrink-0 text-subtle" />}
                    </div>
                    <div className="truncate text-[11.5px] text-muted">{subtitle(c)}</div>
                  </div>
                  <button
                    onClick={(e) => (e.stopPropagation(), onEdit(c))}
                    title="Edit connection"
                    className="hidden rounded-md p-1.5 text-subtle group-hover:block hover:bg-hover hover:text-fg"
                  >
                    <Pencil size={13} />
                  </button>
                </div>
              );
            })}
          </div>
          <div className="flex items-center gap-3 border-t border-line px-4 py-1.5 text-[11px] text-subtle">
            <span className="flex items-center gap-1">
              <Kbd>↑</Kbd>
              <Kbd>↓</Kbd> select
            </span>
            <span className="flex items-center gap-1">
              <Kbd>↵</Kbd> connect
            </span>
            <span className="flex items-center gap-1">
              <Kbd>{mod}</Kbd>
              <Kbd>K</Kbd> commands
            </span>
            <span className="ml-auto">
              {connections.length} connection{connections.length === 1 ? "" : "s"}
              {!isTauri && " · preview"}
            </span>
          </div>
        </div>
      </div>
    </div>
  );
}
