/** Open tabs survive restarts: saved (without results) to localStorage, restored per connection. */
import type { Tab } from "./types";

const KEY = "rh-session-v1";

interface Saved {
  tabs: Tab[];
  active: [string | null, string | null];
}

const KEEP: (keyof Tab)[] = ["id", "kind", "pane", "title", "connId", "sql", "schema", "table", "view", "filters", "sort", "pageSize", "snippetId", "qb"];

export function saveSession(tabs: Tab[], active: [string | null, string | null]) {
  const slim = tabs.map((t) => Object.fromEntries(KEEP.filter((k) => t[k] !== undefined).map((k) => [k, t[k]])) as unknown as Tab);
  try {
    localStorage.setItem(KEY, JSON.stringify({ tabs: slim, active } satisfies Saved));
  } catch {
    /* storage full / unavailable */
  }
}

export function loadSession(): Saved {
  try {
    const s = JSON.parse(localStorage.getItem(KEY) ?? "null") as Saved | null;
    if (s && Array.isArray(s.tabs)) return { tabs: s.tabs.map((t) => ({ ...t, stale: t.kind === "table" })), active: s.active ?? [null, null] };
  } catch {
    /* corrupt: start fresh */
  }
  return { tabs: [], active: [null, null] };
}
