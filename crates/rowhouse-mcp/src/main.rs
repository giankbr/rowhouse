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
        let out = res.map_err(|e| e.to_string())?;
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
                 Start with list_connections, then list_tables / describe_table before writing SQL.",
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
