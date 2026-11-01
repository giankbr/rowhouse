//! Oracle Database via ODPI-C (`oracle` crate). The Oracle Client libraries
//! (Instant Client "Basic" or "Basic Light") are loaded at runtime, so they're
//! only needed when you actually connect to Oracle. We look in the usual
//! places, in `ROWHOUSE_ORACLE_CLIENT` / the connection's client folder, and
//! explain how to install it when it's missing.

use std::{
    path::PathBuf,
    sync::{Arc, OnceLock},
};

use oracle::sql_type::OracleType;
use serde_json::{Number, Value};

use crate::{backend::Expect, Error, QueryResult, Result, ResultColumn};

#[derive(Clone)]
pub struct Client {
    conn: Arc<oracle::Connection>,
    /// one statement at a time per connection (OCI serializes anyway)
    lock: Arc<tokio::sync::Mutex<()>>,
}

fn err(e: oracle::Error) -> Error {
    let s = e.to_string();
    if s.contains("DPI-1047") {
        return Error::Other(format!(
            "Oracle Instant Client wasn't found. Download \"Basic Light\" for your OS from \
             https://www.oracle.com/database/technologies/instant-client/downloads.html, unzip it \
             (e.g. to ~/instantclient or C:\\oracle\\instantclient), then pick that folder under \
             the connection's Oracle client setting or set ROWHOUSE_ORACLE_CLIENT. ({s})"
        ));
    }
    Error::Other(format!("oracle: {s}"))
}

fn home() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|d| d.home_dir().to_path_buf())
}

/// Folders that look like an Instant Client install (contain libclntsh / oci.dll).
pub fn find_instant_client(hint: Option<&str>) -> Option<PathBuf> {
    let has_lib = |d: &std::path::Path| {
        std::fs::read_dir(d).ok().is_some_and(|rd| {
            rd.flatten().any(|e| {
                let n = e.file_name().to_string_lossy().to_lowercase();
                n.starts_with("libclntsh") || n == "oci.dll"
            })
        })
    };
    let mut cands: Vec<PathBuf> = vec![];
    if let Some(h) = hint.filter(|h| !h.trim().is_empty()) {
        cands.push(crate::tunnel::expand_home(h).into());
    }
    if let Ok(p) = std::env::var("ROWHOUSE_ORACLE_CLIENT") {
        cands.push(p.into());
    }
    let mut roots: Vec<PathBuf> = vec![
        "/opt/oracle".into(),
        "/usr/lib/oracle".into(),
        "/usr/local/lib".into(),
        "/opt/homebrew/lib".into(),
        "C:\\oracle".into(),
        "C:\\instantclient".into(),
    ];
    if let Some(h) = home() {
        for r in ["", "Downloads", "lib", "oracle", "Applications", "opt"] {
            roots.push(h.join(r));
        }
    }
    for r in roots {
        if let Ok(rd) = std::fs::read_dir(&r) {
            let mut v: Vec<PathBuf> = rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name().is_some_and(|n| {
                        n.to_string_lossy()
                            .to_lowercase()
                            .starts_with("instantclient")
                    })
                })
                .collect();
            v.sort();
            cands.extend(v.into_iter().rev());
            // /usr/lib/oracle/21/client64/lib
            if r.ends_with("oracle") {
                if let Ok(rd) = std::fs::read_dir(&r) {
                    for e in rd.flatten() {
                        cands.push(e.path().join("client64/lib"));
                    }
                }
            }
        }
    }
    cands.into_iter().find(|d| d.is_dir() && has_lib(d))
}

static INIT: OnceLock<std::result::Result<(), String>> = OnceLock::new();

fn init_client(hint: Option<&str>) -> Result<()> {
    INIT.get_or_init(|| {
        let mut p = oracle::InitParams::new();
        p.default_driver_name("rowhouse")
            .map_err(|e| e.to_string())?;
        if let Some(dir) = find_instant_client(hint) {
            p.oracle_client_lib_dir(dir).map_err(|e| e.to_string())?;
        }
        p.init().map(|_| ()).map_err(|e| err(e).to_string())
    })
    .clone()
    .map_err(Error::Other)
}

