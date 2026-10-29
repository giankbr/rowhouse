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
    config::{ConnectionConfig, DbKind},
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
}

pub(crate) enum PoolRef<'a> {
    Pg(&'a PgPool),
    My(&'a MySqlPool),
    Lite(&'a SqlitePool),
}

pub struct Connection {
    pub config: ConnectionConfig,
    pool: Pool,
    /// Kept alive for as long as the connection; dropped = tunnel closed.
    tunnel: Option<crate::tunnel::Tunnel>,
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
        let timeout = Duration::from_secs(10);
        let mut host = config.host.clone().unwrap_or_else(|| "localhost".into());
        let mut port = config.port.or(config.kind.default_port()).unwrap_or(0);
        let tunnel = match (&config.ssh, config.kind) {
            (Some(ssh), DbKind::Postgres | DbKind::Mysql) => {
                let t = crate::tunnel::Tunnel::open(ssh, ssh_secret, &host, port).await?;
                host = "127.0.0.1".into();
                port = t.local_port;
                Some(t)
            }
            _ => None,
        };
        let pool = match config.kind {
            DbKind::Postgres => {
                let mut o = PgConnectOptions::new()
                    .host(&host)
                    .port(port)
                    .application_name("rowhouse");
                if let Some(u) = &config.user {
                    o = o.username(u);
                }
                if let Some(p) = password {
                    o = o.password(p);
                }
                if let Some(db) = &config.database {
                    o = o.database(db);
                }
                Pool::Pg(
                    PgPoolOptions::new()
                        .max_connections(4)
                        .acquire_timeout(timeout)
                        .connect_with(o)
                        .await?,
                )
            }
            DbKind::Mysql => {
                let mut o = MySqlConnectOptions::new().host(&host).port(port);
                if let Some(u) = &config.user {
                    o = o.username(u);
                }
                if let Some(p) = password {
                    o = o.password(p);
                }
                if let Some(db) = &config.database {
                    o = o.database(db);
                }
                Pool::My(
                    MySqlPoolOptions::new()
                        .max_connections(4)
                        .acquire_timeout(timeout)
                        .connect_with(o)
                        .await?,
                )
            }
            DbKind::Sqlite => {
                let file = config
                    .file
                    .clone()
                    .ok_or_else(|| Error::Other("SQLite profile has no file path".into()))?;
                let o = SqliteConnectOptions::new()
                    .filename(file)
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
        };
        Ok(Self {
            config: config.clone(),
            pool,
            tunnel,
        })
    }

    pub(crate) fn pool(&self) -> PoolRef<'_> {
        match &self.pool {
            Pool::Pg(p) => PoolRef::Pg(p),
            Pool::My(p) => PoolRef::My(p),
            Pool::Lite(p) => PoolRef::Lite(p),
        }
    }

    /// Fingerprint of the SSH host key accepted for this connection, if tunnelled.
    pub fn ssh_host_key(&self) -> Option<&str> {
        self.tunnel.as_ref().map(|t| t.host_key.as_str())
    }

    pub fn kind(&self) -> DbKind {
        self.config.kind
    }

    pub async fn close(&self) {
        match &self.pool {
            Pool::Pg(p) => p.close().await,
            Pool::My(p) => p.close().await,
            Pool::Lite(p) => p.close().await,
        }
    }

    /// Run a script. Respects the profile's `read_only` flag.
    pub async fn query(&self, sql: &str, limit: usize) -> Result<QueryOutput> {
        if self.config.read_only {
            return self.query_read_only(sql, limit).await;
        }
        let started = Instant::now();
        let results = match &self.pool {
            Pool::Pg(p) => run_script!(p, sql, limit, pg_value),
            Pool::My(p) => run_script!(p, sql, limit, my_value),
            Pool::Lite(p) => run_script!(p, sql, limit, lite_value),
        };
        Ok(QueryOutput {
            results,
            elapsed_ms: started.elapsed().as_millis(),
        })
    }

    /// Run a script that must not modify anything. Two layers:
    /// the [`guard`] classifier, then a read-only transaction / `query_only` session.
    pub async fn query_read_only(&self, sql: &str, limit: usize) -> Result<QueryOutput> {
        guard::check_read_only(sql).map_err(Error::ReadOnly)?;
        let started = Instant::now();
        let results = match &self.pool {
            Pool::Pg(p) => {
                let mut c = p.acquire().await?;
                sqlx::raw_sql("BEGIN READ ONLY").execute(&mut *c).await?;
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
        };
        Ok(QueryOutput {
            results,
            elapsed_ms: started.elapsed().as_millis(),
        })
    }

    /// Quick health check.
    pub async fn ping(&self) -> Result<()> {
        self.meta("SELECT 1").await.map(|_| ())
    }

    pub async fn list_tables(&self) -> Result<Vec<TableInfo>> {
        let sql = match self.kind() {
            DbKind::Postgres => "SELECT table_schema, table_name, table_type FROM information_schema.tables \
                 WHERE table_schema NOT IN ('pg_catalog','information_schema') \
                 AND table_schema NOT LIKE 'pg_toast%' ORDER BY 1, 2"
                .to_string(),
            DbKind::Mysql => "SELECT table_schema, table_name, table_type FROM information_schema.tables \
                 WHERE (DATABASE() IS NULL AND table_schema NOT IN ('mysql','information_schema','performance_schema','sys')) \
                 OR table_schema = DATABASE() ORDER BY 1, 2"
                .to_string(),
            DbKind::Sqlite => "SELECT 'main', name, type FROM sqlite_master \
                 WHERE type IN ('table','view') AND name NOT LIKE 'sqlite_%' ORDER BY 2"
                .to_string(),
        };
        Ok(self
            .meta(&sql)
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
        let (s, t) = (self.literal(schema), self.literal(table));
        let rows = match self.kind() {
            DbKind::Postgres => {
                let sql = format!(
                    "SELECT c.column_name, \
                       CASE WHEN c.data_type IN ('USER-DEFINED','ARRAY') THEN c.udt_name ELSE c.data_type END, \
                       c.is_nullable, c.column_default, \
                       EXISTS (SELECT 1 FROM information_schema.table_constraints tc \
                         JOIN information_schema.key_column_usage k \
                           ON k.constraint_name = tc.constraint_name AND k.table_schema = tc.table_schema \
                         WHERE tc.constraint_type = 'PRIMARY KEY' AND tc.table_schema = c.table_schema \
                           AND tc.table_name = c.table_name AND k.column_name = c.column_name) \
                     FROM information_schema.columns c \
                     WHERE c.table_schema = {s} AND c.table_name = {t} ORDER BY c.ordinal_position"
                );
                self.meta(&sql).await?.rows
            }
            DbKind::Mysql => {
                let sql = format!(
                    "SELECT column_name, column_type, is_nullable, column_default, column_key = 'PRI' \
                     FROM information_schema.columns WHERE table_schema = {s} AND table_name = {t} \
                     ORDER BY ordinal_position"
                );
                self.meta(&sql).await?.rows
            }
            DbKind::Sqlite => {
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

    /// `SELECT * FROM <table> LIMIT n` with correct identifier quoting.
    pub fn select_table_sql(&self, schema: &str, table: &str, limit: usize) -> String {
        format!("{} LIMIT {limit}", self.select_all_sql(schema, table))
    }

    /// `SELECT * FROM <table>` (no limit — for full exports).
    pub fn select_all_sql(&self, schema: &str, table: &str) -> String {
        format!("SELECT * FROM {}", self.target(schema, table))
    }

    pub fn ident(&self, name: &str) -> String {
        match self.kind() {
            DbKind::Mysql => format!("`{}`", name.replace('`', "``")),
            _ => format!("\"{}\"", name.replace('"', "\"\"")),
        }
    }

    pub fn literal(&self, s: &str) -> String {
        match self.kind() {
            DbKind::Mysql => format!("'{}'", s.replace('\\', "\\\\").replace('\'', "''")),
            _ => format!("'{}'", s.replace('\'', "''")),
        }
    }

    /// Internal metadata query: first result set, no row limit, ignores read_only.
    async fn meta(&self, sql: &str) -> Result<QueryResult> {
        let mut results = match &self.pool {
            Pool::Pg(p) => run_script!(p, sql, usize::MAX, pg_value),
            Pool::My(p) => run_script!(p, sql, usize::MAX, my_value),
            Pool::Lite(p) => run_script!(p, sql, usize::MAX, lite_value),
        };
        Ok(if results.is_empty() {
            QueryResult::default()
        } else {
            results.remove(0)
        })
    }
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
