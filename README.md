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
- Query history (`⌘/Ctrl+Y`) shared with the MCP server — every query an AI assistant runs shows up, tagged “AI”
- Command palette (`⌘/Ctrl+K`), light/dark theme, compact 12–13px UI
- `rowhouse-mcp`: stdio MCP server sharing the same connection profiles

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
| ✅ integration-tested | PostgreSQL, MySQL / MariaDB, SQLite, DuckDB, ClickHouse | `sqlx`, `duckdb` (embedded), ClickHouse HTTP API |
| 🧪 tests wired into CI (verify on first run) | SQL Server, CockroachDB, MySQL 8 | `tiberius`, Postgres protocol |
| 🟡 beta (wire-compatible, not yet run against a live server) | Amazon Redshift, Greenplum, TiDB, SingleStore | Postgres / MySQL protocol |
| ⏳ planned | LibSQL/Turso, Cloudflare D1, MongoDB, Redis, Cassandra/ScyllaDB, Elasticsearch, BigQuery, Snowflake, Oracle | — |

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
| `run_query` | Read-only SQL (SELECT / WITH / SHOW / EXPLAIN) |

Safety: every MCP query passes a keyword guard **and** runs inside a read-only transaction
(`BEGIN READ ONLY` on Postgres, `START TRANSACTION READ ONLY` on MySQL, `PRAGMA query_only` on SQLite).

Claude Desktop (`claude_desktop_config.json`) / Cursor / any MCP client:

```json
{ "mcpServers": { "rowhouse": { "command": "/absolute/path/to/target/release/rowhouse-mcp" } } }
```

Profiles file location (override with `ROWHOUSE_CONFIG_DIR`):

- macOS: `~/Library/Application Support/id.morfotech.rowhouse/connections.json`
- Windows: `%APPDATA%\morfotech\rowhouse\config\connections.json`
- Linux: `~/.config/rowhouse/connections.json`

## Design

Tokens live in `apps/desktop/src/styles.css` (one set, light + dark): neutral surfaces,
1px borders instead of shadows, a single indigo accent, Geist + Geist Mono, 24–28px controls,
motion ≤160ms. Small primitives are in `src/components/ui.tsx`; components from shadcn-style
libraries (e.g. Pace UI) drop in since the stack is React + Tailwind.

## Roadmap

- [x] Inline cell editing with pending-changes buffer → SQL preview → commit in a transaction
- [ ] Filters / sort pushed down to SQL, pagination for large tables
- [x] SSH tunnel (russh): key / password auth, host-key pinning
- [x] SSH agent auth
- [ ] TLS options (CA / client certs)
- [x] Query history (app + MCP)
- [x] Export (CSV, JSON, SQL INSERTs) and CSV import
- [ ] Saved queries / snippets
- [x] SQL Server, DuckDB, ClickHouse, CockroachDB (+ beta: Redshift, Greenplum, TiDB, SingleStore)
- [ ] MongoDB, Redis, LibSQL/Turso, D1, Cassandra drivers
- [x] AI: natural-language → SQL, fix, explain with your own model
- [ ] AI: streaming responses, inline completions in the editor
- [x] Production safety rails, ERD, split panes, connection import
- [ ] Query plan viewer, schema/data compare, saved snippets
- [ ] ERD view, table structure editor
- [ ] Bundle `rowhouse-mcp` as a Tauri sidecar; MCP write tools gated by in-app approval
- [ ] Auto-update, code signing (macOS notarization, Windows signing)

## License

MIT — see [LICENSE](LICENSE)
