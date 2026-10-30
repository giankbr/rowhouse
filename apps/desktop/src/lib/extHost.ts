// Extension host: builds the sandboxed page for an extension panel and
// answers its `window.rowhouse` calls, checking the permissions the user
// granted. Extensions run in an iframe with an opaque origin (no access to
// the app, its storage or Tauri) and a CSP that only allows the network
// hosts they declared.

import type { ExtensionInfo } from "./api";

/** Injected into every extension page: the `window.rowhouse` API. */
const SDK = `
(() => {
  let seq = 0;
  const pending = new Map();
  const listeners = new Map();
  addEventListener("message", (e) => {
    const m = e.data;
    if (!m || m.rh !== 1) return;
    if (m.reply !== undefined) {
      const p = pending.get(m.reply);
      if (!p) return;
      pending.delete(m.reply);
      m.error !== undefined ? p.reject(new Error(m.error)) : p.resolve(m.result);
    } else if (m.event) {
      if (m.event === "theme") applyTheme(m.data);
      (listeners.get(m.event) || []).forEach((fn) => { try { fn(m.data); } catch (err) { console.error(err); } });
    }
  });
  function call(method, params) {
    const id = ++seq;
    return new Promise((resolve, reject) => {
      pending.set(id, { resolve, reject });
      parent.postMessage({ rh: 1, id, method, params }, "*");
    });
  }
  function applyTheme(t) {
    if (!t) return;
    const r = document.documentElement;
    r.dataset.theme = t.mode;
    for (const [k, v] of Object.entries(t.vars || {})) r.style.setProperty(k, v);
  }
  const on = (ev, fn) => {
    listeners.set(ev, [...(listeners.get(ev) || []), fn]);
    return () => listeners.set(ev, (listeners.get(ev) || []).filter((f) => f !== fn));
  };
  window.rowhouse = {
    context: () => call("context"),
    tables: () => call("schema.tables"),
    columns: (schema, table) => call("schema.columns", { schema, table }),
    query: (sql, opts) => call("query", { sql, limit: opts && opts.limit }),
    openQuery: (sql) => call("openQuery", { sql }),
    openTable: (schema, table) => call("openTable", { schema, table }),
    toast: (message, kind) => call("toast", { message, kind }),
    copy: (text) => call("clipboard.write", { text }),
    storage: {
      get: (key) => call("storage.get", { key }),
      set: (key, value) => call("storage.set", { key, value }),
    },
    on,
    onResult: (fn) => on("result", fn),
    onCommand: (fn) => on("command", fn),
    onConnection: (fn) => on("connection", fn),
  };
  call("ready").then(applyTheme);
})();
`;

const BASE_CSS = `
:root { color-scheme: light dark; }
html, body { margin: 0; background: var(--bg); color: var(--fg); font: 13px/1.45 var(--font-ui, system-ui, sans-serif); }
* { box-sizing: border-box; }
button { font: inherit; }
`;

export function themeSnapshot() {
  const cs = getComputedStyle(document.documentElement);
  const names = ["--bg", "--panel", "--elevated", "--fg", "--muted", "--subtle", "--line", "--line-strong", "--hover", "--selected", "--accent", "--accent-soft", "--accent-fg", "--danger", "--success", "--warning", "--font-ui", "--font-mono"];
  for (let i = 1; i <= 8; i++) names.push(`--series-${i}`);
  const vars: Record<string, string> = {};
  for (const n of names) {
    const v = cs.getPropertyValue(n).trim();
    if (v) vars[n] = v;
  }
  const dark = document.documentElement.dataset.theme === "dark" || (!document.documentElement.dataset.theme && matchMedia("(prefers-color-scheme: dark)").matches);
  return { mode: dark ? "dark" : "light", vars };
}

/** The page for the iframe: CSP + theme + SDK, then the extension's own HTML. */
export function buildSrcdoc(html: string, ext: ExtensionInfo): string {
  const hosts = ext.granted
    .filter((p) => p.startsWith("network:"))
    .map((p) => p.slice(8))
    .map((h) => (h.includes("://") ? h : `https://${h}`));
  const csp = [
    "default-src 'none'",
    "script-src 'unsafe-inline'",
    "style-src 'unsafe-inline'",
    "img-src data: blob:",
    "font-src data:",
    `connect-src ${hosts.length ? hosts.join(" ") : "'none'"}`,
  ].join("; ");
  const t = themeSnapshot();
  const vars = Object.entries(t.vars)
    .map(([k, v]) => `${k}:${v}`)
    .join(";");
  const head = `<meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="${csp}"><style>:root{${vars}}${BASE_CSS}</style><script>${SDK}</script>`;
  if (/<head[^>]*>/i.test(html)) return html.replace(/<head[^>]*>/i, (m) => `${m}${head}`);
  return `<!doctype html><html data-theme="${t.mode}"><head>${head}</head><body>${html}</body></html>`;
}

export const can = (ext: ExtensionInfo, perm: string) => ext.enabled && ext.granted.includes(perm);

export const PERMISSION_TEXT: Record<string, string> = {
  schema: "See the tables and columns of the open connection",
  "query:read": "Run read-only queries on the open connection",
  "query:write": "Run any query on the open connection, including changes",
  result: "Receive query results you send to it",
  clipboard: "Copy text to your clipboard",
};
export const permissionText = (p: string) => PERMISSION_TEXT[p] ?? (p.startsWith("network:") ? `Connect to ${p.slice(8)}` : p);
