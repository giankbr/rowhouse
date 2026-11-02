import { ArrowRight, Bot, Check, FileInput, FolderOpen, Link2, Loader2, Lock, Pencil, Plus, Search, Settings, Sparkles, X } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { api, ENVIRONMENTS, isTauri, kindInfo, type ConnectionConfig, type DbKind, type Environment, type DetectedImport, type LicenseStatus, type NameSuggestion } from "../lib/api";
import { mod } from "../lib/types";
import { KindBadge } from "./KindBadge";
import { Button, Kbd, Modal, cx } from "./ui";

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

const ENV_DOT: Record<Environment, string> = {
  none: "var(--subtle)",
  local: "#22c55e",
  development: "#14b8a6",
  staging: "#eab308",
  production: "#ef4444",
};

/** Order environments appear as sections / rail entries. */
const ENV_ORDER: Environment[] = ["production", "staging", "development", "local", "none"];
const envLabel = (e: Environment) => (e === "none" ? "Other" : ENVIRONMENTS.find((x) => x.value === e)?.label ?? e);

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

type Filter = { t: "all" } | { t: "recent" } | { t: "env"; v: Environment } | { t: "kind"; v: DbKind };

/**
 * Start screen: a filter rail on the left (All / Recent / per-environment /
 * per-engine), a searchable, grouped connection list on the right. ↑/↓ move, Enter
 * opens. "Tidy names" asks the configured model to clean up messy names.
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
  onRenamed,
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
  /** persist renamed connections back into app state */
  onRenamed: (saved: ConnectionConfig[]) => void;
}) {
  const [q, setQ] = useState("");
  const [sel, setSel] = useState(0);
  const [filter, setFilter] = useState<Filter>({ t: "all" });
  const listRef = useRef<HTMLDivElement>(null);
  const [detected, setDetected] = useState<DetectedImport[]>([]);
  const [lic, setLic] = useState<LicenseStatus | null>(null);
  const [tidy, setTidy] = useState(false);
  useEffect(() => {
    api.importDetect().then(setDetected).catch(() => setDetected([]));
    api.license.status().then(setLic).catch(() => {});
  }, []);
  const APP_LABEL = { tableplus: "TablePlus", dbeaver: "DBeaver", navicat: "Navicat" } as const;

  const recent = useMemo(() => new Set(readRecent()), [connections]);
  const rank = useMemo(() => {
    const order = readRecent();
    return (c: ConnectionConfig) => {
      const i = order.indexOf(c.id);
      return i < 0 ? 1e6 : i;
    };
  }, [connections]);

  // Which envs / engines are actually present, with counts, for the rail.
  const envCounts = useMemo(() => {
    const m = new Map<Environment, number>();
    for (const c of connections) m.set(c.env ?? "none", (m.get(c.env ?? "none") ?? 0) + 1);
    return ENV_ORDER.filter((e) => m.has(e)).map((e) => [e, m.get(e)!] as const);
  }, [connections]);
  const kindCounts = useMemo(() => {
    const m = new Map<DbKind, number>();
    for (const c of connections) m.set(c.kind, (m.get(c.kind) ?? 0) + 1);
    return [...m.entries()].sort((a, b) => b[1] - a[1] || kindInfo(a[0]).label.localeCompare(kindInfo(b[0]).label));
  }, [connections]);
  const recentCount = useMemo(() => connections.filter((c) => recent.has(c.id)).length, [connections, recent]);

  // Connections passing the active filter + search, sorted recent-first.
  const matched = useMemo(() => {
    const needle = q.trim().toLowerCase();
    const bySearch = (c: ConnectionConfig) =>
      !needle ||
      [c.name, c.host, c.database, c.file, kindInfo(c.kind).label, c.env, c.ssh?.host]
        .filter(Boolean)
        .some((x) => String(x).toLowerCase().includes(needle));
    const byFilter = (c: ConnectionConfig) =>
      filter.t === "all" ? true : filter.t === "recent" ? recent.has(c.id) : filter.t === "env" ? (c.env ?? "none") === filter.v : c.kind === filter.v;
    return connections.filter((c) => bySearch(c) && byFilter(c)).sort((a, b) => rank(a) - rank(b) || a.name.localeCompare(b.name));
  }, [connections, q, filter, recent, rank]);

  // In "All", break the list into environment sections; otherwise one flat group.
  const groups = useMemo(() => {
    if (filter.t !== "all") return [{ label: null as string | null, items: matched }];
    return ENV_ORDER.map((e) => ({ label: envLabel(e), items: matched.filter((c) => (c.env ?? "none") === e) })).filter((g) => g.items.length);
  }, [filter, matched]);
  const flat = useMemo(() => groups.flatMap((g) => g.items), [groups]);

  useEffect(() => setSel(0), [q, filter]);
  useEffect(() => {
    listRef.current?.querySelector(`[data-i="${sel}"]`)?.scrollIntoView({ block: "nearest" });
  }, [sel]);

  const onKey = (e: React.KeyboardEvent) => {
    if (e.key === "ArrowDown") (e.preventDefault(), setSel((s) => Math.min(flat.length - 1, s + 1)));
    else if (e.key === "ArrowUp") (e.preventDefault(), setSel((s) => Math.max(0, s - 1)));
    else if (e.key === "Enter" && flat[sel]) (e.preventDefault(), onOpen(flat[sel].id));
  };

  const action = "flex h-9 w-full items-center gap-2.5 whitespace-nowrap rounded-lg border border-line bg-elevated/60 px-3 text-[13px] text-fg/90 transition-colors hover:border-line-strong hover:bg-elevated";
  const railItem = (active: boolean) =>
    cx("flex h-7 w-full items-center gap-2 rounded-md px-2 text-left text-[12.5px] transition-colors", active ? "bg-selected text-fg" : "text-fg/75 hover:bg-hover");
  const count = (n: number) => <span className="ml-auto shrink-0 text-[11px] tabular-nums text-subtle">{n}</span>;

  let gi = -1; // running index across groups for data-i / selection

  return (
    <div className="flex min-h-0 flex-1 overflow-hidden bg-elevated">
      <div className="flex min-h-0 w-full overflow-hidden">
        {/* filter rail */}
        <div data-tauri-drag-region className="flex w-56 shrink-0 flex-col border-r border-line bg-panel">
          <div className="flex items-center gap-2 px-3.5 pt-3.5 pb-3">
            <span className="pointer-events-none text-fg">
              <RowhouseMark size={26} />
            </span>
            <span className="min-w-0">
              <span className="block text-[13.5px] font-semibold leading-none tracking-tight">Rowhouse</span>
              <span className="block text-[10.5px] leading-tight text-subtle">
                {version} · {lic?.license ? (lic.license.plan === "team" ? "Team" : "Pro") : "open source"}
              </span>
            </span>
          </div>

          <div className="min-h-0 flex-1 overflow-y-auto px-2 pb-1">
            <button className={railItem(filter.t === "all")} onClick={() => setFilter({ t: "all" })}>
              All {count(connections.length)}
            </button>
            {recentCount > 0 && (
              <button className={railItem(filter.t === "recent")} onClick={() => setFilter({ t: "recent" })}>
                Recent {count(recentCount)}
              </button>
            )}
            {envCounts.length > 1 && (
              <>
                <div className="px-2 pt-3 pb-1 text-[10px] font-medium tracking-wide text-subtle uppercase">Environment</div>
                {envCounts.map(([e, n]) => (
                  <button key={e} className={railItem(filter.t === "env" && filter.v === e)} onClick={() => setFilter({ t: "env", v: e })}>
                    <span className="size-2 shrink-0 rounded-full" style={{ background: ENV_DOT[e] }} />
                    <span className="truncate">{envLabel(e)}</span>
                    {count(n)}
                  </button>
                ))}
              </>
            )}
            {kindCounts.length > 1 && (
              <>
                <div className="px-2 pt-3 pb-1 text-[10px] font-medium tracking-wide text-subtle uppercase">Engine</div>
                {kindCounts.map(([k, n]) => (
                  <button key={k} className={railItem(filter.t === "kind" && filter.v === k)} onClick={() => setFilter({ t: "kind", v: k })}>
                    <KindBadge kind={k} size={16} />
                    <span className="truncate">{kindInfo(k).label}</span>
                    {count(n)}
                  </button>
                ))}
              </>
            )}
          </div>

          <div className="flex flex-col gap-1.5 border-t border-line p-2.5">
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
            {connections.length > 1 && (
              <button
                onClick={() => setTidy(true)}
                title="Tidy connection names with AI"
                className="flex h-9 shrink-0 items-center gap-1.5 rounded-full border border-line px-3 text-[12.5px] text-muted hover:border-accent/60 hover:text-accent"
              >
                <Sparkles size={14} /> Tidy names
              </button>
            )}
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
            {connections.length > 0 && flat.length === 0 && (
              <div className="p-8 text-center text-xs text-subtle">
                {q ? <>No connection matches “{q}”</> : <>Nothing here yet</>}
              </div>
            )}
            {groups.map((g) => (
              <div key={g.label ?? "_"}>
                {g.label && groups.length > 1 && (
                  <div className="px-2.5 pt-3 pb-1 text-[10px] font-medium tracking-wide text-subtle uppercase">{g.label}</div>
                )}
                {g.items.map((c) => {
                  gi += 1;
                  const i = gi;
                  const env = c.env && c.env !== "none" ? ENVIRONMENTS.find((e) => e.value === c.env) : undefined;
                  return (
                    <div
                      key={c.id}
                      data-i={i}
                      onMouseEnter={() => setSel(i)}
                      onClick={() => onOpen(c.id)}
                      className={cx("group flex cursor-default items-center gap-2.5 rounded-lg px-2.5 py-1.5", i === sel ? "bg-selected" : "hover:bg-hover")}
                    >
                      <KindBadge kind={c.kind} size={26} />
                      <div className="min-w-0 flex-1">
                        <div className="flex items-center gap-2">
                          <span className="truncate text-[13px] font-medium">{c.name || "Untitled"}</span>
                          {env && <span className={cx("shrink-0 rounded-md px-1.5 py-px text-[11.5px] font-medium", ENV_STYLE[c.env!])}>{env.label.toLowerCase()}</span>}
                          {c.readOnly && <Lock size={12} className="shrink-0 text-subtle" />}
                        </div>
                        <div className="truncate text-[11px] text-muted">{subtitle(c)}</div>
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
            ))}
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

      {tidy && <TidyNamesDialog connections={connections} onClose={() => setTidy(false)} onApplied={onRenamed} onSettings={onSettings} />}
    </div>
  );
}

/** AI name tidier: fetch suggestions, let the user approve each, then save. */
function TidyNamesDialog({
  connections,
  onClose,
  onApplied,
  onSettings,
}: {
  connections: ConnectionConfig[];
  onClose: () => void;
  onApplied: (saved: ConnectionConfig[]) => void;
  onSettings: () => void;
}) {
  const [state, setState] = useState<"loading" | "ready" | "clean" | "error" | "saving">("loading");
  const [error, setError] = useState("");
  const [rows, setRows] = useState<{ from: string; to: string; conn: ConnectionConfig; on: boolean }[]>([]);

  useEffect(() => {
    let alive = true;
    api.ai
      .suggestNames(
        connections.map((c) => ({ id: c.id, name: c.name, kind: c.kind, host: c.host, database: c.database, ssh_host: c.ssh?.host, env: c.env })),
      )
      .then((sug: NameSuggestion[]) => {
        if (!alive) return;
        const byId = new Map(connections.map((c) => [c.id, c]));
        const next = sug
          .map((s) => ({ conn: byId.get(s.id)!, from: byId.get(s.id)?.name ?? "", to: s.name, on: true }))
          .filter((r) => r.conn);
        setRows(next);
        setState(next.length ? "ready" : "clean");
      })
      .catch((e: unknown) => {
        if (!alive) return;
        setError(String((e as { message?: string })?.message ?? e).replace(/^ai:\s*/, ""));
        setState("error");
      });
    return () => {
      alive = false;
    };
  }, [connections]);

  const apply = async () => {
    setState("saving");
    const saved: ConnectionConfig[] = [];
    for (const r of rows.filter((x) => x.on)) {
      try {
        saved.push(await api.saveConnection({ ...r.conn, name: r.to }));
      } catch {
        /* skip ones that fail to save */
      }
    }
    onApplied(saved);
    onClose();
  };

  const chosen = rows.filter((r) => r.on).length;

  return (
    <Modal
      open
      onClose={onClose}
      width={560}
      title={
        <span className="flex items-center gap-2">
          <Sparkles size={15} className="text-accent" /> Tidy connection names
        </span>
      }
      footer={
        state === "ready" || state === "saving" ? (
          <>
            <span className="text-xs text-subtle">
              {chosen} of {rows.length} selected
            </span>
            <span className="flex-1" />
            <Button onClick={onClose} disabled={state === "saving"}>
              Cancel
            </Button>
            <Button variant="primary" onClick={apply} disabled={state === "saving" || chosen === 0}>
              {state === "saving" ? "Applying…" : `Rename ${chosen}`}
            </Button>
          </>
        ) : undefined
      }
    >
      {state === "loading" && (
        <div className="flex items-center justify-center gap-2 py-10 text-sm text-muted">
          <Loader2 size={16} className="animate-spin" /> Asking the model…
        </div>
      )}
      {state === "clean" && <div className="py-10 text-center text-sm text-muted">Your connection names already look clean. 👍</div>}
      {state === "error" && (
        <div className="flex flex-col items-center gap-3 py-8 text-center">
          <p className="max-w-sm text-sm text-danger">{error || "Couldn't get suggestions."}</p>
          <p className="max-w-sm text-xs text-subtle">This needs an AI model configured. Set a provider and API key in settings, then try again.</p>
          <Button onClick={() => (onClose(), onSettings())}>Open AI settings</Button>
        </div>
      )}
      {(state === "ready" || state === "saving") && (
        <div className="flex flex-col gap-1">
          {rows.map((r, i) => (
            <button
              key={r.conn.id}
              onClick={() => setRows((rs) => rs.map((x, j) => (j === i ? { ...x, on: !x.on } : x)))}
              disabled={state === "saving"}
              className="flex items-center gap-3 rounded-lg px-2 py-1.5 text-left hover:bg-hover"
            >
              <span className={cx("flex size-4 shrink-0 items-center justify-center rounded border", r.on ? "border-accent bg-accent text-white" : "border-line")}>
                {r.on && <Check size={12} />}
              </span>
              <KindBadge kind={r.conn.kind} size={22} />
              <span className="flex min-w-0 flex-1 items-center gap-1.5 text-[13px]">
                <span className="truncate text-muted line-through">{r.from || "Untitled"}</span>
                <ArrowRight size={13} className="shrink-0 text-subtle" />
                <span className="truncate font-medium">{r.to}</span>
              </span>
            </button>
          ))}
          <p className="mt-2 px-2 text-[11px] text-subtle">
            <X size={11} className="mr-1 inline align-[-1px]" />
            Only names you keep checked are changed. Hosts, databases and passwords stay the same.
          </p>
        </div>
      )}
    </Modal>
  );
}
