//! rowhouse-mcp — stdio MCP server over the connections saved in Rowhouse.
//!
//! Safety model (MVP): the server is **read-only**. Every query goes through
//! `Connection::query_read_only` (keyword guard + read-only transaction), and
//! only profiles with `mcpEnabled: true` are visible.
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
use rowhouse_core::{history::NewEntry, Connection, ConnectionConfig, ConnectionStore, History};
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
            .map(|c| json!({ "name": c.name, "id": c.id, "kind": c.kind, "database": c.database.or(c.file) }))
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
        let sql = c.select_table_sql(&schema, &a.table, limit);
        self.run_logged(&c, &sql, limit).await
    }

    #[tool(
        description = "Run a READ-ONLY SQL query. Writes (INSERT/UPDATE/DELETE/DDL) are rejected."
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
                "Rowhouse gives read-only access to the user's saved database connections. \
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
