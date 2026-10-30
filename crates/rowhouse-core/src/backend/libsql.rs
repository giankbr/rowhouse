//! LibSQL / Turso over HTTP (Hrana v2 pipeline API). SQLite dialect.
//!
//! Scripts run statement by statement inside one batch; writes from grid edits
//! run as BEGIN … COMMIT with a guard after each keyed statement, so a stale
//! grid rolls the whole batch back.

use serde_json::{json, Value};

use super::Expect;
use crate::{guard, Error, QueryResult, Result, ResultColumn};

pub struct Client {
    http: reqwest::Client,
    url: String,
    token: Option<String>,
}

fn err(msg: impl std::fmt::Display) -> Error {
    Error::Other(format!("libsql: {msg}"))
}

/// Fails the batch (integer overflow) unless the previous statement changed the
/// expected number of rows.
pub(crate) fn changes_guard(e: Expect) -> Option<&'static str> {
    match e {
        Expect::Any => None,
        Expect::One => {
            Some("SELECT CASE WHEN changes() = 1 THEN 1 ELSE abs(-9223372036854775808) END")
        }
        Expect::AtMostOne => {
            Some("SELECT CASE WHEN changes() <= 1 THEN 1 ELSE abs(-9223372036854775808) END")
        }
    }
}

impl Client {
    /// `url`: `libsql://db-org.turso.io`, `https://…`, or `http://127.0.0.1:8080` (sqld).
    pub async fn connect(url: &str, token: Option<&str>) -> Result<Self> {
        let url = url.trim().trim_end_matches('/');
        if url.is_empty() {
            return Err(err("database URL is empty"));
        }
        let url = if let Some(rest) = url.strip_prefix("libsql://") {
            format!("https://{rest}")
        } else if url.starts_with("http://") || url.starts_with("https://") {
            url.to_string()
        } else {
            format!("https://{url}")
        };
        let c = Self {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .map_err(err)?,
            url,
            token: token.filter(|t| !t.is_empty()).map(str::to_string),
        };
        c.run("SELECT 1", 1, false).await?;
        Ok(c)
    }

    async fn pipeline(&self, requests: Vec<Value>) -> Result<Vec<Value>> {
        let mut req = self
            .http
            .post(format!("{}/v2/pipeline", self.url))
            .json(&json!({ "baton": null, "requests": requests }));
        if let Some(t) = &self.token {
            req = req.bearer_auth(t);
        }
        let resp = req.send().await.map_err(err)?;
        let status = resp.status();
        let body: Value = resp.json().await.map_err(err)?;
        if !status.is_success() {
            let msg = body
                .get("error")
                .and_then(|e| {
                    e.as_str()
                        .map(str::to_string)
                        .or_else(|| e.get("message").and_then(Value::as_str).map(str::to_string))
                })
                .or_else(|| {
                    body.get("message")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .unwrap_or_else(|| body.to_string());
            return Err(err(format!("HTTP {status}: {msg}")));
        }
        let results = body
            .get("results")
            .and_then(Value::as_array)
            .cloned()
            .ok_or_else(|| err("unexpected response"))?;
        for r in &results {
            if r.get("type").and_then(Value::as_str) == Some("error") {
                return Err(err(error_message(r.get("error"))));
            }
        }
        Ok(results)
    }

    /// One batch: steps run in order; `cond(i)` decides each step's condition.
    async fn batch(&self, steps: Vec<Value>) -> Result<(Vec<Option<Value>>, Vec<Option<String>>)> {
        let res = self
            .pipeline(vec![
                json!({ "type": "batch", "batch": { "steps": steps } }),
                json!({ "type": "close" }),
            ])
            .await?;
        let r = res
            .first()
            .and_then(|r| r.get("response"))
            .and_then(|r| r.get("result"))
            .ok_or_else(|| err("unexpected batch response"))?;
        let results = r
            .get("step_results")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .map(|v| (!v.is_null()).then(|| v.clone()))
                    .collect()
            })
            .unwrap_or_default();
        let errors = r
            .get("step_errors")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .map(|v| (!v.is_null()).then(|| error_message(Some(v))))
                    .collect()
            })
            .unwrap_or_default();
        Ok((results, errors))
    }

    /// Run a script. Read-only scripts run inside BEGIN … ROLLBACK, so nothing sticks
    /// even if the keyword guard missed something.
    pub async fn run(&self, sql: &str, limit: usize, read_only: bool) -> Result<Vec<QueryResult>> {
        let stmts = guard::split_statements(sql);
        if stmts.is_empty() {
            return Ok(vec![]);
        }
        let mut steps = Vec::new();
        let offset = usize::from(read_only);
        if read_only {
            steps.push(json!({ "stmt": { "sql": "BEGIN", "want_rows": false } }));
        }
        for (i, s) in stmts.iter().enumerate() {
            let mut step = json!({ "stmt": { "sql": s, "want_rows": true } });
            if i + offset > 0 {
                // stop at the first error, like other drivers
                step["condition"] = json!({ "type": "ok", "step": i + offset - 1 });
            }
            steps.push(step);
        }
        if read_only {
            steps.push(json!({ "stmt": { "sql": "ROLLBACK", "want_rows": false } }));
        }
        let (results, errors) = self.batch(steps).await?;
        let mut out = Vec::new();
        for i in 0..stmts.len() {
            let k = i + offset;
            if let Some(Some(e)) = errors.get(k) {
                return Err(err(e));
            }
            if let Some(Some(r)) = results.get(k) {
                out.push(stmt_result(r, limit));
            }
        }
        Ok(out)
    }

    /// Statements in one transaction; each one's affected-row count must satisfy its [`Expect`].
    pub async fn exec_tx(&self, stmts: Vec<(Expect, String)>) -> Result<u64> {
        let mut steps = vec![json!({ "stmt": { "sql": "BEGIN", "want_rows": false } })];
        // (step index of the statement, its sql)
        let mut watched = Vec::new();
        for (e, s) in &stmts {
            let prev = steps.len() - 1;
            steps.push(json!({ "stmt": { "sql": s, "want_rows": false }, "condition": { "type": "ok", "step": prev } }));
            watched.push((steps.len() - 1, s.clone(), *e));
            if let Some(g) = changes_guard(*e) {
                let prev = steps.len() - 1;
                steps.push(json!({ "stmt": { "sql": g, "want_rows": false }, "condition": { "type": "ok", "step": prev } }));
            }
        }
        let last = steps.len() - 1;
        steps.push(json!({ "stmt": { "sql": "COMMIT", "want_rows": false }, "condition": { "type": "ok", "step": last } }));
        let commit = steps.len() - 1;
        steps.push(json!({
            "stmt": { "sql": "ROLLBACK", "want_rows": false },
            "condition": { "type": "not", "cond": { "type": "ok", "step": commit } }
        }));
        let (results, errors) = self.batch(steps).await?;
        let mut total = 0;
        for (idx, sql, expect) in &watched {
            if let Some(Some(e)) = errors.get(*idx) {
                return Err(err(format!("{e} — rolled back.\n{sql}")));
            }
            if let Some(Some(e)) = errors.get(idx + 1).filter(|_| *expect != Expect::Any) {
                let n = results
                    .get(*idx)
                    .and_then(|r| r.as_ref())
                    .and_then(|r| r.get("affected_row_count"))
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let _ = e;
                return Err(Error::Other(format!(
                    "expected 1 row, {n} affected — rolled back.\n{sql}"
                )));
            }
            total += results
                .get(*idx)
                .and_then(|r| r.as_ref())
                .and_then(|r| r.get("affected_row_count"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
        }
        if let Some(Some(e)) = errors.get(commit) {
            return Err(err(format!("commit failed: {e}")));
        }
        Ok(total)
    }
}

fn error_message(e: Option<&Value>) -> String {
    match e {
        Some(Value::String(s)) => s.clone(),
        Some(v) => v
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| v.to_string()),
        None => "unknown error".into(),
    }
}

