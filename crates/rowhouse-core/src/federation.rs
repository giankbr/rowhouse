//! Cross-database queries: a DuckDB connection with other saved connections
//! and files attached, so one query can join Postgres, MySQL, SQLite, DuckDB
//! files and CSV / Parquet / JSON.
//!
//! A "workspace" is an ordinary DuckDB connection profile with `attach`
//! entries. Databases attach read-only under their alias (`pg.public.users`);
//! files become views (`SELECT * FROM sales` for `sales` → `~/data/*.parquet`).
//! DuckDB downloads its postgres / mysql / sqlite / parquet extensions the first
//! time they're needed (needs internet once).

use serde::{Deserialize, Serialize};

use crate::{config::Driver, ConnectionConfig, DbKind, Error, Result};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    /// name to use in queries: a catalog (databases) or a view (files)
    pub alias: String,
    /// a saved connection (Postgres / MySQL family, SQLite, DuckDB)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection_id: Option<String>,
    /// or a file / glob: .csv .tsv .parquet .json .ndjson .duckdb .db .sqlite
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

fn err(msg: impl std::fmt::Display) -> Error {
    Error::Other(msg.to_string())
}

/// `alias` must be a plain identifier (it's used unquoted in queries).
pub fn check_alias(a: &str) -> Result<()> {
    let ok = !a.is_empty()
        && a.chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && a.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !ok {
        return Err(err(format!(
            "`{a}` isn't a valid alias — use letters, digits and _ (e.g. prod_pg)"
        )));
    }
    if ["main", "memory", "system", "temp"].contains(&a.to_lowercase().as_str()) {
        return Err(err(format!(
            "`{a}` is reserved by DuckDB — pick another alias"
        )));
    }
    Ok(())
}

fn lit(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// libpq / MySQL key=value strings: quote values with spaces or quotes.
fn kv(k: &str, v: &str) -> String {
    if v.is_empty() || v.contains([' ', '\'', '\\']) {
        format!("{k}='{}'", v.replace('\\', "\\\\").replace('\'', "\\'"))
    } else {
        format!("{k}={v}")
    }
}

/// Statements that attach one connection. `host`/`port` are what to dial
/// (the SSH tunnel's local end when there is one).
pub fn attach_connection_sql(
    alias: &str,
    cfg: &ConnectionConfig,
    password: Option<&str>,
    host: &str,
    port: u16,
) -> Result<Vec<String>> {
    check_alias(alias)?;
    let user = cfg.user.as_deref().unwrap_or("");
    let db = cfg.database.as_deref().unwrap_or("");
    Ok(match cfg.kind.driver() {
        Driver::Postgres if cfg.kind != DbKind::Redshift => {
            let mut parts = vec![kv("host", host), kv("port", &port.to_string())];
            if !db.is_empty() {
                parts.push(kv("dbname", db));
            }
            if !user.is_empty() {
                parts.push(kv("user", user));
            }
            if let Some(p) = password.filter(|p| !p.is_empty()) {
                parts.push(kv("password", p));
            }
            let ssl = match cfg.tls.as_ref().map(|t| t.mode) {
                Some(crate::config::TlsMode::Disable) => "disable",
                Some(crate::config::TlsMode::Require) => "require",
                Some(crate::config::TlsMode::VerifyCa) => "verify-ca",
                Some(crate::config::TlsMode::VerifyFull) => "verify-full",
                _ => "prefer",
            };
            parts.push(kv("sslmode", ssl));
            vec![
                "INSTALL postgres".into(),
                "LOAD postgres".into(),
                format!(
                    "ATTACH {} AS {alias} (TYPE postgres, READ_ONLY)",
                    lit(&parts.join(" "))
                ),
            ]
        }
        Driver::Mysql => {
            let mut parts = vec![kv("host", host), kv("port", &port.to_string())];
            if !db.is_empty() {
                parts.push(kv("database", db));
            }
            if !user.is_empty() {
                parts.push(kv("user", user));
            }
            if let Some(p) = password.filter(|p| !p.is_empty()) {
                parts.push(kv("password", p));
            }
            vec![
                "INSTALL mysql".into(),
                "LOAD mysql".into(),
                format!(
                    "ATTACH {} AS {alias} (TYPE mysql, READ_ONLY)",
                    lit(&parts.join(" "))
                ),
            ]
        }
        Driver::Sqlite if cfg.kind == DbKind::Sqlite => {
            let f = cfg
                .file
                .as_deref()
                .ok_or_else(|| err("that SQLite profile has no file"))?;
            vec![
                "INSTALL sqlite".into(),
                "LOAD sqlite".into(),
                format!(
                    "ATTACH {} AS {alias} (TYPE sqlite, READ_ONLY)",
                    lit(&crate::tunnel::expand_home(f))
                ),
            ]
        }
        Driver::Duckdb if cfg.attach.is_empty() => {
            let f = cfg
                .file
                .as_deref()
                .ok_or_else(|| err("that DuckDB profile has no file"))?;
            vec![format!(
                "ATTACH {} AS {alias} (READ_ONLY)",
                lit(&crate::tunnel::expand_home(f))
            )]
        }
        _ => {
            return Err(err(format!(
                "{} can't be attached yet — Postgres, MySQL/MariaDB, SQLite and DuckDB can",
                cfg.name
            )))
        }
    })
}

/// Statements for a file attachment.
pub fn attach_file_sql(alias: &str, path: &str) -> Result<Vec<String>> {
    check_alias(alias)?;
    let p = crate::tunnel::expand_home(path.trim());
    let lower = p.to_lowercase();
    Ok(if lower.ends_with(".duckdb") || lower.ends_with(".ddb") {
        vec![format!("ATTACH {} AS {alias} (READ_ONLY)", lit(&p))]
    } else if lower.ends_with(".sqlite") || lower.ends_with(".sqlite3") || lower.ends_with(".db") {
        vec![
            "INSTALL sqlite".into(),
            "LOAD sqlite".into(),
            format!("ATTACH {} AS {alias} (TYPE sqlite, READ_ONLY)", lit(&p)),
        ]
    } else {
        let reader = if lower.ends_with(".parquet") {
            "read_parquet"
        } else if lower.ends_with(".json")
            || lower.ends_with(".ndjson")
            || lower.ends_with(".jsonl")
        {
            "read_json_auto"
        } else if lower.ends_with(".csv") || lower.ends_with(".tsv") || lower.ends_with(".txt") {
            "read_csv_auto"
        } else {
            return Err(err(format!(
                    "don't know how to read {path} — use .csv, .tsv, .parquet, .json, .duckdb or .sqlite"
                )));
        };
        vec![format!(
            "CREATE OR REPLACE VIEW {alias} AS SELECT * FROM {reader}({})",
            lit(&p)
        )]
    })
}

/// A friendlier message when DuckDB can't fetch an extension.
pub fn explain_error(e: &str) -> String {
    if e.contains("Failed to download extension") {
        let ext = e
            .split("extension \"")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .unwrap_or("an extension");
        format!(
            "DuckDB needs to download its {ext} extension once (from extensions.duckdb.org) — check your internet connection or proxy, then try again"
        )
    } else {
        e.to_string()
    }
}
