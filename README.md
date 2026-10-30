# Rowhouse

A fast, compact, open-source database client for macOS and Windows (Linux too),
built with Tauri 2 + Rust + React, with a built-in read-only **MCP server** so AI
assistants can explore your databases safely.

> Status: early scaffold (v0.1). PostgreSQL, MySQL/MariaDB and SQLite work end-to-end.

## Features (v0.1)

- Connection manager: passwords live in the OS keychain (Keychain / Credential Manager)
- Per-connection **read-only** mode and color tags (e.g. red = production)
- Schema tree with filter, table **Data** / **Structure** views
- SQL editor (CodeMirror 6): dialect-aware highlighting, table autocomplete, `⌘/Ctrl+↵` runs the selection or the whole script
- Multi-statement scripts → multiple result sets
- Virtualized result grid: sort, resize columns, keyboard navigation, `⌘/Ctrl+C` copies a cell
- Inline editing for tables with a primary key: edit / NULL / add / delete rows, review the generated SQL, commit atomically (`⌘/Ctrl+S`); every UPDATE/DELETE must hit exactly one row or the transaction rolls back
- SSH tunnels: private key (incl. encrypted), SSH agent (incl. 1Password / Pageant / Windows OpenSSH), or password; host key checked against `~/.ssh/known_hosts`, otherwise pinned on first connect
- Export query/table results to CSV, JSON or SQL INSERTs (streamed, no row limit); import CSV into a table in one transaction
- AI, bring your own model (opt-in): **Ask AI** (`⌘/Ctrl+I`) turns a sentence into SQL, **Fix with AI** on errors, **Explain** — proposals are shown as a diff and only applied when you accept. Works with OpenAI-compatible APIs (OpenAI, OpenRouter, Ollama, LM Studio, …) and Anthropic. Only the schema (names/types) and your SQL are sent — never rows
- Production safety: tag connections Local / Dev / Staging / Production — production starts read-only, gets a red window frame, and every DROP / TRUNCATE / DELETE or UPDATE without WHERE asks you to type the table name first
- ER diagram (`Diagram` in the sidebar): built from foreign keys, pan / zoom / drag, click to focus a table's relations, double-click to open it, export SVG
- Split panes (`⌘/Ctrl+\\`) and drag-and-drop tabs between them
- Import connections from DBeaver, TablePlus and Navicat (auto-detects their files; passwords are re-entered once)
- Start screen with every connection (search, ↑/↓, Enter), environment badges, recent first; open SQLite/DuckDB files directly
- Table structure editor: add / rename / change / drop columns, DDL previewed before it runs
- Paste a connection URL (`postgresql://…`, `mysql://…`); Neon and Supabase are recognised and use TLS by default
- TLS options: disable / prefer / require / verify-ca / verify-full, custom CA and client certificates (mutual TLS)
- Table browsing pushed down to SQL: filters, server-side sort and pagination with row counts
- Query plan viewer (**Plan**): EXPLAIN as a tree with cost/time bars and hints (full scans, bad estimates, disk sorts); `ANALYZE` on PostgreSQL runs read-only
- Saved queries: **Save** in a query tab, find them under *Saved* in the history drawer (per connection or global)
- Schema compare between any two connections/schemas, with a migration script for the target (drops commented out)
- Query history (`⌘/Ctrl+Y`) shared with the MCP server — every query an AI assistant runs shows up, tagged “AI”
- Command palette (`⌘/Ctrl+K`), light/dark theme, compact 12–13px UI
- `rowhouse-mcp`: stdio MCP server bundled with the app, sharing the same connection profiles. Reads are read-only; on connections where you allow it, the AI can *request* writes (`execute_sql`) and each one waits for your approval in the app (destructive ones need the table name typed)

## Layout

```
crates/rowhouse-core   Rust engine: drivers (sqlx), connection store, read-only guard
crates/rowhouse-mcp    MCP server binary (rmcp, stdio)
apps/desktop         Tauri 2 app — React + Tailwind v4 UI in src/, Rust shell in src-tauri/
```

All query execution goes through `sqlx::raw_sql` (simple/text protocol), so any column
type can be displayed and multi-statement scripts work. Values become JSON
(numbers / booleans / JSON stay typed, everything else is a string; NUMERIC stays a
string to keep precision).

## Develop