/// `//host:port/service`, or the host field as-is when it's already a
/// connect string (EZConnect with a path, a TNS alias or a descriptor).
pub fn connect_string(host: &str, port: u16, service: Option<&str>) -> String {
    let h = host.trim();
    if h.contains('/')
        || h.contains('(')
        || (!h.contains('.')
            && !h.contains(':')
            && h != "localhost"
            && service.is_none()
            && h.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
    {
        return h.to_string();
    }
    match service.filter(|s| !s.trim().is_empty()) {
        Some(s) => format!("//{h}:{port}/{}", s.trim()),
        None => format!("//{h}:{port}"),
    }
}

/// Split a script: SQL statements at `;` (the `;` is dropped — Oracle rejects
/// it), PL/SQL blocks and CREATE FUNCTION/PROCEDURE/PACKAGE/TRIGGER/TYPE up to a
/// line with just `/` (or the end).
pub fn split(script: &str) -> Vec<String> {
    let mut out = vec![];
    let mut sql_buf = String::new();
    let mut plsql: Option<String> = None;
    let flush_sql = |buf: &mut String, out: &mut Vec<String>| {
        for s in crate::script::split(buf, false) {
            out.push(s.sql.trim().trim_end_matches(';').trim().to_string());
        }
        buf.clear();
    };
    for line in script.lines() {
        if let Some(block) = plsql.as_mut() {
            if line.trim() == "/" {
                out.push(block.trim().to_string());
                plsql = None;
            } else {
                block.push_str(line);
                block.push('\n');
            }
            continue;
        }
        let t = line.trim_start().to_uppercase();
        let starts_block =
            (t.starts_with("BEGIN") || t.starts_with("DECLARE") || is_plsql_create(&t))
                && (sql_buf.trim().is_empty() || sql_buf.trim_end().ends_with(';'));
        if starts_block {
            flush_sql(&mut sql_buf, &mut out);
            plsql = Some(format!("{line}\n"));
        } else if line.trim() == "/" {
            flush_sql(&mut sql_buf, &mut out);
        } else {
            sql_buf.push_str(line);
            sql_buf.push('\n');
        }
    }
    if let Some(b) = plsql {
        if !b.trim().is_empty() {
            out.push(b.trim().to_string());
        }
    }
    flush_sql(&mut sql_buf, &mut out);
    out.retain(|s| !s.is_empty());
    out
}

fn is_plsql_create(upper: &str) -> bool {
    let rest = upper
        .strip_prefix("CREATE")
        .map(str::trim_start)
        .unwrap_or("");
    let rest = rest
        .strip_prefix("OR REPLACE")
        .map(str::trim_start)
        .unwrap_or(rest);
    let rest = rest
        .strip_prefix("EDITIONABLE")
        .or_else(|| rest.strip_prefix("NONEDITIONABLE"))
        .map(str::trim_start)
        .unwrap_or(rest);
    [
        "FUNCTION",
        "PROCEDURE",
        "PACKAGE",
        "TRIGGER",
        "TYPE BODY",
        "TYPE ",
    ]
    .iter()
    .any(|k| rest.starts_with(k))
        && upper.starts_with("CREATE")
}

fn num(s: &str) -> Value {
    if let Ok(i) = s.parse::<i64>() {
        return Value::from(i);
    }
    // keep precision: NUMBER can hold more than an f64
    if s.len() <= 15 {
        if let Some(n) = s.parse::<f64>().ok().and_then(Number::from_f64) {
            return Value::Number(n);
        }
    }
    Value::String(s.to_string())
}

fn cell(v: &oracle::SqlValue, t: &OracleType) -> Value {
    if v.is_null().unwrap_or(true) {
        return Value::Null;
    }
    match t {
        OracleType::Number(_, _)
        | OracleType::Float(_)
        | OracleType::Int64
        | OracleType::UInt64 => v.get::<String>().map(|s| num(&s)).unwrap_or(Value::Null),
        OracleType::BinaryFloat | OracleType::BinaryDouble => v
            .get::<f64>()
            .ok()
            .and_then(Number::from_f64)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        OracleType::Boolean => v.get::<bool>().map(Value::Bool).unwrap_or(Value::Null),
        OracleType::Raw(_) | OracleType::BLOB | OracleType::LongRaw => v
            .get::<Vec<u8>>()
            .map(|b| {
                Value::String(format!(
                    "0x{}",
                    b.iter()
                        .take(4096)
                        .map(|x| format!("{x:02x}"))
                        .collect::<String>()
                ))
            })
            .unwrap_or(Value::Null),
        OracleType::Date => v
            .get::<chrono::NaiveDateTime>()
            .map(|d| Value::String(d.format("%Y-%m-%d %H:%M:%S").to_string()))
            .unwrap_or_else(|_| Value::String(v.to_string())),
        OracleType::Timestamp(_) => v
            .get::<chrono::NaiveDateTime>()
            .map(|d| Value::String(d.format("%Y-%m-%d %H:%M:%S%.f").to_string()))
            .unwrap_or_else(|_| Value::String(v.to_string())),
        _ => v
            .get::<String>()
            .map(Value::String)
            .unwrap_or_else(|_| Value::String(v.to_string())),
    }
}

impl Client {
    pub async fn connect(
        host: &str,
        port: u16,
        user: Option<&str>,
        password: Option<&str>,
        service: Option<&str>,
        client_dir: Option<&str>,
    ) -> Result<Self> {
        let cs = connect_string(host, port, service);
        let (u, p, dir) = (
            user.unwrap_or("system").to_string(),
            password.unwrap_or_default().to_string(),
            client_dir.map(String::from),
        );
        let conn = tokio::task::spawn_blocking(move || -> Result<oracle::Connection> {
            init_client(dir.as_deref())?;
            let c = oracle::Connector::new(u, p, cs).connect().map_err(err)?;
            // dates as ISO text both ways, so grid edits and filters round-trip
            c.execute(
                "ALTER SESSION SET NLS_DATE_FORMAT = 'YYYY-MM-DD HH24:MI:SS' \
                 NLS_TIMESTAMP_FORMAT = 'YYYY-MM-DD HH24:MI:SS.FF' \
                 NLS_TIMESTAMP_TZ_FORMAT = 'YYYY-MM-DD HH24:MI:SS.FF TZH:TZM' \
                 NLS_NUMERIC_CHARACTERS = '.,'",
                &[],
            )
            .map_err(err)?;
            Ok(c)
        })
        .await
        .map_err(|e| Error::Other(e.to_string()))??;
        Ok(Self {
            conn: Arc::new(conn),
            lock: Default::default(),
        })
    }

    fn run_one(c: &oracle::Connection, sql: &str, limit: usize) -> Result<QueryResult> {
        let mut st = c
            .statement(sql)
            .fetch_array_size(256)
            .build()
            .map_err(err)?;
        if st.is_query() {
            let rs = st.query(&[]).map_err(err)?;
            let info: Vec<(String, OracleType)> = rs
                .column_info()
                .iter()
                .map(|ci| (ci.name().to_string(), ci.oracle_type().clone()))
                .collect();
            let mut rows = vec![];
            let mut truncated = false;
            for row in rs {
                if rows.len() >= limit {
                    truncated = true;
                    break;
                }
                let row = row.map_err(err)?;
                rows.push(
                    row.sql_values()
                        .iter()
                        .zip(&info)
                        .map(|(v, (_, t))| cell(v, t))
                        .collect(),
                );
            }
            Ok(QueryResult {
                columns: info
                    .into_iter()
                    .map(|(name, t)| ResultColumn {
                        name,
                        data_type: t.to_string(),
                    })
                    .collect(),
                rows,
                rows_affected: 0,
                truncated,
            })
        } else {
            st.execute(&[]).map_err(err)?;
            Ok(QueryResult {
                rows_affected: st.row_count().unwrap_or(0),
                ..Default::default()
            })
        }
    }

    pub async fn run(&self, sql: &str, limit: usize, read_only: bool) -> Result<Vec<QueryResult>> {
        let _g = self.lock.lock().await;
        let c = self.conn.clone();
        let sql = sql.to_string();
        tokio::task::spawn_blocking(move || -> Result<Vec<QueryResult>> {
            let stmts = split(&sql);
            if read_only {
                c.execute("SET TRANSACTION READ ONLY", &[]).map_err(err)?;
            }
            let mut out = vec![];
            let res = (|| {
                for s in &stmts {
                    out.push(Self::run_one(&c, s, limit)?);
                }
                Ok::<_, Error>(())
            })();
            if read_only || res.is_err() {
                let _ = c.rollback();
            } else {
                c.commit().map_err(err)?;
            }
            res?;
            if out.is_empty() {
                out.push(QueryResult::default());
            }
            Ok(out)
        })
        .await
        .map_err(|e| Error::Other(e.to_string()))?
    }

    pub async fn exec_tx(&self, stmts: Vec<(Expect, String)>) -> Result<u64> {
        let _g = self.lock.lock().await;
        let c = self.conn.clone();
        tokio::task::spawn_blocking(move || -> Result<u64> {
            let mut total = 0;
            for (expect, s) in &stmts {
                let n = match c.execute(s.trim_end_matches(';'), &[]) {
                    Ok(st) => st.row_count().unwrap_or(0),
                    Err(e) => {
                        let _ = c.rollback();
                        return Err(err(e));
                    }
                };
                if !expect.check(n) {
                    let _ = c.rollback();
                    return Err(Error::Other(format!(
                        "expected {}, {n} affected — rolled back.\n{s}",
                        expect.describe()
                    )));
                }
                total += n;
            }
            c.commit().map_err(err)?;
            Ok(total)
        })
        .await
        .map_err(|e| Error::Other(e.to_string()))?
    }

    /// Interrupt whatever runs on this connection right now.
    pub fn cancel(&self) -> Result<()> {
        self.conn.break_execution().map_err(err)
    }

    pub async fn ping(&self) -> Result<()> {
        let c = self.conn.clone();
        tokio::task::spawn_blocking(move || c.ping().map_err(err))
            .await
            .map_err(|e| Error::Other(e.to_string()))?
    }

    pub fn close(&self) {
        let _ = self.conn.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_plsql_and_sql() {
        let s = "select 1 from dual;\nselect 'a;b' from dual;\n\
                 CREATE OR REPLACE PROCEDURE p AS\nBEGIN\n  NULL;\nEND;\n/\n\
                 BEGIN\n  p;\nEND;\n/\ninsert into t values (1);\n";
        let v = split(s);
        assert_eq!(v.len(), 5, "{v:?}");
        assert_eq!(v[0], "select 1 from dual");
        assert_eq!(v[1], "select 'a;b' from dual");
        assert!(v[2].starts_with("CREATE OR REPLACE PROCEDURE") && v[2].ends_with("END;"));
        assert!(v[3].starts_with("BEGIN") && v[3].ends_with("END;"));
        assert_eq!(v[4], "insert into t values (1)");
        assert_eq!(
            split("SELECT sysdate FROM dual"),
            ["SELECT sysdate FROM dual"]
        );
    }

    #[test]
    fn connect_strings() {
        assert_eq!(
            connect_string("db.example.com", 1521, Some("FREEPDB1")),
            "//db.example.com:1521/FREEPDB1"
        );
        assert_eq!(connect_string("localhost", 1521, None), "//localhost:1521");
        assert_eq!(connect_string("db:1522/ORCL", 1521, None), "db:1522/ORCL");
        assert_eq!(
            connect_string("(DESCRIPTION=(ADDRESS=(HOST=x)))", 1521, None),
            "(DESCRIPTION=(ADDRESS=(HOST=x)))"
        );
        assert_eq!(connect_string("PRODTNS", 1521, None), "PRODTNS");
        assert_eq!(num("12"), Value::from(12));
        assert_eq!(
            num("12345678901234567890.5"),
            Value::String("12345678901234567890.5".into())
        );
    }
}
