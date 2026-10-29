//! DuckDB (embedded, file based). Blocking API → run on the blocking pool.

use std::sync::{Arc, Mutex};

use chrono::{DateTime, NaiveDate, NaiveTime};
use duckdb::types::{TimeUnit, Value as DV};
use serde_json::{Number, Value};

use super::Expect;
use crate::{guard, Error, QueryResult, Result, ResultColumn};

#[derive(Clone)]
pub struct Client {
    conn: Arc<Mutex<duckdb::Connection>>,
}

fn err(e: duckdb::Error) -> Error {
    Error::Other(format!("duckdb: {e}"))
}

/// Statements that produce rows (vs. DDL / DML that report a change count).
fn returns_rows(stmt: &str) -> bool {
    matches!(
        guard::first_keyword(stmt).as_str(),
        "select"
            | "with"
            | "show"
            | "describe"
            | "desc"
            | "explain"
            | "pragma"
            | "values"
            | "from"
            | "summarize"
            | "table"
            | "call"
    )
}

fn micros_to_string(us: i64) -> String {
    DateTime::from_timestamp_micros(us)
        .map(|d| d.naive_utc().to_string())
        .unwrap_or_else(|| us.to_string())
}

fn to_json(v: DV) -> Value {
    match v {
        DV::Null => Value::Null,
        DV::Boolean(b) => Value::Bool(b),
        DV::TinyInt(i) => i.into(),
        DV::SmallInt(i) => i.into(),
        DV::Int(i) => i.into(),
        DV::BigInt(i) => i.into(),
        DV::UTinyInt(i) => i.into(),
        DV::USmallInt(i) => i.into(),
        DV::UInt(i) => i.into(),
        DV::UBigInt(i) => i.into(),
        DV::HugeInt(i) => i64::try_from(i)
            .map(Value::from)
            .unwrap_or_else(|_| Value::String(i.to_string())),
        DV::UHugeInt(i) => u64::try_from(i)
            .map(Value::from)
            .unwrap_or_else(|_| Value::String(i.to_string())),
        DV::Float(f) => Number::from_f64(f as f64)
            .map(Value::Number)
            .unwrap_or(Value::String(f.to_string())),
        DV::Double(f) => Number::from_f64(f)
            .map(Value::Number)
            .unwrap_or(Value::String(f.to_string())),
        DV::Decimal(d) => Value::String(d.to_string()),
        DV::Text(s) | DV::Enum(s) => Value::String(s),
        DV::Blob(b) | DV::Geometry(b) => Value::String(format!(
            "0x{}",
            b.iter().map(|x| format!("{x:02x}")).collect::<String>()
        )),
        DV::Timestamp(unit, v) => Value::String(micros_to_string(unit.to_micros(v))),
        DV::Date32(days) => Value::String(
            NaiveDate::from_ymd_opt(1970, 1, 1)
                .and_then(|d| d.checked_add_signed(chrono::Duration::days(days as i64)))
                .map(|d| d.to_string())
                .unwrap_or_else(|| days.to_string()),
        ),
        DV::Time64(unit, v) => {
            let us = TimeUnit::to_micros(&unit, v);
            Value::String(
                NaiveTime::from_num_seconds_from_midnight_opt(
                    (us / 1_000_000) as u32,
                    ((us % 1_000_000) * 1000) as u32,
                )
                .map(|t| t.to_string())
                .unwrap_or_else(|| us.to_string()),
            )
        }
        DV::Interval {
            months,
            days,
            nanos,
        } => Value::String(format!("{months} months {days} days {}us", nanos / 1000)),
        DV::List(items) | DV::Array(items) => {
            Value::Array(items.into_iter().map(to_json).collect())
        }
        DV::Struct(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), to_json(v.clone())))
                .collect(),
        ),
        DV::Map(map) => Value::Array(
            map.iter()
                .map(|(k, v)| Value::Array(vec![to_json(k.clone()), to_json(v.clone())]))
                .collect(),
        ),
        DV::Union(v) => to_json(*v),
        other => Value::String(format!("{other:?}")),
    }
}