Prerequisites: Rust (stable), Node 20+, pnpm 10, plus the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/)
(Xcode CLT on macOS; WebView2 + MSVC build tools on Windows).

```bash
pnpm install          # also installs git hooks (husky)
pnpm dev              # desktop app with hot reload
pnpm dev:ui           # UI only in the browser, backed by a mock API (fast design loop)
pnpm test             # cargo test --workspace
pnpm build            # release MCP binary + installers (.dmg/.app, .msi/.exe)
```

Integration tests (skipped unless configured):

```bash
ROWHOUSE_TEST_PG=localhost:5432:postgres cargo test -p rowhouse-core --test postgres
ROWHOUSE_TEST_PG=localhost:5432:postgres ROWHOUSE_TEST_SSH=127.0.0.1:22:me:~/.ssh/id_ed25519 cargo test -p rowhouse-core --test ssh
ROWHOUSE_TEST_MYSQL=127.0.0.1:3306:root:root:rowhouse cargo test -p rowhouse-core --test wire_compat
ROWHOUSE_TEST_MSSQL='localhost:1433:sa:Pass!word1' cargo test -p rowhouse-core --test mssql
ROWHOUSE_TEST_CH=localhost:8123:default: cargo test -p rowhouse-core --test clickhouse
```

The first build compiles DuckDB from source and takes a while; later builds are cached.

## Contributing

