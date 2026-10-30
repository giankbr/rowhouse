# Rowhouse extensions

An extension is a small web app that runs in a panel inside Rowhouse. It can read the schema, run queries on the open connection, receive query results, and show whatever UI you like — table stats, exporters, dashboards, internal tools for your team.

Extensions run in a sandboxed iframe with an opaque origin: no access to the app, its storage, the file system or Tauri. Everything goes through `window.rowhouse`, and the host checks every call against the permissions the user approved.

## Layout

```
my-extension/
  manifest.json
  index.html        ← the entry page
  main.js           ← local <script src> and <link rel="stylesheet"> files are inlined
  style.css
```

Install it with **Tools → Extensions… → Install from folder…** while developing (re-install to pick up changes), or zip the folder and rename it to `.rhext` to share it:

```sh
cd my-extension && zip -r ../my-extension.rhext .
```

## manifest.json

```json
{
  "id": "table-stats",
  "name": "Table stats",
  "version": "1.0.0",
  "description": "Row counts for every table in the open connection.",
  "author": "You",
  "entry": "index.html",
  "contributes": {
    "panels": [{ "id": "main", "title": "Table stats" }],
    "resultActions": [{ "id": "send", "title": "Send to Table stats" }]
  },
  "permissions": ["schema", "query:read"]
}
```

- `id` — lowercase letters, digits and dashes.
- `contributes.panels` — tabs the extension can open (Command palette and the Extensions window list them).
- `contributes.resultActions` — entries in the **Extensions** menu above query results; picking one opens the first panel and sends it a `result` event. Needs the `result` permission.

### Permissions

| Permission | Allows |
|---|---|
| `schema` | `rowhouse.tables()`, `rowhouse.columns()` |
| `query:read` | `rowhouse.query()` with read-only statements (enforced by the database guard) |
| `query:write` | `rowhouse.query()` with any statement |
| `result` | receiving results the user sends from a query tab |
| `clipboard` | `rowhouse.copy(text)` |
| `network:<host>` | `fetch()` to `https://<host>` (everything else is blocked by CSP) |

An update that asks for new permissions is disabled until the user approves it again.

## The `window.rowhouse` API

Every call returns a promise.

```js
const ctx = await rowhouse.context();
// { extension: { id, version, permissions }, connection: { id, name, kind, dialect, database, readOnly } | null, theme: "light" | "dark" }

const tables = await rowhouse.tables();                 // [{ schema, name, kind }]
const cols = await rowhouse.columns("public", "users"); // [{ name, data_type, nullable, default, primary_key }]

const out = await rowhouse.query("SELECT count(*) FROM users", { limit: 1000 });
// { results: [{ columns: [{ name, data_type }], rows: [[…]], rows_affected, truncated }], elapsed_ms }

await rowhouse.openQuery("SELECT * FROM users WHERE …"); // opens a query tab (not run)
await rowhouse.openTable("public", "users");
await rowhouse.toast("Done");                             // or toast(msg, "err")
await rowhouse.copy("text");                              // needs clipboard
await rowhouse.storage.set("prefs", { compact: true });   // per-extension, 1 MB
const prefs = await rowhouse.storage.get("prefs");

rowhouse.onResult(({ action, result, sql }) => { … });    // from a result action
rowhouse.onConnection((conn) => { … });                    // the user switched connections
rowhouse.on("theme", ({ mode, vars }) => { … });
```

### Styling

The app's design tokens are set as CSS variables on `:root`, and update when the theme changes: `--bg`, `--panel`, `--elevated`, `--fg`, `--muted`, `--subtle`, `--line`, `--hover`, `--accent`, `--accent-fg`, `--danger`, `--success`, `--warning`, `--font-ui`, `--font-mono` and `--series-1` … `--series-8` for charts. Use them and your extension matches light and dark mode for free.

## Examples

[`extensions/examples`](../extensions/examples) has two complete extensions:

- **table-stats** — a panel that counts rows in every table (`schema`, `query:read`).
- **result-to-markdown** — a result action that turns a grid into a Markdown table (`result`, `clipboard`).
