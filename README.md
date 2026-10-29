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

Postgres integration test: `ROWHOUSE_TEST_PG=localhost:5432:postgres cargo test -p rowhouse-core --test postgres`

## Contributing

Commits follow [Conventional Commits](https://www.conventionalcommits.org), enforced by commitlint:

```
feat(grid): inline cell editing
fix(mcp): reject EXPLAIN ANALYZE on writes
docs: add SSH tunnel guide
```

Types: `feat`, `fix`, `docs`, `refactor`, `perf`, `test`, `build`, `ci`, `chore`, `style`, `revert`.
Suggested scopes: `core`, `mcp`, `desktop`, `ui`, `grid`, `editor`, `drivers`, `deps`, `ci`, `docs`.
The pre-commit hook runs `cargo fmt --check` and the TypeScript type check; CI runs tests on Linux and builds on macOS + Windows.

## Database support

| Status | Databases | Rust driver |
| --- | --- | --- |
| ✅ v0.1 | PostgreSQL, MySQL, MariaDB, SQLite | `sqlx` |
| 🟢 easy (Postgres/MySQL wire) | CockroachDB, Amazon Redshift, Greenplum, SingleStore, TiDB | `sqlx` |
| 🟡 planned | SQL Server, ClickHouse, DuckDB, LibSQL/Turso, Cloudflare D1, MongoDB, Redis, Cassandra/ScyllaDB, Elasticsearch | `tiberius`, `clickhouse`, `duckdb`, `libsql`, HTTP API, `mongodb`, `redis`, `scylla`, `reqwest` |
| 🟠 harder | BigQuery, Snowflake, DynamoDB, Oracle, Vertica | REST/SDK (`gcp-bigquery-client`, Snowflake SQL API, `aws-sdk-dynamodb`), Oracle needs Instant Client, Vertica via ODBC |

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
- [ ] SSH tunnel (russh) and TLS options
- [ ] Import / export (CSV, JSON, SQL dump)
- [ ] Query history & saved queries (local SQLite)
- [ ] SQL Server, MongoDB, Redis, DuckDB, ClickHouse drivers
- [ ] ERD view, table structure editor
- [ ] Bundle `rowhouse-mcp` as a Tauri sidecar; MCP write tools gated by in-app approval
- [ ] Auto-update, code signing (macOS notarization, Windows signing)

## License

MIT — see [LICENSE](LICENSE)