impl Client {
    pub async fn open(path: &str, read_only: bool) -> Result<Self> {
        let path = path.to_string();
        let conn = tokio::task::spawn_blocking(move || {
            let mut cfg = duckdb::Config::default();
            if read_only {
                cfg = cfg.access_mode(duckdb::AccessMode::ReadOnly).map_err(err)?;
            }
            duckdb::Connection::open_with_flags(&path, cfg).map_err(err)
        })
        .await
        .map_err(|e| Error::Other(e.to_string()))??;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    fn run_sync(conn: &duckdb::Connection, sql: &str, limit: usize) -> Result<Vec<QueryResult>> {
        let mut out = Vec::new();
        for stmt_sql in guard::split_statements(sql) {
            let mut stmt = conn.prepare(&stmt_sql).map_err(err)?;
            if !returns_rows(&stmt_sql) {
                let n = stmt.execute([]).map_err(err)?;
                out.push(QueryResult {
                    rows_affected: n as u64,
                    ..Default::default()
                });
                continue;
            }
            let mut rows = stmt.query([]).map_err(err)?;
            let mut result = QueryResult::default();
            let mut cols: Option<usize> = None;
            while let Some(row) = rows.next().map_err(err)? {
                let s = row.as_ref();
                let n = *cols.get_or_insert_with(|| {
                    result.columns = (0..s.column_count())
                        .map(|i| ResultColumn {
                            name: s.column_name(i).cloned().unwrap_or_default(),
                            data_type: s.column_type(i).to_string(),
                        })
                        .collect();
                    s.column_count()
                });
                if result.rows.len() >= limit {
                    result.truncated = true;
                    break;
                }
                result.rows.push(
                    (0..n)
                        .map(|i| to_json(row.get::<_, DV>(i).unwrap_or(DV::Null)))
                        .collect(),
                );
            }
            drop(rows);
            if cols.is_none() {
                // empty result: still report the column names
                result.columns = stmt
                    .column_names()
                    .into_iter()
                    .enumerate()
                    .map(|(i, name)| ResultColumn {
                        name,
                        data_type: stmt.column_type(i).to_string(),
                    })
                    .collect();
            }
            out.push(result);
        }
        Ok(out)
    }

    pub async fn run(&self, sql: &str, limit: usize, read_only: bool) -> Result<Vec<QueryResult>> {
        let conn = self.conn.clone();
        let sql = sql.to_string();
        tokio::task::spawn_blocking(move || {
            let c = conn.lock().unwrap();
            if read_only {
                // writes inside the transaction are discarded
                c.execute_batch("BEGIN TRANSACTION").map_err(err)?;
                let r = Self::run_sync(&c, &sql, limit);
                c.execute_batch("ROLLBACK").map_err(err)?;
                r
            } else {
                Self::run_sync(&c, &sql, limit)
            }
        })
        .await
        .map_err(|e| Error::Other(e.to_string()))?
    }

    pub async fn exec_tx(&self, stmts: Vec<(Expect, String)>) -> Result<u64> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let c = conn.lock().unwrap();
            c.execute_batch("BEGIN TRANSACTION").map_err(err)?;
            let mut total = 0u64;
            for (expect, s) in &stmts {
                let n = match c.execute(s, []) {
                    Ok(n) => n as u64,
                    Err(e) => {
                        let _ = c.execute_batch("ROLLBACK");
                        return Err(err(e));
                    }
                };
                if !expect.check(n) {
                    let _ = c.execute_batch("ROLLBACK");
                    return Err(Error::Other(format!(
                        "expected 1 row, {n} affected — rolled back.\n{s}"
                    )));
                }
                total += n;
            }
            c.execute_batch("COMMIT").map_err(err)?;
            Ok(total)
        })
        .await
        .map_err(|e| Error::Other(e.to_string()))?
    }
}