Commits follow [Conventional Commits](https://www.conventionalcommits.org), enforced by commitlint:

```
feat(grid): inline cell editing
fix(mcp): reject EXPLAIN ANALYZE on writes
docs: add SSH tunnel guide
```

Types: `feat`, `fix`, `docs`, `refactor`, `perf`, `test`, `build`, `ci`, `chore`, `style`, `revert`.
Suggested scopes: `core`, `mcp`, `ai`, `desktop`, `ui`, `grid`, `editor`, `drivers`, `deps`, `ci`, `docs`.
The pre-commit hook runs `cargo fmt --check` and the TypeScript type check; CI runs tests on Linux and builds on macOS + Windows.

## Database support

| Status | Databases | How |
| --- | --- | --- |
| ✅ integration-tested | PostgreSQL (incl. TLS / mTLS), Redis (incl. TLS), MySQL / MariaDB, SQLite, DuckDB, ClickHouse | `sqlx`, `duckdb` (embedded), ClickHouse HTTP API |
| ✅ tested against a protocol-level fake backed by real SQLite | Turso / libSQL (Hrana HTTP), Cloudflare D1 (REST API) | `reqwest` |
| 🧪 tests wired into CI (verify on first run) | SQL Server, CockroachDB, MySQL 8, MongoDB (beta), Cassandra / ScyllaDB (beta) | `tiberius`, Postgres protocol, official `mongodb` driver, `scylla` (CQL) |
| 🟡 beta (wire-compatible, not yet run against a live server) | Neon, Supabase (Postgres + TLS), Amazon Redshift, Greenplum, TiDB, SingleStore | Postgres / MySQL protocol |
| ⏳ planned | Elasticsearch, BigQuery, Snowflake, Oracle | — |

MongoDB: collections are tables; the editor takes mongosh-style statements (`db.orders.find({ status: 'paid' }).sort({ total: -1 }).limit(10)`, `aggregate([...])`, `insertOne`, `updateMany`, `show collections`, …). Grid edits are keyed by `_id` and keep each field's BSON type; they're checked up front but not transactional (MongoDB needs a replica set for that). Paste a `mongodb://` / `mongodb+srv://` URI to connect to Atlas.

Turso and D1 use the SQLite dialect over HTTPS. Grid edits run as one batch with a row-count guard after each keyed statement, so a stale row rolls the whole batch back (D1 runs a multi-statement request as one transaction). D1 needs a Cloudflare API token with D1 permissions; there is no read-only transaction over its API, so read-only D1 connections rely on the keyword guard.

Cassandra / ScyllaDB speak CQL. Keyspaces show up as schemas. Grid paging skips rows client-side because CQL has no OFFSET. Filters on non-key columns add `ALLOW FILTERING`, and sorting applies to the fetched page. Grid edits go through `fromJson()`, so uuid, timestamp and collection columns just work. Keyed UPDATE/DELETE use `IF EXISTS`, so a stale row is reported instead of upserted. There are no transactions, so earlier statements in a change set stay applied.

Notes: ClickHouse has no transactions, so grid editing is disabled there (CSV import still works).
SQL Server connections encrypt and accept self-signed certificates, like most DB GUIs.

## MCP

`rowhouse-mcp` reads the same `connections.json` as the app and only exposes profiles with
**“Expose to AI assistants via MCP”** enabled.

| Tool | What it does |
| --- | --- |
| `list_connections` | Saved connections visible to MCP |
| `list_tables` | Tables & views |
| `describe_table` | Columns, types, nullability, defaults, PK |
| `sample_rows` | First N rows of a table |
| `run_query` | Read-only SQL (SELECT / WITH / SHOW / EXPLAIN); read commands for Redis, find/aggregate for MongoDB |
| `execute_sql` | A write, only after you approve it in Rowhouse (connections with “Allow AI to request writes”) |

Safety: every MCP query passes a keyword guard **and** runs inside a read-only transaction
(`BEGIN READ ONLY` on Postgres, `START TRANSACTION READ ONLY` on MySQL, `PRAGMA query_only` on SQLite).

Claude Desktop (`claude_desktop_config.json`) / Cursor / any MCP client:

```json
{ "mcpServers": { "rowhouse": { "command": "/absolute/path/to/target/release/rowhouse-mcp" } } }
```

Profiles file location (override with `ROWHOUSE_CONFIG_DIR`):

- macOS: `~/Library/Application Support/id.novagen.rowhouse/connections.json`
- Windows: `%APPDATA%\novagen\rowhouse\config\connections.json`
- Linux: `~/.config/rowhouse/connections.json`

## Design

Tokens live in `apps/desktop/src/styles.css` (one set, light + dark): neutral surfaces,
1px borders instead of shadows, a single indigo accent, Inter + JetBrains Mono by default (Geist / system fonts and a compact or comfortable density in Settings → Appearance), 22–26px controls,
motion ≤160ms. Small primitives are in `src/components/ui.tsx`; components from shadcn-style
libraries (e.g. Pace UI) drop in since the stack is React + Tailwind.

## Roadmap

- [x] Inline cell editing with pending-changes buffer → SQL preview → commit in a transaction
- [x] Filters / sort pushed down to SQL, pagination for large tables
- [x] SSH tunnel (russh): key / password auth, host-key pinning
- [x] SSH agent auth
- [x] TLS options (CA / client certs), Neon & Supabase
- [x] Query history (app + MCP)
- [x] Export (CSV, JSON, SQL INSERTs) and CSV import
- [x] Saved queries / snippets
- [x] SQL Server, DuckDB, ClickHouse, CockroachDB (+ beta: Redshift, Greenplum, TiDB, SingleStore)
- [x] Redis
- [x] MongoDB (beta)
- [x] LibSQL/Turso, Cloudflare D1, Redis TLS / Upstash
- [x] Cassandra / ScyllaDB (beta)
- [x] AI: natural-language → SQL, fix, explain with your own model
- [x] AI: streaming responses, inline completions in the editor
- [x] AI inline edit (Mod+I in the editor: rewrite the selection or generate at the cursor)
- [x] Grid range selection + bulk ops (set value / NULL, duplicate, delete, paste from spreadsheets, copy as CSV/JSON/Markdown/INSERT)
- [x] Production safety rails, ERD, split panes, connection import
- [x] Visual query builder (tables on a canvas, FK auto-joins, drag-to-join, filters, aggregates, live SQL)
- [x] Query plan viewer, schema compare, saved queries
- [x] Data compare (PK-matched rows + sync script)
- [x] Cancel running queries (server-side on Postgres/MySQL), query timeout, tabs restored after restart
- [x] Row details panel with JSON editor, find in results, SQL formatter, database switcher
- [x] Backup / restore (pg_dump, pg_restore, psql, mysqldump, mysql, SQLite VACUUM INTO), server activity with cancel/kill
- [x] Index and foreign key editing, auto-reconnect after sleep
- [x] Scheduled queries (interval / daily / weekly, export to CSV/JSON, notify on failure or change)
- [x] ERD view, table structure editor
- [x] Bundle `rowhouse-mcp` as a Tauri sidecar; MCP write tools gated by in-app approval
- [x] Auto-update and code-signing pipeline (see [RELEASING.md](RELEASING.md); needs your keys/certificates)

## License

MIT — see [LICENSE](LICENSE)
