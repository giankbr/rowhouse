//! One `Connection` type over every supported database.
//!
//! Queries run through `sqlx::raw_sql`, i.e. the *simple / text* protocol. That gives
//! us three things a DB GUI wants:
//!   1. multi-statement scripts work (`select 1; select 2;`)
//!   2. every value arrives as text, so any column type can be displayed
//!   3. no prepared-statement cache bloat from ad-hoc queries
//!
//! Values are then mapped to JSON (numbers / bools / json stay typed, everything
//! else is a string) for the UI grid and the MCP server.

use std::time::{Duration, Instant};

use futures_util::TryStreamExt;
use serde_json::{Number, Value};
use sqlx::{
    mysql::{MySqlConnectOptions, MySqlPool, MySqlPoolOptions, MySqlRow},
    postgres::{PgConnectOptions, PgPool, PgPoolOptions, PgRow},
    sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions, SqliteRow},
    AssertSqlSafe, Column, Either, Row, TypeInfo, ValueRef,
};

use crate::{
    backend::{self, Expect},
    config::{ConnectionConfig, DbKind, Driver, TlsMode},
    guard, ColumnInfo, Error, QueryOutput, QueryResult, Result, ResultColumn, TableInfo,
};

/// Stream the first result set of `$sql`, calling `$f(&columns, values)` per row.
/// Evaluates to the number of rows seen.
macro_rules! for_each_row {
    ($exec:expr, $sql:expr, $decode:path, $f:expr) => {{
        use futures_util::TryStreamExt as _;
        use sqlx::{Column as _, Row as _, TypeInfo as _};
        let mut stream = sqlx::raw_sql(sqlx::AssertSqlSafe($sql.to_string())).fetch_many($exec);
        let mut cols: Option<Vec<$crate::ResultColumn>> = None;
        let mut n = 0u64;
        while let Some(item) = stream.try_next().await? {
            match item {
                sqlx::Either::Left(_) => {
                    if cols.is_some() {
                        break;
                    }
                }
                sqlx::Either::Right(row) => {
                    let c = cols.get_or_insert_with(|| {
                        row.columns()
                            .iter()
                            .map(|c| $crate::ResultColumn {
                                name: c.name().to_string(),
                                data_type: c.type_info().name().to_string(),
                            })
                            .collect()
                    });
                    let vals: Vec<serde_json::Value> =
                        (0..row.len()).map(|i| $decode(&row, i)).collect();
                    $f(c, vals)?;
                    n += 1;
                }
            }
        }
        n
    }};
}
pub(crate) use for_each_row;

/// Stream a script and fold it into one `QueryResult` per statement.
macro_rules! run_script {
    ($exec:expr, $sql:expr, $limit:expr, $decode:ident) => {{
        let mut stream = sqlx::raw_sql(AssertSqlSafe($sql.to_string())).fetch_many($exec);
        let mut results: Vec<QueryResult> = Vec::new();
        let mut cur = QueryResult::default();
        let mut has_rows = false;
        while let Some(item) = stream.try_next().await? {
            match item {
                Either::Left(done) => {
                    cur.rows_affected = done.rows_affected();
                    results.push(std::mem::take(&mut cur));
                    has_rows = false;
                }
                Either::Right(row) => {
                    if !has_rows {
                        cur.columns = row
                            .columns()
                            .iter()
                            .map(|c| ResultColumn {
                                name: c.name().to_string(),
                                data_type: c.type_info().name().to_string(),
                            })
                            .collect();
                        has_rows = true;
                    }
                    if cur.rows.len() >= $limit {
                        cur.truncated = true;
                        // Stop pulling rows: dropping the stream cancels the rest.
                        break;
                    }
                    cur.rows
                        .push((0..row.len()).map(|i| $decode(&row, i)).collect());
                }
            }
        }
        drop(stream);
        if has_rows || cur.truncated {
            results.push(cur);
        }
        results
    }};
}

/// Default max rows returned per statement.
pub const DEFAULT_ROW_LIMIT: usize = 1_000;

enum Pool {
    Pg(PgPool),
    My(MySqlPool),
    Lite(SqlitePool),
    Duck(backend::duck::Client),
    Mssql(backend::mssql::Client),
    Ch(backend::clickhouse::Client),
    Redis(backend::redis::Client),
    Mongo(backend::mongo::Client),
    Libsql(backend::libsql::Client),
    D1(backend::d1::Client),
    Cql(backend::cassandra::Client),
}

