//! rowhouse-mcp — stdio MCP server over the connections saved in Rowhouse.
//!
//! Safety model: reads go through `Connection::query_read_only` (keyword guard
//! + read-only transaction), and only profiles with `mcpEnabled: true` are
//! visible. Writes (`execute_sql`) are only offered for profiles with
//! `mcpWrites: true`, and every single one waits for the user to approve it in
//! the Rowhouse app (see `rowhouse_core::approvals`).
//!
//! Register it in an MCP client, e.g. Claude Desktop `claude_desktop_config.json`:
//! ```json
//! { "mcpServers": { "rowhouse": { "command": "/path/to/rowhouse-mcp" } } }
//! ```

use std::{collections::HashMap, sync::Arc};

use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{Implementation, ServerCapabilities, ServerConfig},
    schemars, tool, tool_handler, tool_router, ServerHandler, ServiceExt,
};
use rowhouse_core::{
    approvals::Approvals, history::NewEntry, Connection, ConnectionConfig, ConnectionStore, History,
};
use serde::Deserialize;
use serde_json::json;
use tokio::sync::Mutex;

const DEFAULT_LIMIT: usize = 100;
const MAX_LIMIT: usize = 1_000;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ConnArg {
    /// Connection name or id, as returned by `list_connections`.
    connection: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct TableArg {
    /// Connection name or id.
    connection: String,
    /// Table name.
    table: String,
    /// Schema (Postgres: e.g. "public"; MySQL: database name). Optional for SQLite.
    schema: Option<String>,
    /// Max rows for `sample_rows` (default 20).
    limit: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct QueryArg {
    /// Connection name or id.
    connection: String,
    /// Read-only SQL (SELECT / WITH / SHOW / EXPLAIN). Multiple statements allowed.
    sql: String,
    /// Max rows per statement (default 100, max 1000).
    limit: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct WriteArg {
    /// Connection name or id. The connection must allow AI writes.
    connection: String,
    /// SQL to execute: INSERT / UPDATE / DELETE / DDL. Shown to the user verbatim for approval.
    sql: String,
    /// One sentence for the user: why this change is needed.
    reason: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SearchArg {
    /// Connection name or id.
    connection: String,
    /// Words to look for in table and column names, e.g. "customer email" or "invoice".
    query: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ExplainArg {
    /// Connection name or id.
    connection: String,
    /// One statement to explain.
    sql: String,
    /// Run it to get real timings (only allowed for read-only statements). Default false.
    analyze: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ObjectArg {
    /// Connection name or id.
    connection: String,
    /// Object name as returned by `list_objects`.
    name: String,
    /// function | procedure | trigger | sequence | type | matview | macro
    kind: String,
    /// Schema of the object.
    schema: Option<String>,
    /// For functions: the argument list from `list_objects` (picks the right overload).
    detail: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DashArg {
    /// Dashboard name or id, as returned by `list_dashboards`.
    dashboard: String,
    /// Filter values for this run, overriding the saved ones, e.g. {"from": "2026-10-01"}.
    filters: Option<std::collections::BTreeMap<String, String>>,
    /// Max rows per tile (default 50).
    limit: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SavedArg {
    /// Connection name or id. For `list_saved_queries` it filters; global queries are always included.
    connection: Option<String>,
    /// Saved query name (for `run_saved_query`).
    name: Option<String>,
    /// Max rows (default 100).
    limit: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct HistoryArg {
    /// Connection name or id.
    connection: String,
    /// Only queries containing this text.
    search: Option<String>,
    /// How many (default 20, max 100).
    limit: Option<i64>,
}

/// How long a write waits for the user to answer.
const APPROVAL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

#[derive(Clone)]
struct Rowhouse {
    store: Arc<ConnectionStore>,
    history: Option<History>,
    pool: Arc<Mutex<HashMap<String, Arc<Connection>>>>,
    tool_router: ToolRouter<Self>,
}

impl Rowhouse {
    fn new(store: ConnectionStore, history: Option<History>) -> Self {
        Self {
            store: Arc::new(store),
            history,
            pool: Default::default(),
            tool_router: Self::tool_router(),
        }
    }

    fn profile(&self, name: &str) -> Result<ConnectionConfig, String> {
        let cfg = self.store.get(name).map_err(|e| e.to_string())?;
        if !cfg.mcp_enabled {
            return Err(format!("connection `{}` is not enabled for MCP", cfg.name));
        }
        Ok(cfg)
    }

    async fn conn(&self, name: &str) -> Result<Arc<Connection>, String> {
        let cfg = self.profile(name)?;
        let mut pool = self.pool.lock().await;
        if let Some(c) = pool.get(&cfg.id) {
            return Ok(c.clone());
        }
        let c = Arc::new(Connection::connect(&cfg).await.map_err(|e| e.to_string())?);
        pool.insert(cfg.id.clone(), c.clone());
        Ok(c)
    }

    /// Run read-only and log to the shared query history (source = "mcp").
    async fn run_logged(&self, c: &Connection, sql: &str, limit: usize) -> Result<String, String> {
        let started = std::time::Instant::now();
        let res = c.query_read_only(sql, limit).await;
        if let Some(h) = &self.history {
            let err = res.as_ref().err().map(|e| e.to_string());
            let rows = res
                .as_ref()
                .map(|o| o.results.iter().map(|r| r.rows.len() as i64).sum())
                .unwrap_or(0);
            let _ = h
                .record(NewEntry {
                    connection_id: &c.config.id,
                    sql,
                    source: "mcp",
                    error: err.as_deref(),
                    rows,
                    elapsed_ms: started.elapsed().as_millis() as i64,
                })
                .await;
        }
        let mut out = res.map_err(|e| e.to_string())?;
        if c.config.masks() {
            // personal data and secrets stay on this machine
            for r in &mut out.results {
                rowhouse_core::mask::mask_result(r);
            }
        }
        Ok(serde_json::to_string(&out).unwrap_or_default())
    }

    async fn log(&self, c: &Connection, sql: &str, err: Option<&str>, rows: i64, ms: i64) {
        if let Some(h) = &self.history {
            let _ = h
                .record(NewEntry {
                    connection_id: &c.config.id,
                    sql,
                    source: "mcp",
                    error: err,
                    rows,
                    elapsed_ms: ms,
                })
                .await;
        }
    }

    async fn default_schema(
        &self,
        c: &Connection,
        schema: Option<String>,
        table: &str,
    ) -> Result<String, String> {
        if let Some(s) = schema {
            return Ok(s);
        }
        let tables = c.list_tables().await.map_err(|e| e.to_string())?;
        tables
            .into_iter()
            .find(|t| t.name == table)
            .map(|t| t.schema)
            .ok_or_else(|| format!("table `{table}` not found"))
    }
}

fn pretty(v: impl serde::Serialize) -> String {
    serde_json::to_string_pretty(&v).unwrap_or_default()
}

#[tool_router]
impl Rowhouse {
    #[tool(description = "List database connections saved in Rowhouse that are available to MCP.")]
    async fn list_connections(&self) -> Result<String, String> {
        let list = self.store.list().map_err(|e| e.to_string())?;
        let out: Vec<_> = list
            .into_iter()
            .filter(|c| c.mcp_enabled)
            .map(|c| {
                json!({
                    "masked": if c.masks() { "yes: emails, phones, card numbers, IDs and secrets come back masked" } else { "no" },
                    "name": c.name, "id": c.id, "kind": c.kind, "database": c.database.or(c.file),
                    "environment": c.env,
                    "writes": if c.mcp_writes && !c.read_only { "with user approval (execute_sql)" } else { "no" },
                })
            })
            .collect();
        Ok(serde_json::to_string(&out).unwrap_or_default())
    }

    #[tool(description = "List tables and views in a connection.")]
    async fn list_tables(&self, Parameters(a): Parameters<ConnArg>) -> Result<String, String> {
        let c = self.conn(&a.connection).await?;
        Ok(pretty(c.list_tables().await.map_err(|e| e.to_string())?))
    }

    #[tool(
        description = "Describe the columns of a table: name, type, nullable, default, primary key."
    )]
    async fn describe_table(&self, Parameters(a): Parameters<TableArg>) -> Result<String, String> {
        let c = self.conn(&a.connection).await?;
        let schema = self.default_schema(&c, a.schema, &a.table).await?;
        Ok(pretty(
            c.describe_table(&schema, &a.table)
                .await
                .map_err(|e| e.to_string())?,
        ))
    }

    #[tool(description = "Return the first rows of a table (default 20).")]
    async fn sample_rows(&self, Parameters(a): Parameters<TableArg>) -> Result<String, String> {
        let c = self.conn(&a.connection).await?;
        let schema = self.default_schema(&c, a.schema, &a.table).await?;
        let limit = a.limit.unwrap_or(20).min(MAX_LIMIT);
        if c.config.kind.driver() == rowhouse_core::config::Driver::Redis {
            // a "table" is a key: show its value page
            let page = c
                .table_page(&rowhouse_core::browse::PageRequest {
                    schema,
                    table: a.table,
                    filters: vec![],
                    sort: None,
                    limit,
                    offset: 0,
                    count: true,
                })
                .await
                .map_err(|e| e.to_string())?;
            return Ok(serde_json::to_string(&page).unwrap_or_default());
        }
        let sql = c.select_table_sql(&schema, &a.table, limit);
        self.run_logged(&c, &sql, limit).await
    }

    #[tool(
        description = "Run a write (INSERT/UPDATE/DELETE/DDL) on a connection that allows AI writes. \
                       The user sees the exact SQL in the Rowhouse app and must approve it; this call waits \
                       for that answer (up to 5 minutes). Prefer the smallest, most targeted statement."
    )]
    async fn execute_sql(&self, Parameters(a): Parameters<WriteArg>) -> Result<String, String> {
        let cfg = self.profile(&a.connection)?;
        if cfg.read_only {
            return Err(format!(
                "`{}` is read-only in Rowhouse; writes are not possible",
                cfg.name
            ));
        }
        if let Err(e) = rowhouse_core::license::require_pro("AI write requests (execute_sql)") {
            return Err(e.to_string());
        }
        if !cfg.mcp_writes {
            return Err(format!(
                "AI writes are turned off for `{}`. The user can enable “Allow AI to request writes” in its connection settings.",
                cfg.name
            ));
        }
        let approvals = Approvals::open_default().map_err(|e| e.to_string())?;
        if !approvals.app_alive() {
            return Err("The Rowhouse app isn't running, so nobody can approve this write. Ask the user to open Rowhouse.".into());
        }
        let shown = match a.reason.as_deref().filter(|r| !r.trim().is_empty()) {
            Some(r) => format!("-- {}\n{}", r.trim().replace('\n', " "), a.sql),
            None => a.sql.clone(),
        };
        let req = approvals
            .submit(&cfg.id, &cfg.name, &shown, "AI assistant")
            .map_err(|e| e.to_string())?;
        let answer = approvals
            .wait(&req.id, APPROVAL_TIMEOUT)
            .await
            .map_err(|e| e.to_string())?;
        if !answer.approved {
            return Err(format!(
                "The user declined this write{}.",
                answer.reason.map(|r| format!(": {r}")).unwrap_or_default()
            ));
        }
        let c = self.conn(&a.connection).await?;
        let started = std::time::Instant::now();
        let res = c.query(&a.sql, DEFAULT_LIMIT).await;
        let ms = started.elapsed().as_millis() as i64;
        match res {
            Ok(out) => {
                let rows = out
                    .results
                    .iter()
                    .map(|r| r.rows_affected.max(r.rows.len() as u64) as i64)
                    .sum();
                self.log(&c, &a.sql, None, rows, ms).await;
                Ok(
                    serde_json::to_string(&json!({ "approved": true, "result": out }))
                        .unwrap_or_default(),
                )
            }
            Err(e) => {
                let msg = e.to_string();
                self.log(&c, &a.sql, Some(&msg), 0, ms).await;
                Err(msg)
            }
        }
    }

    #[tool(
        description = "Find tables and columns whose names match the words in `query` (case-insensitive). \
                       Use this first on large databases instead of listing every table."
    )]
    async fn search_schema(&self, Parameters(a): Parameters<SearchArg>) -> Result<String, String> {
        let c = self.conn(&a.connection).await?;
        let words: Vec<String> = a
            .query
            .to_lowercase()
            .split_whitespace()
            .map(String::from)
            .collect();
        if words.is_empty() {
            return Err("give at least one word to search for".into());
        }
        let erd = c.erd(None).await.map_err(|e| e.to_string())?;
        let mut hits = vec![];
        for t in &erd.tables {
            let tname = t.name.to_lowercase();
            let cols: Vec<_> = t
                .columns
                .iter()
                .filter(|col| words.iter().any(|w| col.name.to_lowercase().contains(w)))
                .map(|col| json!({ "column": col.name, "type": col.data_type }))
                .collect();
            let table_hit = words.iter().any(|w| tname.contains(w));
            if table_hit || !cols.is_empty() {
                hits.push(json!({
                    "schema": t.schema, "table": t.name, "kind": t.kind,
                    "matchedTable": table_hit, "matchedColumns": cols,
                    "columns": t.columns.len(),
                }));
            }
        }
        hits.sort_by_key(|h| {
            std::cmp::Reverse(
                h["matchedTable"].as_bool().unwrap_or(false) as u8 * 100
                    + h["matchedColumns"].as_array().map(|a| a.len()).unwrap_or(0) as u8,
            )
        });
        hits.truncate(50);
        Ok(pretty(
            json!({ "matches": hits, "truncated": erd.truncated }),
        ))
    }

    #[tool(
        description = "Foreign keys of a table: what it references (outgoing) and which tables reference it (incoming). Use it to write correct JOINs."
    )]
    async fn get_relations(&self, Parameters(a): Parameters<TableArg>) -> Result<String, String> {
        let c = self.conn(&a.connection).await?;
        let schema = self.default_schema(&c, a.schema, &a.table).await?;
        let fks = c.foreign_keys().await.map_err(|e| e.to_string())?;
        let outgoing: Vec<_> = fks
            .iter()
            .filter(|f| f.schema == schema && f.table == a.table)
            .collect();
        let incoming: Vec<_> = fks
            .iter()
            .filter(|f| f.ref_schema == schema && f.ref_table == a.table)
            .collect();
        Ok(pretty(
            json!({ "outgoing": outgoing, "incoming": incoming }),
        ))
    }

    #[tool(
        description = "The CREATE statement (DDL) of a table or view, including indexes and constraints."
    )]
    async fn get_ddl(&self, Parameters(a): Parameters<TableArg>) -> Result<String, String> {
        let c = self.conn(&a.connection).await?;
        let schema = self.default_schema(&c, a.schema, &a.table).await?;
        c.table_ddl(&schema, &a.table)
            .await
            .map_err(|e| e.to_string())
    }

    #[tool(
        description = "List functions, procedures, triggers, sequences, enum types and materialized views."
    )]
    async fn list_objects(&self, Parameters(a): Parameters<ConnArg>) -> Result<String, String> {
        let c = self.conn(&a.connection).await?;
        Ok(pretty(c.list_objects().await.map_err(|e| e.to_string())?))
    }

    #[tool(
        description = "The definition (CREATE statement) of a function, trigger, sequence, type or materialized view from `list_objects`."
    )]
    async fn get_object_definition(
        &self,
        Parameters(a): Parameters<ObjectArg>,
    ) -> Result<String, String> {
        let c = self.conn(&a.connection).await?;
        let o = rowhouse_core::objects::DbObject {
            schema: a.schema.unwrap_or_default(),
            name: a.name,
            kind: a.kind,
            detail: a.detail,
        };
        c.object_definition(&o).await.map_err(|e| e.to_string())
    }

    #[tool(
        description = "Show the query plan (EXPLAIN) for one statement, as a tree with costs. `analyze: true` runs it for real timings and is only allowed for reads."
    )]
    async fn explain_query(&self, Parameters(a): Parameters<ExplainArg>) -> Result<String, String> {
        let c = self.conn(&a.connection).await?;
        Ok(pretty(
            c.explain_plan(&a.sql, a.analyze.unwrap_or(false))
                .await
                .map_err(|e| e.to_string())?,
        ))
    }

    #[tool(
        description = "Approximate row counts per table (from the database's statistics, so it's cheap even on big tables)."
    )]
    async fn table_stats(&self, Parameters(a): Parameters<ConnArg>) -> Result<String, String> {
        use rowhouse_core::config::Driver;
        let c = self.conn(&a.connection).await?;
        let sql = match c.driver() {
            Driver::Postgres => "SELECT n.nspname AS schema, c.relname AS table, c.reltuples::bigint AS approx_rows, pg_total_relation_size(c.oid) AS bytes FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE c.relkind IN ('r','p') AND n.nspname NOT IN ('pg_catalog','information_schema') AND n.nspname NOT LIKE 'pg_toast%' ORDER BY c.reltuples DESC LIMIT 200",
            Driver::Mysql => "SELECT table_schema AS `schema`, table_name AS `table`, table_rows AS approx_rows, data_length + index_length AS bytes FROM information_schema.tables WHERE table_type = 'BASE TABLE' AND table_schema NOT IN ('mysql','information_schema','performance_schema','sys') ORDER BY table_rows DESC LIMIT 200",
            Driver::Mssql => "SELECT s.name AS [schema], t.name AS [table], SUM(p.rows) AS approx_rows FROM sys.tables t JOIN sys.schemas s ON s.schema_id = t.schema_id JOIN sys.partitions p ON p.object_id = t.object_id AND p.index_id IN (0,1) GROUP BY s.name, t.name ORDER BY 3 DESC",
            Driver::Clickhouse => "SELECT database AS schema, name AS table, total_rows AS approx_rows, total_bytes AS bytes FROM system.tables WHERE database = currentDatabase() ORDER BY total_rows DESC",
            _ => return Err("table_stats isn't available for this database — use run_query with COUNT(*)".into()),
        };
        self.run_logged(&c, sql, 200).await
    }

    #[tool(
        description = "Queries the user saved in Rowhouse (name + SQL). Good starting points: they reflect how the user actually queries this data."
    )]
    async fn list_saved_queries(
        &self,
        Parameters(a): Parameters<SavedArg>,
    ) -> Result<String, String> {
        let h = self
            .history
            .as_ref()
            .ok_or("query history is unavailable")?;
        let id = match &a.connection {
            Some(n) => Some(self.profile(n)?.id),
            None => None,
        };
        let list = h
            .list_snippets(id.as_deref(), None)
            .await
            .map_err(|e| e.to_string())?;
        Ok(pretty(list.into_iter().map(|s| json!({ "name": s.name, "sql": s.sql, "global": s.connection_id.is_none() })).collect::<Vec<_>>()))
    }

    #[tool(
        description = "The user's Rowhouse dashboards (on MCP-enabled connections): tiles with their titles and views, the filters they take, and any snapshot schedule."
    )]
    async fn list_dashboards(&self) -> Result<String, String> {
        let list = rowhouse_core::dashboards::DashboardStore::default()
            .list()
            .map_err(|e| e.to_string())?;
        let out: Vec<_> = list
            .into_iter()
            .filter_map(|d| {
                let cfg = self.profile(&d.connection_id).ok()?;
                let colon = !matches!(
                    cfg.kind.driver(),
                    rowhouse_core::config::Driver::Redis | rowhouse_core::config::Driver::Mongo
                );
                let mut vars: Vec<String> = Vec::new();
                for t in &d.tiles {
                    for v in rowhouse_core::vars::find(&t.sql, colon) {
                        if !vars.contains(&v) {
                            vars.push(v);
                        }
                    }
                }
                Some(json!({
                    "id": d.id,
                    "name": d.name,
                    "connection": cfg.name,
                    "tiles": d.tiles.iter().map(|t| json!({ "title": t.title, "view": t.view, "followsSavedQuery": t.snippet_id.is_some() })).collect::<Vec<_>>(),
                    "filters": vars.iter().map(|v| json!({ "name": v, "value": d.filters.get(v) })).collect::<Vec<_>>(),
                    "snapshot": d.snapshot.as_ref().filter(|s| s.enabled).map(|s| rowhouse_core::schedules::describe(&s.every)),
                }))
            })
            .collect();
        Ok(pretty(out))
    }

    #[tool(
        description = "Run every tile of a dashboard (read-only) and return each tile's rows. Pass `filters` to try other values (e.g. a date range) without changing the saved dashboard."
    )]
    async fn run_dashboard(&self, Parameters(a): Parameters<DashArg>) -> Result<String, String> {
        let list = rowhouse_core::dashboards::DashboardStore::default()
            .list()
            .map_err(|e| e.to_string())?;
        let d = list
            .into_iter()
            .find(|d| d.id == a.dashboard || d.name.eq_ignore_ascii_case(&a.dashboard))
            .ok_or_else(|| format!("no dashboard named `{}`", a.dashboard))?;
        let c = self.conn(&d.connection_id).await?;
        let snippets = match &self.history {
            Some(h) => h
                .list_snippets(Some(&c.config.id), None)
                .await
                .unwrap_or_default(),
            None => vec![],
        };
        let mut filters = d.filters.clone();
        filters.extend(a.filters.unwrap_or_default());
        let colon = !matches!(
            c.driver(),
            rowhouse_core::config::Driver::Redis | rowhouse_core::config::Driver::Mongo
        );
        let limit = a.limit.unwrap_or(50).clamp(1, MAX_LIMIT);
        let mut tiles = Vec::new();
        for t in &d.tiles {
            let raw = t
                .snippet_id
                .and_then(|id| snippets.iter().find(|s| s.id == id))
                .map(|s| s.sql.clone())
                .unwrap_or_else(|| t.sql.clone());
            let sql = rowhouse_core::vars::substitute(&raw, &filters, colon, |v| c.literal(v));
            let started = std::time::Instant::now();
            let res = c.query_read_only(&sql, limit).await;
            let err = res.as_ref().err().map(|e| e.to_string());
            let mut result = res.ok().and_then(|o| {
                let mut r = o.results;
                let i = r.iter().rposition(|x| !x.columns.is_empty()).unwrap_or(0);
                (!r.is_empty()).then(|| r.swap_remove(i))
            });
            if c.config.masks() {
                if let Some(r) = result.as_mut() {
                    rowhouse_core::mask::mask_result(r);
                }
            }
            self.log(
                &c,
                &sql,
                err.as_deref(),
                result.as_ref().map(|r| r.rows.len() as i64).unwrap_or(0),
                started.elapsed().as_millis() as i64,
            )
            .await;
            tiles.push(json!({
                "title": t.title,
                "view": t.view,
                "error": err,
                "columns": result.as_ref().map(|r| r.columns.iter().map(|c| c.name.clone()).collect::<Vec<_>>()),
                "rows": result.as_ref().map(|r| &r.rows),
                "truncated": result.as_ref().map(|r| r.truncated),
            }));
        }
        Ok(pretty(
            json!({ "dashboard": d.name, "connection": c.config.name, "filters": filters, "tiles": tiles }),
        ))
    }

    #[tool(description = "Run one of the user's saved queries by name (read-only).")]
    async fn run_saved_query(&self, Parameters(a): Parameters<SavedArg>) -> Result<String, String> {
        let h = self
            .history
            .as_ref()
            .ok_or("query history is unavailable")?;
        let conn = a.connection.ok_or("`connection` is required")?;
        let name = a.name.ok_or("`name` is required")?;
        let c = self.conn(&conn).await?;
        let list = h
            .list_snippets(Some(&c.config.id), None)
            .await
            .map_err(|e| e.to_string())?;
        let s = list
            .into_iter()
            .find(|s| s.name.eq_ignore_ascii_case(&name))
            .ok_or_else(|| format!("no saved query named `{name}`"))?;
        self.run_logged(&c, &s.sql, a.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT))
            .await
    }

    #[tool(
        description = "Recent queries run on a connection (from the app and from MCP), newest first, with errors and row counts."
    )]
    async fn recent_queries(
        &self,
        Parameters(a): Parameters<HistoryArg>,
    ) -> Result<String, String> {
        let h = self
            .history
            .as_ref()
            .ok_or("query history is unavailable")?;
        let cfg = self.profile(&a.connection)?;
        let list = h
            .list(
                Some(&cfg.id),
                a.search.as_deref(),
                a.limit.unwrap_or(20).clamp(1, 100),
            )
            .await
            .map_err(|e| e.to_string())?;
        Ok(pretty(
            list.into_iter()
                .map(|e| json!({ "sql": e.sql, "source": e.source, "ok": e.ok, "error": e.error, "rows": e.rows, "ms": e.elapsed_ms, "at": e.at }))
                .collect::<Vec<_>>(),
        ))
    }

    #[tool(
        description = "Run a READ-ONLY query: SQL for SQL databases, Redis read commands for Redis, mongosh find/aggregate/countDocuments for MongoDB (e.g. db.orders.find({ status: \"paid\" }).limit(10)). Writes are rejected."
    )]
    async fn run_query(&self, Parameters(a): Parameters<QueryArg>) -> Result<String, String> {
        let c = self.conn(&a.connection).await?;
        let limit = a.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT);
        self.run_logged(&c, &a.sql, limit).await
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Rowhouse {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("rowhouse", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "Rowhouse gives access to the user's saved database connections. Reads are read-only; \
                 writes go through execute_sql and need the user's approval in the app. \
                 Start with list_connections, then search_schema (or list_tables) and describe_table / get_relations \
                 before writing SQL; list_saved_queries shows how the user usually queries the data.",
            )
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Logs must go to stderr: stdout is the MCP channel.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let store = ConnectionStore::open_default();
    tracing::info!("rowhouse-mcp using {}", store.path().display());
    let history = History::open_default()
        .await
        .map_err(|e| tracing::warn!("query history disabled: {e}"))
        .ok();
    let service = Rowhouse::new(store, history)
        .serve(rmcp::transport::stdio())
        .await?;
    service.waiting().await?;
    Ok(())
}