/// Hrana value → JSON for the grid.
pub fn value(v: &Value) -> Value {
    match v.get("type").and_then(Value::as_str) {
        Some("integer") => match v.get("value") {
            Some(Value::String(s)) => s
                .parse::<i64>()
                .map(Value::from)
                .unwrap_or_else(|_| Value::String(s.clone())),
            Some(other) => other.clone(),
            None => Value::Null,
        },
        Some("float") => v.get("value").cloned().unwrap_or(Value::Null),
        Some("text") => v.get("value").cloned().unwrap_or(Value::Null),
        Some("blob") => {
            let b64 = v.get("base64").and_then(Value::as_str).unwrap_or("");
            match base64_decode(b64) {
                Some(bytes) => Value::String(format!(
                    "0x{}",
                    bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
                )),
                None => Value::String(b64.into()),
            }
        }
        _ => Value::Null,
    }
}

fn stmt_result(r: &Value, limit: usize) -> QueryResult {
    let columns: Vec<ResultColumn> = r
        .get("cols")
        .and_then(Value::as_array)
        .map(|cols| {
            cols.iter()
                .map(|c| ResultColumn {
                    name: c
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    data_type: c
                        .get("decltype")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                })
                .collect()
        })
        .unwrap_or_default();
    let rows: Vec<Vec<Value>> = r
        .get("rows")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .map(|row| {
                    row.as_array()
                        .map(|cells| cells.iter().map(value).collect())
                        .unwrap_or_default()
                })
                .collect()
        })
        .unwrap_or_default();
    let truncated = rows.len() > limit;
    QueryResult {
        columns,
        rows: rows.into_iter().take(limit).collect(),
        rows_affected: r
            .get("affected_row_count")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        truncated,
    }
}

/// Small standard base64 decoder (no extra dependency for one blob format).
pub fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        } as u32)
    };
    let bytes: Vec<u8> = s
        .bytes()
        .filter(|c| !c.is_ascii_whitespace() && *c != b'=')
        .collect();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    for chunk in bytes.chunks(4) {
        let mut acc = 0u32;
        for (i, &c) in chunk.iter().enumerate() {
            acc |= val(c)? << (18 - 6 * i);
        }
        out.push((acc >> 16) as u8);
        if chunk.len() > 2 {
            out.push((acc >> 8) as u8);
        }
        if chunk.len() > 3 {
            out.push(acc as u8);
        }
    }
    Some(out)
}