pub(crate) enum PoolRef<'a> {
    Pg(&'a PgPool),
    My(&'a MySqlPool),
    Lite(&'a SqlitePool),
    /// Non-sqlx backend: callers use `Connection::query` / `exec_tx`.
    Other,
}

pub struct Connection {
    pub config: ConnectionConfig,
    pool: Pool,
    /// Kept alive for as long as the connection; dropped = tunnel closed.
    tunnel: Option<crate::tunnel::Tunnel>,
    /// Server-side session ids of running tagged queries (for cancel).
    running: std::sync::Mutex<std::collections::HashMap<String, i64>>,
}

impl Connection {
    /// Connect using the secrets stored in the OS keychain.
    pub async fn connect(config: &ConnectionConfig) -> Result<Self> {
        let password = config.password()?;
        let ssh_secret = config.ssh_secret()?;
        Self::connect_with_secrets(config, password.as_deref(), ssh_secret.as_deref()).await
    }

    /// Connect with an explicit DB password (no SSH secret).
    pub async fn connect_with(config: &ConnectionConfig, password: Option<&str>) -> Result<Self> {
        Self::connect_with_secrets(config, password, None).await
    }

    /// Connect with explicit secrets (used by "Test connection" before saving).
    pub async fn connect_with_secrets(
        config: &ConnectionConfig,
        password: Option<&str>,
        ssh_secret: Option<&str>,
    ) -> Result<Self> {
        crate::init_crypto();
        let timeout = Duration::from_secs(10);
        let mut host = config.host.clone().unwrap_or_else(|| "localhost".into());
        let mut port = config.port.or(config.kind.default_port()).unwrap_or(0);
        let tunnel = match &config.ssh {
            Some(ssh) if !config.kind.is_file() && !config.kind.is_http_api() => {
                let t = crate::tunnel::Tunnel::open(ssh, ssh_secret, &host, port).await?;
                host = "127.0.0.1".into();
                port = t.local_port;
                Some(t)
            }
            _ => None,
        };
        let user = config.user.as_deref();
        let db = config.database.as_deref();
        let file = || {
            config
                .file
                .clone()
                .ok_or_else(|| Error::Other("profile has no database file path".into()))
        };
        let pool = match config.kind.driver() {
            _ if config.kind == DbKind::Turso => Pool::Libsql(
                backend::libsql::Client::connect(config.host.as_deref().unwrap_or(""), password)
                    .await?,
            ),
            _ if config.kind == DbKind::D1 => Pool::D1(
                backend::d1::Client::connect(
                    config.host.as_deref(),
                    user.unwrap_or(""),
                    db.unwrap_or(""),
                    password.unwrap_or(""),
                )
                .await?,
            ),
            Driver::Postgres => {
                let mut o = PgConnectOptions::new()
                    .host(&host)
                    .port(port)
                    .application_name("rowhouse");
                if let Some(u) = user {
                    o = o.username(u);
                }
                if let Some(p) = password {
                    o = o.password(p);
                }
                if let Some(db) = db {
                    o = o.database(db);
                }
                if let Some(t) = &config.tls {
                    use sqlx::postgres::PgSslMode as M;
                    o = o.ssl_mode(match t.mode {
                        TlsMode::Disable => M::Disable,
                        TlsMode::Prefer => M::Prefer,
                        TlsMode::Require => M::Require,
                        TlsMode::VerifyCa => M::VerifyCa,
                        TlsMode::VerifyFull => M::VerifyFull,
                    });
                    if let Some(p) = tls_path(&t.ca_cert) {
                        o = o.ssl_root_cert(p);
                    }
                    if let Some(p) = tls_path(&t.client_cert) {
                        o = o.ssl_client_cert(p);
                    }
                    if let Some(p) = tls_path(&t.client_key) {
                        o = o.ssl_client_key(p);
                    }
                }
                Pool::Pg(
                    PgPoolOptions::new()
                        .max_connections(4)
                        .acquire_timeout(timeout)
                        .connect_with(o)
                        .await?,
                )
            }
            Driver::Mysql => {
                let mut o = MySqlConnectOptions::new().host(&host).port(port);
                if let Some(u) = user {
                    o = o.username(u);
                }
                if let Some(p) = password {
                    o = o.password(p);
                }
                if let Some(db) = db {
                    o = o.database(db);
                }
                if let Some(t) = &config.tls {
                    use sqlx::mysql::MySqlSslMode as M;
                    o = o.ssl_mode(match t.mode {
                        TlsMode::Disable => M::Disabled,
                        TlsMode::Prefer => M::Preferred,
                        TlsMode::Require => M::Required,
                        TlsMode::VerifyCa => M::VerifyCa,
                        TlsMode::VerifyFull => M::VerifyIdentity,
                    });
                    if let Some(p) = tls_path(&t.ca_cert) {
                        o = o.ssl_ca(p);
                    }
                    if let Some(p) = tls_path(&t.client_cert) {
                        o = o.ssl_client_cert(p);
                    }
                    if let Some(p) = tls_path(&t.client_key) {
                        o = o.ssl_client_key(p);
                    }
                }
                Pool::My(
                    MySqlPoolOptions::new()
                        .max_connections(4)
                        .acquire_timeout(timeout)
                        .connect_with(o)
                        .await?,
                )
            }
            Driver::Sqlite => {
                let o = SqliteConnectOptions::new()
                    .filename(file()?)
                    .create_if_missing(false)
                    .read_only(config.read_only);
                Pool::Lite(
                    SqlitePoolOptions::new()
                        .max_connections(4)
                        .acquire_timeout(timeout)
                        .connect_with(o)
                        .await?,
                )
            }
            Driver::Duckdb => {
                Pool::Duck(backend::duck::Client::open(&file()?, config.read_only).await?)
            }
            Driver::Mssql => Pool::Mssql(
                backend::mssql::Client::connect(
                    &host,
                    port,
                    user,
                    password,
                    db,
                    config.tls.as_ref(),
                )
                .await?,
            ),
            Driver::Redis => Pool::Redis(
                backend::redis::Client::connect(
                    &host,
                    port,
                    user,
                    password,
                    db,
                    config.tls.as_ref(),
                )
                .await?,
            ),
            Driver::Cassandra => Pool::Cql(
                backend::cassandra::Client::connect(
                    &host,
                    port,
                    user,
                    password,
                    db,
                    config.tls.as_ref(),
                )
                .await?,
            ),
            Driver::Mongo => Pool::Mongo(
                backend::mongo::Client::connect(
                    &host,
                    port,
                    user,
                    password,
                    db,
                    config.tls.as_ref(),
                )
                .await?,
            ),
            Driver::Clickhouse => Pool::Ch(
                backend::clickhouse::Client::connect(
                    &host,
                    port,
                    user,
                    password,
                    db,
                    config.tls.as_ref(),
                )
                .await?,
            ),
        };
        Ok(Self {
            config: config.clone(),
            pool,
            tunnel,
            running: Default::default(),
        })
    }

    pub(crate) fn pool(&self) -> PoolRef<'_> {
        match &self.pool {
            Pool::Pg(p) => PoolRef::Pg(p),
            Pool::My(p) => PoolRef::My(p),
            Pool::Lite(p) => PoolRef::Lite(p),
            _ => PoolRef::Other,
        }
    }

    /// Fingerprint of the SSH host key accepted for this connection, if tunnelled.
    pub fn ssh_host_key(&self) -> Option<&str> {
        self.tunnel.as_ref().map(|t| t.host_key.as_str())
    }

    pub fn kind(&self) -> DbKind {
        self.config.kind
    }

    pub fn driver(&self) -> Driver {
        self.config.kind.driver()
    }

    pub async fn close(&self) {
        match &self.pool {
            Pool::Pg(p) => p.close().await,
            Pool::My(p) => p.close().await,
            Pool::Lite(p) => p.close().await,
            _ => {}
        }
    }

    /// Run a script. Respects the profile's `read_only` flag.
    pub async fn query(&self, sql: &str, limit: usize) -> Result<QueryOutput> {
        if self.config.read_only {
            return self.query_read_only(sql, limit).await;
        }
        let started = Instant::now();
        let results = self.run(sql, limit).await?;
        Ok(QueryOutput {
            results,
            elapsed_ms: started.elapsed().as_millis(),
        })
    }

    /// Like [`query`](Self::query), but remembers the server session running it under
    /// `tag`, so [`cancel`](Self::cancel) can stop it on the server (Postgres family,
    /// MySQL family). Other engines stop when the caller drops the future.
    pub async fn query_tagged(&self, sql: &str, limit: usize, tag: &str) -> Result<QueryOutput> {
        if self.config.read_only {
            return self.query_read_only(sql, limit).await;
        }
        let started = Instant::now();
        let out = |results| QueryOutput {
            results,
            elapsed_ms: started.elapsed().as_millis(),
        };
        match &self.pool {
            Pool::Pg(p) => {
                let mut c = p.acquire().await?;
                let pid: Result<Vec<QueryResult>> =
                    async { Ok(run_script!(&mut *c, "SELECT pg_backend_pid()", 1, pg_value)) }
                        .await;
                if let Some(pid) = pid.ok().as_deref().and_then(first_i64) {
                    self.track(tag, Some(pid));
                }
                let r: Result<Vec<QueryResult>> =
                    async { Ok(run_script!(&mut *c, sql, limit, pg_value)) }.await;
                self.track(tag, None);
                Ok(out(r?))
            }
            Pool::My(p) => {
                let mut c = p.acquire().await?;
                let id: Result<Vec<QueryResult>> =
                    async { Ok(run_script!(&mut *c, "SELECT CONNECTION_ID()", 1, my_value)) }.await;
                if let Some(id) = id.ok().as_deref().and_then(first_i64) {
                    self.track(tag, Some(id));
                }
                let r: Result<Vec<QueryResult>> =
                    async { Ok(run_script!(&mut *c, sql, limit, my_value)) }.await;
                self.track(tag, None);
                Ok(out(r?))
            }
            _ => Ok(out(self.run(sql, limit).await?)),
        }
    }

    fn track(&self, tag: &str, id: Option<i64>) {
        if let Ok(mut m) = self.running.lock() {
            match id {
                Some(id) => m.insert(tag.to_string(), id),
                None => m.remove(tag),
            };
        }
    }

    /// Ask the server to stop the query started with `tag`. `Ok(false)` when the
    /// engine can't cancel server-side (dropping the future is all there is).
    pub async fn cancel(&self, tag: &str) -> Result<bool> {
        let id = self.running.lock().ok().and_then(|mut m| m.remove(tag));
        let Some(id) = id else { return Ok(false) };
        match &self.pool {
            Pool::Pg(p) => {
                sqlx::raw_sql(AssertSqlSafe(format!("SELECT pg_cancel_backend({id})")))
                    .execute(p)
                    .await?;
                Ok(true)
            }
            Pool::My(p) => {
                sqlx::raw_sql(AssertSqlSafe(format!("KILL QUERY {id}")))
                    .execute(p)
                    .await?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    async fn run(&self, sql: &str, limit: usize) -> Result<Vec<QueryResult>> {
        Ok(match &self.pool {
            Pool::Pg(p) => run_script!(p, sql, limit, pg_value),
            Pool::My(p) => run_script!(p, sql, limit, my_value),
            Pool::Lite(p) => run_script!(p, sql, limit, lite_value),
            Pool::Duck(c) => c.run(sql, limit, false).await?,
            Pool::Mssql(c) => c.run(sql, limit, false).await?,
            Pool::Ch(c) => c.run(sql, limit, false).await?,
            Pool::Redis(c) => c.run(sql, limit, false).await?,
            Pool::Mongo(c) => c.run(sql, limit, false).await?,
            Pool::Libsql(c) => c.run(sql, limit, false).await?,
            Pool::D1(c) => c.run(sql, limit, false).await?,
            Pool::Cql(c) => c.run(sql, limit, false).await?,
        })
    }

    /// Run a script that must not modify anything. Two layers:
    /// the [`guard`] classifier, then a read-only transaction / session setting.
    pub async fn query_read_only(&self, sql: &str, limit: usize) -> Result<QueryOutput> {
        let started = Instant::now();
        if let Pool::Redis(c) = &self.pool {
            // Redis commands, not SQL: the backend checks them against its read allowlist
            let results = c.run(sql, limit, true).await?;
            return Ok(QueryOutput {
                results,
                elapsed_ms: started.elapsed().as_millis(),
            });
        }
        if let Pool::Cql(c) = &self.pool {
            // CQL: its own read allowlist (there are no read-only transactions)
            let results = c.run(sql, limit, true).await?;
            return Ok(QueryOutput {
                results,
                elapsed_ms: started.elapsed().as_millis(),
            });
        }
        if let Pool::Mongo(c) = &self.pool {
            // mongosh statements: the backend checks them against its read allowlist
            let results = c.run(sql, limit, true).await?;
            return Ok(QueryOutput {
                results,
                elapsed_ms: started.elapsed().as_millis(),
            });
        }
        guard::check_read_only(sql).map_err(Error::ReadOnly)?;
        let results = match &self.pool {
            Pool::Pg(p) => {
                let mut c = p.acquire().await?;
                // Redshift doesn't know READ ONLY transactions; the guard still applies.
                let begin = if self.kind() == DbKind::Redshift {
                    "BEGIN"
                } else {
                    "BEGIN READ ONLY"
                };
                sqlx::raw_sql(begin).execute(&mut *c).await?;
                let r: Result<Vec<QueryResult>> =
                    async { Ok(run_script!(&mut *c, sql, limit, pg_value)) }.await;
                sqlx::raw_sql("ROLLBACK").execute(&mut *c).await?;
                r?
            }
            Pool::My(p) => {
                let mut c = p.acquire().await?;
                sqlx::raw_sql("START TRANSACTION READ ONLY")
                    .execute(&mut *c)
                    .await?;
                let r: Result<Vec<QueryResult>> =
                    async { Ok(run_script!(&mut *c, sql, limit, my_value)) }.await;
                sqlx::raw_sql("ROLLBACK").execute(&mut *c).await?;
                r?
            }
            Pool::Lite(p) => {
                let mut c = p.acquire().await?;
                sqlx::raw_sql("PRAGMA query_only = ON")
                    .execute(&mut *c)
                    .await?;
                let r: Result<Vec<QueryResult>> =
                    async { Ok(run_script!(&mut *c, sql, limit, lite_value)) }.await;
                sqlx::raw_sql("PRAGMA query_only = OFF")
                    .execute(&mut *c)
                    .await?;
                r?
            }
            Pool::Duck(c) => c.run(sql, limit, true).await?,
            Pool::Mssql(c) => c.run(sql, limit, true).await?,
            Pool::Ch(c) => c.run(sql, limit, true).await?,
            Pool::Libsql(c) => c.run(sql, limit, true).await?,
            Pool::D1(c) => c.run(sql, limit, true).await?,
            Pool::Redis(_) | Pool::Mongo(_) | Pool::Cql(_) => unreachable!("handled above"),
        };
        Ok(QueryOutput {
            results,
            elapsed_ms: started.elapsed().as_millis(),
        })
    }

    /// Run statements atomically (where the database supports transactions);
    /// each statement's affected-row count must satisfy its [`Expect`].
    pub(crate) async fn exec_tx(&self, stmts: Vec<(Expect, String)>) -> Result<u64> {
        macro_rules! sqlx_tx {
            ($pool:expr) => {{
                let mut tx = $pool.begin().await?;
                let mut total = 0u64;
                for (expect, s) in &stmts {
                    let n = sqlx::raw_sql(AssertSqlSafe(s.clone()))
                        .execute(&mut *tx)
                        .await?
                        .rows_affected();
                    if !expect.check(n) {
                        tx.rollback().await?;
                        return Err(Error::Other(format!(
                            "expected 1 row, {n} affected — rolled back.\n{s}"
                        )));
                    }
                    total += n;
                }
                tx.commit().await?;
                Ok(total)
            }};
        }
        match &self.pool {
            Pool::Pg(p) => sqlx_tx!(p),
            Pool::My(p) => sqlx_tx!(p),
            Pool::Lite(p) => sqlx_tx!(p),
            Pool::Duck(c) => c.exec_tx(stmts).await,
            Pool::Mssql(c) => c.exec_tx(stmts).await,
            Pool::Libsql(c) => c.exec_tx(stmts).await,
            Pool::D1(c) => c.exec_tx(stmts).await,
            Pool::Cql(c) => c.exec_tx(stmts).await,
            Pool::Redis(_) => Err(Error::Other(
                "editing isn't supported for Redis — use commands in a query tab".into(),
            )),
            Pool::Mongo(_) => Err(Error::Other(
                "not supported for MongoDB — use the grid or mongosh statements in a query tab"
                    .into(),
            )),
            Pool::Ch(c) => {
                if stmts.iter().any(|(e, _)| *e != Expect::Any) {
                    return Err(Error::Other(
                        "editing rows is not supported for ClickHouse (no transactions)".into(),
                    ));
                }
                c.exec_all(&stmts.into_iter().map(|(_, s)| s).collect::<Vec<_>>())
                    .await
            }
        }
    }

    /// Quick health check.
    pub async fn ping(&self) -> Result<()> {
        if let Pool::Redis(c) = &self.pool {
            return c.raw(&["PING".into()]).await.map(|_| ());
        }
        if let Pool::Mongo(c) = &self.pool {
            return c.ping().await;
        }
        self.meta("SELECT 1").await.map(|_| ())
    }

    pub(crate) fn mongo(&self) -> Option<&backend::mongo::Client> {
        match &self.pool {
            Pool::Mongo(c) => Some(c),
            _ => None,
        }
    }

    pub(crate) fn cql(&self) -> Option<&backend::cassandra::Client> {
        match &self.pool {
            Pool::Cql(c) => Some(c),
            _ => None,
        }
    }

    pub(crate) fn redis(&self) -> Option<&backend::redis::Client> {
        match &self.pool {
            Pool::Redis(c) => Some(c),
            _ => None,
        }
    }

    pub async fn list_tables(&self) -> Result<Vec<TableInfo>> {
        if let Pool::Redis(c) = &self.pool {
            return c.keys(2_000).await;
        }
        if let Pool::Mongo(c) = &self.pool {
            return c.collections().await;
        }
        if let Pool::Cql(c) = &self.pool {
            return c.tables().await;
        }
        let sql = match self.driver() {
            Driver::Redis | Driver::Mongo | Driver::Cassandra => unreachable!("handled above"),
            Driver::Postgres => "SELECT table_schema, table_name, table_type FROM information_schema.tables \
                 WHERE table_schema NOT IN ('pg_catalog','information_schema','crdb_internal','pg_extension','pg_internal','gp_toolkit') \
                 AND table_schema NOT LIKE 'pg_toast%' ORDER BY 1, 2",
            Driver::Mysql => "SELECT table_schema, table_name, table_type FROM information_schema.tables \
                 WHERE (DATABASE() IS NULL AND LOWER(table_schema) NOT IN \
                   ('mysql','information_schema','performance_schema','sys','metrics_schema','memsql','cluster')) \
                 OR table_schema = DATABASE() ORDER BY 1, 2",
            Driver::Sqlite => "SELECT 'main', name, type FROM sqlite_master \
                 WHERE type IN ('table','view') AND name NOT LIKE 'sqlite_%' AND substr(name, 1, 4) <> '_cf_' AND substr(name, 1, 7) <> 'libsql_' ORDER BY 2",
            Driver::Duckdb => "SELECT table_schema, table_name, table_type FROM information_schema.tables \
                 WHERE table_schema NOT IN ('information_schema','pg_catalog') ORDER BY 1, 2",
            Driver::Mssql => "SELECT TABLE_SCHEMA, TABLE_NAME, TABLE_TYPE FROM INFORMATION_SCHEMA.TABLES \
                 WHERE TABLE_SCHEMA NOT IN ('sys','INFORMATION_SCHEMA') ORDER BY 1, 2",
            Driver::Clickhouse => "SELECT database, name, engine FROM system.tables \
                 WHERE database = currentDatabase() ORDER BY name",
        };
        Ok(self
            .meta(sql)
            .await?
            .rows
            .into_iter()
            .map(|r| TableInfo {
                schema: text(&r[0]),
                name: text(&r[1]),
                kind: if text(&r[2]).to_lowercase().contains("view") {
                    "view"
                } else {
                    "table"
                }
                .into(),
            })
            .collect())
    }

    pub async fn describe_table(&self, schema: &str, table: &str) -> Result<Vec<ColumnInfo>> {
        if let Pool::Mongo(c) = &self.pool {
            let _ = schema;
            return c.describe(table).await;
        }
        if let Pool::Cql(c) = &self.pool {
            return c.describe(schema, table).await;
        }
        if let Pool::Redis(c) = &self.pool {
            // a key has no schema; report the value's type as a single pseudo column
            let ty = c.raw(&["TYPE".into(), table.into()]).await?;
            let ty = match ty {
                redis::Value::SimpleString(s) => s,
                other => format!("{other:?}"),
            };
            let _ = schema;
            return Ok(vec![ColumnInfo {
                name: "value".into(),
                data_type: ty,
                nullable: true,
                default: None,
                primary_key: false,
            }]);
        }
        let (s, t) = (self.literal(schema), self.literal(table));
        // information_schema with a portable primary-key flag (Postgres family, DuckDB, SQL Server)
        let info_schema = |type_expr: &str| {
            format!(
                "SELECT c.column_name, {type_expr}, c.is_nullable, c.column_default, \
                   CASE WHEN EXISTS (SELECT 1 FROM information_schema.table_constraints tc \
                     JOIN information_schema.key_column_usage k \
                       ON k.constraint_name = tc.constraint_name AND k.table_schema = tc.table_schema \
                       AND k.table_name = tc.table_name \
                     WHERE tc.constraint_type = 'PRIMARY KEY' AND tc.table_schema = c.table_schema \
                       AND tc.table_name = c.table_name AND k.column_name = c.column_name) THEN 1 ELSE 0 END \
                 FROM information_schema.columns c \
                 WHERE c.table_schema = {s} AND c.table_name = {t} ORDER BY c.ordinal_position"
            )
        };
        let rows = match self.driver() {
            Driver::Redis | Driver::Mongo | Driver::Cassandra => unreachable!("handled above"),
            // Redshift's catalog is too old for this query; it uses information_schema below.
            Driver::Postgres if self.kind() != DbKind::Redshift => {
                // pg_catalog gives full types (varchar(64), numeric(12,2), text[]) and real defaults
                let sql = format!(
                    "SELECT a.attname, format_type(a.atttypid, a.atttypmod), \
                       CASE WHEN a.attnotnull THEN 'NO' ELSE 'YES' END, \
                       pg_get_expr(d.adbin, d.adrelid), \
                       CASE WHEN EXISTS (SELECT 1 FROM pg_index i WHERE i.indrelid = c.oid AND i.indisprimary \
                         AND a.attnum = ANY(i.indkey)) THEN 1 ELSE 0 END \
                     FROM pg_attribute a \
                     JOIN pg_class c ON c.oid = a.attrelid JOIN pg_namespace n ON n.oid = c.relnamespace \
                     LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum \
                     WHERE n.nspname = {s} AND c.relname = {t} AND a.attnum > 0 AND NOT a.attisdropped \
                     ORDER BY a.attnum"
                );
                self.meta(&sql).await?.rows
            }
            Driver::Postgres => {
                let sql = info_schema(
                    "CASE WHEN c.data_type IN ('USER-DEFINED','ARRAY') THEN c.udt_name ELSE c.data_type END",
                );
                self.meta(&sql).await?.rows
            }
            Driver::Duckdb | Driver::Mssql => self.meta(&info_schema("c.data_type")).await?.rows,
            Driver::Mysql => {
                let sql = format!(
                    "SELECT column_name, column_type, is_nullable, column_default, column_key = 'PRI' \
                     FROM information_schema.columns WHERE table_schema = {s} AND table_name = {t} \
                     ORDER BY ordinal_position"
                );
                self.meta(&sql).await?.rows
            }
            Driver::Clickhouse => {
                let sql = format!(
                    "SELECT name, type, if(startsWith(type, 'Nullable'), 'YES', 'NO'), \
                       if(default_expression = '', NULL, default_expression), \
                       is_in_primary_key \
                     FROM system.columns WHERE database = {s} AND table = {t}"
                );
                self.meta(&sql).await?.rows
            }
            Driver::Sqlite => {
                // cid, name, type, notnull, dflt_value, pk
                let sql = format!("PRAGMA table_info({t})");
                self.meta(&sql)
                    .await?
                    .rows
                    .into_iter()
                    .map(|r| {
                        vec![
                            r[1].clone(),
                            r[2].clone(),
                            Value::String(if truthy(&r[3]) { "NO" } else { "YES" }.into()),
                            r[4].clone(),
                            Value::Bool(truthy(&r[5])),
                        ]
                    })
                    .collect()
            }
        };
        Ok(rows
            .into_iter()
            .map(|r| ColumnInfo {
                name: text(&r[0]),
                data_type: text(&r[1]),
                nullable: text(&r[2]).eq_ignore_ascii_case("yes"),
                default: if r[3].is_null() {
                    None
                } else {
                    Some(text(&r[3]))
                },
                primary_key: truthy(&r[4]),
            })
            .collect())
    }

    /// `SELECT * FROM <table> LIMIT n` (or `TOP n`) with correct identifier quoting.
    pub fn select_table_sql(&self, schema: &str, table: &str, limit: usize) -> String {
        if self.driver() == Driver::Mongo {
            return format!("{}.limit({limit})", self.select_all_sql(schema, table));
        }
        if self.driver() == Driver::Mssql {
            return format!("SELECT TOP {limit} * FROM {}", self.target(schema, table));
        }
        format!("{} LIMIT {limit}", self.select_all_sql(schema, table))
    }

    /// `SELECT * FROM <table>` (no limit — for full exports).
    pub fn select_all_sql(&self, schema: &str, table: &str) -> String {
        if self.driver() == Driver::Mongo {
            let _ = schema;
            return if table.chars().all(|c| c.is_alphanumeric() || c == '_') {
                format!("db.{table}.find({{}})")
            } else {
                format!(
                    "db.getCollection({}).find({{}})",
                    serde_json::to_string(table).unwrap_or_default()
                )
            };
        }
        format!("SELECT * FROM {}", self.target(schema, table))
    }

    pub fn ident(&self, name: &str) -> String {
        match self.driver() {
            Driver::Mysql | Driver::Clickhouse => format!("`{}`", name.replace('`', "``")),
            Driver::Mssql => format!("[{}]", name.replace(']', "]]")),
            _ => format!("\"{}\"", name.replace('"', "\"\"")),
        }
    }

    pub fn literal(&self, s: &str) -> String {
        match self.driver() {
            Driver::Mysql | Driver::Clickhouse => {
                format!("'{}'", s.replace('\\', "\\\\").replace('\'', "''"))
            }
            Driver::Mssql => format!("N'{}'", s.replace('\'', "''")),
            _ => format!("'{}'", s.replace('\'', "''")),
        }
    }

    /// Internal metadata query: first result set, no row limit, ignores read_only.
    async fn meta(&self, sql: &str) -> Result<QueryResult> {
        let mut results = self.run(sql, usize::MAX).await?;
        Ok(if results.is_empty() {
            QueryResult::default()
        } else {
            results.remove(0)
        })
    }
}

/// Non-empty, `~`-expanded certificate path.
fn tls_path(p: &Option<String>) -> Option<String> {
    p.as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(crate::tunnel::expand_home)
}

fn text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_i64().unwrap_or(0) != 0,
        Value::String(s) => matches!(s.as_str(), "1" | "t" | "true" | "YES" | "yes"),
        _ => false,
    }
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(2 + bytes.len() * 2);
    s.push_str("0x");
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn num_from_str(s: &str) -> Value {
    if let Ok(i) = s.parse::<i64>() {
        return Value::Number(i.into());
    }
    if let Ok(u) = s.parse::<u64>() {
        return Value::Number(u.into());
    }
    match s.parse::<f64>().ok().and_then(Number::from_f64) {
        Some(n) => Value::Number(n),
        None => Value::String(s.to_string()),
    }
}

pub(crate) fn pg_value(row: &PgRow, i: usize) -> Value {
    let Ok(raw) = row.try_get_raw(i) else {
        return Value::Null;
    };
    if raw.is_null() {
        return Value::Null;
    }
    let ty = raw.type_info().name().to_ascii_uppercase();
    let Ok(s) = raw.as_str() else {
        return raw
            .as_bytes()
            .map(|b| Value::String(hex(b)))
            .unwrap_or(Value::Null);
    };
    match ty.as_str() {
        "BOOL" => Value::Bool(s == "t"),
        "INT2" | "INT4" | "INT8" | "OID" | "FLOAT4" | "FLOAT8" => num_from_str(s),
        // NUMERIC stays a string to keep full precision.
        "JSON" | "JSONB" => serde_json::from_str(s).unwrap_or_else(|_| Value::String(s.into())),
        _ => Value::String(s.to_string()),
    }
}

pub(crate) fn my_value(row: &MySqlRow, i: usize) -> Value {
    let Ok(raw) = row.try_get_raw(i) else {
        return Value::Null;
    };
    if raw.is_null() {
        return Value::Null;
    }
    let ty = raw.type_info().name().to_ascii_uppercase();
    let Ok(bytes) = row.try_get_unchecked::<Vec<u8>, _>(i) else {
        return Value::Null;
    };
    let Ok(s) = std::str::from_utf8(&bytes) else {
        return Value::String(hex(&bytes));
    };
    let base = ty.trim_end_matches(" UNSIGNED");
    match base {
        "BOOLEAN" => Value::Bool(s == "1"),
        "TINYINT" | "SMALLINT" | "MEDIUMINT" | "INT" | "BIGINT" | "FLOAT" | "DOUBLE" | "YEAR" => {
            num_from_str(s)
        }
        "JSON" => serde_json::from_str(s).unwrap_or_else(|_| Value::String(s.into())),
        _ if base.contains("BLOB") || base.contains("BINARY") => {
            if s.chars().all(|c| !c.is_control() || c.is_whitespace()) {
                Value::String(s.to_string())
            } else {
                Value::String(hex(&bytes))
            }
        }
        _ => Value::String(s.to_string()),
    }
}

pub(crate) fn lite_value(row: &SqliteRow, i: usize) -> Value {
    let Ok(raw) = row.try_get_raw(i) else {
        return Value::Null;
    };
    if raw.is_null() {
        return Value::Null;
    }
    let storage = raw.type_info().name().to_ascii_uppercase();
    match storage.as_str() {
        "INTEGER" => row
            .try_get_unchecked::<i64, _>(i)
            .map(|v| v.into())
            .unwrap_or(Value::Null),
        "REAL" => row
            .try_get_unchecked::<f64, _>(i)
            .ok()
            .and_then(Number::from_f64)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        "BLOB" => row
            .try_get_unchecked::<Vec<u8>, _>(i)
            .map(|b| Value::String(hex(&b)))
            .unwrap_or(Value::Null),
        _ => row
            .try_get_unchecked::<String, _>(i)
            .map(Value::String)
            .unwrap_or(Value::Null),
    }
}

/// First cell of the first result as i64.
fn first_i64(r: &[QueryResult]) -> Option<i64> {
    match r.first()?.rows.first()?.first()? {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}
