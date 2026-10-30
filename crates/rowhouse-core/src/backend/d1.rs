//! Cloudflare D1 over the REST API (`/accounts/{account}/d1/database/{id}/raw`).
//! SQLite dialect. A request with several statements runs as one D1 batch,
//! which D1 executes as a single transaction.

use serde_json::{json, Value};

use super::{libsql::changes_guard, Expect};
use crate::{guard, Error, QueryResult, Result, ResultColumn};

pub const DEFAULT_API: &str = "https://api.cloudflare.com/client/v4";

pub struct Client {
    http: reqwest::Client,
    endpoint: String,
    token: String,
}

fn err(msg: impl std::fmt::Display) -> Error {
    Error::Other(format!("d1: {msg}"))
}

impl Client {
    pub async fn connect(
        api: Option<&str>,
        account: &str,
        database: &str,
        token: &str,
    ) -> Result<Self> {
        if account.trim().is_empty() || database.trim().is_empty() {
            return Err(err("account ID and database ID are required"));
        }
        if token.trim().is_empty() {
            return Err(err("an API token with D1 permissions is required"));
        }
        let api = api
            .filter(|a| !a.trim().is_empty())
            .unwrap_or(DEFAULT_API)
            .trim_end_matches('/');
        let c = Self {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .map_err(err)?,
            endpoint: format!(
                "{api}/accounts/{}/d1/database/{}/raw",
                account.trim(),
                database.trim()
            ),
            token: token.trim().to_string(),
        };
        c.raw("SELECT 1").await?;
        Ok(c)
    }

    async fn raw(&self, sql: &str) -> Result<Vec<Value>> {
        let resp = self
            .http
            .post(&self.endpoint)
            .bearer_auth(&self.token)
            .json(&json!({ "sql": sql }))
            .send()
            .await
            .map_err(err)?;
        let status = resp.status();
        let body: Value = resp
            .json()
            .await
            .map_err(|e| err(format!("HTTP {status}: {e}")))?;
        if !body
            .get("success")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            let msgs: Vec<String> = body
                .get("errors")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .map(|e| {
                            e.get("message")
                                .and_then(Value::as_str)
                                .map(str::to_string)
                                .unwrap_or_else(|| e.to_string())
                        })
                        .collect()
                })
                .unwrap_or_default();
            return Err(err(if msgs.is_empty() {
                format!("HTTP {status}")
            } else {
                msgs.join("; ")
            }));
        }
        Ok(body
            .get("result")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default())
    }

    pub async fn run(&self, sql: &str, limit: usize, read_only: bool) -> Result<Vec<QueryResult>> {
        let stmts = guard::split_statements(sql);
        if stmts.is_empty() {
            return Ok(vec![]);
        }
        if read_only {
            // D1 has no read-only transactions over the API; the keyword guard
            // (already checked by the caller) is the only layer here.
        }
        let script = stmts.join(";\n");
        let results = self.raw(&script).await?;
        Ok(results.iter().map(|r| to_result(r, limit)).collect())
    }

    /// One batch (= one D1 transaction). Keyed statements are followed by a guard
    /// that fails the batch when the row count is wrong, which rolls it all back.
    pub async fn exec_tx(&self, stmts: Vec<(Expect, String)>) -> Result<u64> {
        let mut script = Vec::new();
        let mut positions = Vec::new();
        for (e, s) in &stmts {
            positions.push(script.len());
            script.push(s.clone());
            if let Some(g) = changes_guard(*e) {
                script.push(g.to_string());
            }
        }
        let results = self.raw(&script.join(";\n")).await.map_err(|e| {
            let m = e.to_string();
            if m.contains("integer overflow") {
                Error::Other(
                    "expected 1 row, a different number would be affected — rolled back".into(),
                )
            } else {
                e
            }
        })?;
        Ok(positions
            .iter()
            .filter_map(|&i| results.get(i))
            .map(|r| {
                r.get("meta")
                    .and_then(|m| m.get("changes"))
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
            })
            .sum())
    }
}

fn to_result(r: &Value, limit: usize) -> QueryResult {
    let res = r.get("results");
    let columns: Vec<ResultColumn> = res
        .and_then(|x| x.get("columns"))
        .and_then(Value::as_array)
        .map(|c| {
            c.iter()
                .map(|n| ResultColumn {
                    name: n.as_str().unwrap_or("").to_string(),
                    data_type: String::new(),
                })
                .collect()
        })
        .unwrap_or_default();
    let rows: Vec<Vec<Value>> = res
        .and_then(|x| x.get("rows"))
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .map(|row| row.as_array().cloned().unwrap_or_default())
                .collect()
        })
        .unwrap_or_default();
    let truncated = rows.len() > limit;
    QueryResult {
        columns,
        rows: rows.into_iter().take(limit).collect(),
        rows_affected: r
            .get("meta")
            .and_then(|m| m.get("changes"))
            .and_then(Value::as_u64)
            .unwrap_or(0),
        truncated,
    }
}
