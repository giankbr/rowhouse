import { Check, ChevronsUpDown, FileInput, Pencil, Plus, Search } from "lucide-react";
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { ENVIRONMENTS, kindInfo, type ConnectionConfig } from "../lib/api";
import { mod } from "../lib/types";
import { KindBadge } from "./KindBadge";
import { readRecent } from "./Launcher";
import { Dot, Kbd, cx } from "./ui";

const ENV_ORDER = ["production", "staging", "development", "local", "none"];

function subtitle(c: ConnectionConfig) {
  const k = kindInfo(c.kind);
  if (k.file) return c.file?.split(/[\\/]/).pop() ?? "";
  return [c.host, c.database].filter(Boolean).join(" / ");
}

/** Searchable connection picker: recents first, then grouped by environment. Keyboard-first. */
export function ConnectionSwitcher({
  connections,
  activeId,
  connected,
  open,
  onOpenChange,
  onSelect,
  onEdit,
  onNew,
  onImport,
  anchor = "sidebar",
  anchorRect,
}: {
  /** where to open when there is no trigger button (collapsed rail) */
  anchorRect?: { x: number; y: number };
  connections: ConnectionConfig[];
  activeId: string | null;
  connected: Record<string, boolean>;
  open: boolean;
  onOpenChange: (o: boolean) => void;
  onSelect: (id: string) => void;
  onEdit: (c: ConnectionConfig) => void;
  onNew: () => void;
  onImport: () => void;
  anchor?: "sidebar" | "rail";
}) {
  const active = connections.find((c) => c.id === activeId) ?? null;
  const [q, setQ] = useState("");
  const [cursor, setCursor] = useState(0);
  const box = useRef<HTMLDivElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const [pos, setPos] = useState({ x: 8, y: 48 });
  useLayoutEffect(() => {
    if (!open) return;
    const r = trigger.current?.getBoundingClientRect();
    setPos(r ? { x: r.left, y: r.bottom + 4 } : (anchorRect ?? { x: 52, y: 48 }));
  }, [open, anchorRect]);

  useEffect(() => {
    if (!open) return;
    setQ("");
    setCursor(0);
    const close = (e: MouseEvent) => !box.current?.contains(e.target as Node) && onOpenChange(false);
    window.addEventListener("mousedown", close);
    return () => window.removeEventListener("mousedown", close);
  }, [open, onOpenChange]);

  const sections = useMemo(() => {
    const needle = q.trim().toLowerCase();
    const match = (c: ConnectionConfig) =>
      !needle || [c.name, c.host, c.database, c.file, kindInfo(c.kind).label, c.env].some((x) => x?.toLowerCase().includes(needle));
    const list = connections.filter(match);
    if (needle) return [{ label: `${list.length} match${list.length === 1 ? "" : "es"}`, items: list }];
    const recentIds = readRecent().filter((id) => connections.some((c) => c.id === id)).slice(0, 5);
    const out: { label: string; items: ConnectionConfig[] }[] = [];
    if (recentIds.length && connections.length > 6) out.push({ label: "Recent", items: recentIds.map((id) => connections.find((c) => c.id === id)!) });
    const byEnv = new Map<string, ConnectionConfig[]>();
    for (const c of list) {
      const e = c.env && c.env !== "none" ? c.env : "none";
      byEnv.set(e, [...(byEnv.get(e) ?? []), c]);
    }
    const envs = [...byEnv.keys()].sort((a, b) => ENV_ORDER.indexOf(a) - ENV_ORDER.indexOf(b));
    for (const e of envs) {
      const items = byEnv.get(e)!.sort((a, b) => a.name.localeCompare(b.name));
      out.push({ label: e === "none" ? (envs.length > 1 ? "Other" : "All connections") : (ENVIRONMENTS.find((x) => x.value === e)?.label ?? e), items });
    }
    return out;
  }, [connections, q]);
  const flat = sections.flatMap((s) => s.items);

  useLayoutEffect(() => {
    listRef.current?.querySelector(`[data-idx="${cursor}"]`)?.scrollIntoView({ block: "nearest" });
  }, [cursor]);

  const pick = (c: ConnectionConfig | undefined) => {
    if (!c) return;
    onOpenChange(false);
    onSelect(c.id);
  };

  let idx = -1;
  return (
    <div ref={box} className="relative">
      {anchor === "sidebar" && (
        <button
          ref={trigger}
          onClick={() => onOpenChange(!open)}
          title={`Switch connection (${mod}+O)`}
          className={cx("flex h-8 w-full items-center gap-2 rounded-md px-2 text-left hover:bg-hover", open && "bg-hover")}
        >
          {active ? (
            <>
              <KindBadge kind={active.kind} size={18} />
              <span className="min-w-0 flex-1">
                <span className="flex items-center gap-1.5 text-xs font-semibold">
                  <span className="truncate">{active.name}</span>
                  {active.color && <Dot color={active.color} className="size-1.5" />}
                </span>
              </span>
            </>
          ) : (
            <span className="flex-1 text-xs text-muted">Choose a connection</span>
          )}
          <ChevronsUpDown size={12} className="shrink-0 text-subtle" />
        </button>
      )}

      {open && (
        <div
          className={cx(
            "animate-pop fixed z-50 flex w-80 flex-col overflow-hidden rounded-xl border border-line bg-elevated shadow-pop",
          )}
          style={{ left: pos.x, top: pos.y }}
          onKeyDown={(e) => {
            if (e.key === "ArrowDown") (e.preventDefault(), setCursor((c) => Math.min(flat.length - 1, c + 1)));
            else if (e.key === "ArrowUp") (e.preventDefault(), setCursor((c) => Math.max(0, c - 1)));
            else if (e.key === "Enter") (e.preventDefault(), pick(flat[cursor]));
            else if (e.key === "Escape") (e.preventDefault(), onOpenChange(false));
          }}
        >
          <div className="flex items-center gap-2 border-b border-line px-3">
            <Search size={13} className="shrink-0 text-subtle" />
            <input
              autoFocus
              value={q}
              onChange={(e) => (setQ(e.target.value), setCursor(0))}
              placeholder={`Search ${connections.length} connections`}
              spellCheck={false}
              className="h-9 min-w-0 flex-1 bg-transparent text-xs outline-none placeholder:text-subtle"
            />
          </div>
          <div ref={listRef} className="max-h-[min(440px,60vh)] overflow-y-auto p-1">
            {flat.length === 0 && <div className="px-3 py-6 text-center text-xs text-subtle">No connection matches “{q}”</div>}
            {sections.map((s) => (
              <div key={s.label} className="mb-1">
                <div className="px-2 pt-1.5 pb-0.5 text-[10px] font-semibold tracking-wide text-subtle uppercase">{s.label}</div>
                {s.items.map((c) => {
                  idx++;
                  const i = idx;
                  return (
                    <div
                      key={s.label + c.id}
                      data-idx={i}
                      onMouseMove={() => setCursor(i)}
                      onClick={() => pick(c)}
                      className={cx("group flex h-8 cursor-default items-center gap-2 rounded-md px-2", i === cursor ? "bg-hover" : "")}
                    >
                      <KindBadge kind={c.kind} size={16} />
                      <span className="min-w-0 flex-1 leading-tight">
                        <span className="flex items-center gap-1.5 text-xs">
                          <span className="truncate font-medium">{c.name}</span>
                          {c.color && <Dot color={c.color} className="size-1.5" />}
                        </span>
                        <span className="block truncate text-[10.5px] text-subtle">{subtitle(c)}</span>
                      </span>
                      {connected[c.id] && <span className="size-1.5 shrink-0 rounded-full bg-success" title="Connected" />}
                      {c.id === activeId && <Check size={12} className="shrink-0 text-accent" />}
                      <button
                        title="Edit"
                        onClick={(e) => (e.stopPropagation(), onOpenChange(false), onEdit(c))}
                        className="hidden shrink-0 rounded p-0.5 text-subtle group-hover:block hover:bg-selected hover:text-fg"
                      >
                        <Pencil size={11} />
                      </button>
                    </div>
                  );
                })}
              </div>
            ))}
          </div>
          <div className="flex items-center gap-1 border-t border-line bg-panel/70 p-1">
            <button onClick={() => (onOpenChange(false), onNew())} className="flex h-7 items-center gap-1.5 rounded-md px-2 text-xs hover:bg-hover">
              <Plus size={12} /> New connection
            </button>
            <button onClick={() => (onOpenChange(false), onImport())} className="flex h-7 items-center gap-1.5 rounded-md px-2 text-xs text-muted hover:bg-hover hover:text-fg">
              <FileInput size={12} /> Import
            </button>
            <span className="ml-auto flex items-center gap-0.5 pr-1.5 text-[10px] text-subtle">
              <Kbd>↑</Kbd>
              <Kbd>↓</Kbd> <Kbd>↵</Kbd>
            </span>
          </div>
        </div>
      )}
    </div>
  );
}
