//! Google BigQuery over the REST API (`jobs.query` / `getQueryResults`).
//!
//! Auth: a service-account JSON key (signed RS256 JWT → OAuth access token,
//! refreshed before it expires), or a raw OAuth access token
//! (`gcloud auth print-access-token`). Datasets show up as schemas.

use std::sync::Mutex;

use serde_json::{json, Value};

use super::Expect;
use crate::{guard, ColumnInfo, Error, QueryResult, Result, ResultColumn, TableInfo};

pub const DEFAULT_API: &str = "https://bigquery.googleapis.com/bigquery/v2";
const SCOPE: &str = "https://www.googleapis.com/auth/bigquery";
const PAGE: usize = 10_000;

pub struct Client {
    http: reqwest::Client,
    api: String,
    pub project: String,
    location: Option<String>,
    cred: Cred,
    token: Mutex<Option<(String, i64)>>,
}

enum Cred {
    ServiceAccount {
        email: String,
        key: ring::signature::RsaKeyPair,
        token_uri: String,
    },
    Token(String),
}

fn err(msg: impl std::fmt::Display) -> Error {
    Error::Other(format!("bigquery: {msg}"))
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn b64url(b: &[u8]) -> String {
    crate::sync::base64_encode(b)
        .trim_end_matches('=')
        .replace('+', "-")
        .replace('/', "_")
}

/// PEM (`-----BEGIN PRIVATE KEY-----`) → PKCS#8 DER.
fn pem_der(pem: &str) -> Option<Vec<u8>> {
    let body: String = pem
        .lines()
        .filter(|l| !l.starts_with("-----"))
        .collect::<Vec<_>>()
        .join("");
    super::libsql::base64_decode(&body)
}

/// A signed JWT asking for a BigQuery-scoped token.
pub fn sign_jwt(
    key: &ring::signature::RsaKeyPair,
    email: &str,
    aud: &str,
    iat: i64,
) -> Result<String> {
    let header = b64url(br#"{"alg":"RS256","typ":"JWT"}"#);
    let claims = b64url(
        json!({ "iss": email, "scope": SCOPE, "aud": aud, "iat": iat, "exp": iat + 3600 })
            .to_string()
            .as_bytes(),
    );
    let input = format!("{header}.{claims}");
    let mut sig = vec![0u8; key.public().modulus_len()];
    key.sign(
        &ring::signature::RSA_PKCS1_SHA256,
        &ring::rand::SystemRandom::new(),
        input.as_bytes(),
        &mut sig,
    )
    .map_err(|_| err("couldn't sign the token request"))?;
    Ok(format!("{input}.{}", b64url(&sig)))
}

impl Client {
    /// `key_json`: the service-account key file's contents; `token`: an OAuth
    /// access token instead. `project` defaults to the key's `project_id`.
    pub async fn connect(
        api: Option<&str>,
        project: Option<&str>,
        location: Option<&str>,
        key_json: Option<&str>,
        token: Option<&str>,
    ) -> Result<Self> {
        let (cred, key_project) = match (key_json.filter(|k| !k.trim().is_empty()), token) {
            (Some(k), _) => {
                let v: Value = serde_json::from_str(k)
                    .map_err(|e| err(format!("the key file isn't valid JSON: {e}")))?;
                if v["type"].as_str() != Some("service_account") {
                    return Err(err(
                        "expected a service-account key (\"type\": \"service_account\")",
                    ));
                }
                let der = v["private_key"]
                    .as_str()
                    .and_then(pem_der)
                    .ok_or_else(|| err("the key file has no private_key"))?;
                let key = ring::signature::RsaKeyPair::from_pkcs8(&der)
                    .map_err(|e| err(format!("unreadable private key: {e}")))?;
                (
                    Cred::ServiceAccount {
                        email: v["client_email"].as_str().unwrap_or_default().to_string(),
                        key,
                        token_uri: v["token_uri"]
                            .as_str()
                            .unwrap_or("https://oauth2.googleapis.com/token")
                            .to_string(),
                    },
                    v["project_id"].as_str().map(str::to_string),
                )
            }
            (None, Some(t)) if !t.trim().is_empty() => (Cred::Token(t.trim().to_string()), None),
            _ => {
                return Err(err(
                    "pick a service-account key file, or paste an access token as the password",
                ))
            }
        };
        let project = project
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(str::to_string)
            .or(key_project)
            .ok_or_else(|| err("enter the project ID"))?;
        let c = Self {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(300))
                .build()
                .map_err(err)?,
            api: api
                .map(str::trim)
                .filter(|a| !a.is_empty())
                .unwrap_or(DEFAULT_API)
                .trim_end_matches('/')
                .to_string(),
            project,
            location: location
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_string),
            cred,
            token: Mutex::new(None),
        };
        c.get(&format!("/projects/{}/datasets?maxResults=1", c.project))
            .await?;
        Ok(c)
    }

    async fn access_token(&self) -> Result<String> {
        let (email, key, token_uri) = match &self.cred {
            Cred::Token(t) => return Ok(t.clone()),
            Cred::ServiceAccount {
                email,
                key,
                token_uri,
            } => (email, key, token_uri),
        };
        if let Some((t, exp)) = self.token.lock().unwrap().as_ref() {
            if *exp - 60 > now() {
                return Ok(t.clone());
            }
        }
        let jwt = sign_jwt(key, email, token_uri, now())?;
        let res = self
            .http
            .post(token_uri)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(format!(
                "grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Ajwt-bearer&assertion={jwt}"
            ))
            .send()
            .await
            .map_err(err)?;
        let status = res.status();
        let v: Value = res.json().await.map_err(err)?;
        if !status.is_success() {
            return Err(err(format!(
                "sign-in failed: {}",
                v["error_description"]
                    .as_str()
                    .or_else(|| v["error"].as_str())
                    .unwrap_or("unknown error")
            )));
        }
        let t = v["access_token"]
            .as_str()
            .ok_or_else(|| err("no access token in the sign-in response"))?
            .to_string();
        let exp = now() + v["expires_in"].as_i64().unwrap_or(3600);
        *self.token.lock().unwrap() = Some((t.clone(), exp));
        Ok(t)
    }

    async fn send(&self, req: reqwest::RequestBuilder) -> Result<Value> {
        let res = req
            .bearer_auth(self.access_token().await?)
            .send()
            .await
            .map_err(err)?;
        let status = res.status();
        let text = res.text().await.map_err(err)?;
        let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        if !status.is_success() {
            let msg = v["error"]["message"]
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| text.trim().to_string());
            return Err(err(msg));
        }
        Ok(v)
    }

    async fn get(&self, path: &str) -> Result<Value> {
        self.send(self.http.get(format!("{}{path}", self.api)))
            .await
    }

    /// Run one statement (or script) and collect up to `limit` rows.
    async fn statement(&self, sql: &str, limit: usize) -> Result<(QueryResult, Value)> {
        let mut body = json!({
            "query": sql,
            "useLegacySql": false,
            "maxResults": limit.saturating_add(1).clamp(1, PAGE),
            "timeoutMs": 30_000,
        });
        if let Some(l) = &self.location {
            body["location"] = json!(l);
        }
        let mut page = self
            .send(
                self.http
                    .post(format!("{}/projects/{}/queries", self.api, self.project))
                    .json(&body),
            )
            .await?;
        let job = page["jobReference"]["jobId"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let loc = page["jobReference"]["location"]
            .as_str()
            .map(str::to_string)
            .or_else(|| self.location.clone());
        let results_path = |token: Option<&str>| {
            let mut p = format!(
                "/projects/{}/queries/{job}?timeoutMs=30000&maxResults={}",
                self.project,
                limit.saturating_add(1).clamp(1, PAGE)
            );
            if let Some(l) = &loc {
                p.push_str(&format!("&location={l}"));
            }
            if let Some(t) = token {
                p.push_str(&format!("&pageToken={t}"));
            }
            p
        };
        while page["jobComplete"] == json!(false) {
            page = self.get(&results_path(None)).await?;
        }
        if let Some(e) = page["errors"].as_array().and_then(|a| a.first()) {
            return Err(err(e["message"].as_str().unwrap_or("query failed")));
        }
        let fields = page["schema"]["fields"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let columns = fields
            .iter()
            .map(|f| ResultColumn {
                name: f["name"].as_str().unwrap_or("").to_string(),
                data_type: type_name(f),
            })
            .collect();
        let mut rows: Vec<Vec<Value>> = Vec::new();
        loop {
            if let Some(r) = page["rows"].as_array() {
                rows.extend(r.iter().map(|row| row_values(&fields, row)));
            }
            let token = page["pageToken"].as_str().map(str::to_string);
            match token {
                Some(t) if rows.len() <= limit && !job.is_empty() => {
                    page = self.get(&results_path(Some(&t))).await?;
                }
                _ => break,
            }
        }
        let truncated = rows.len() > limit;
        rows.truncate(limit);
        let affected = page["numDmlAffectedRows"]
            .as_str()
            .and_then(|n| n.parse().ok())
            .unwrap_or(0);
        Ok((
            QueryResult {
                columns,
                rows,
                rows_affected: affected,
                truncated,
            },
            page,
        ))
    }

    pub async fn run(&self, sql: &str, limit: usize, read_only: bool) -> Result<Vec<QueryResult>> {
        if read_only {
            guard::check_read_only(sql).map_err(Error::ReadOnly)?;
        }
        let mut out = Vec::new();
        for stmt in guard::split_statements(sql) {
            out.push(self.statement(&stmt, limit).await?.0);
        }
        Ok(out)
    }

    /// One multi-statement transaction; `ASSERT @@row_count` checks each
    /// statement, and a failed assertion rolls the whole thing back.
    pub async fn exec_tx(&self, stmts: Vec<(Expect, String)>) -> Result<u64> {
        let mut script = String::from("BEGIN TRANSACTION;\n");
        for (expect, s) in &stmts {
            script.push_str(s.trim().trim_end_matches(';'));
            script.push_str(";\n");
            match expect {
                Expect::Any => {}
                Expect::One => script.push_str(
                    "ASSERT @@row_count = 1 AS 'expected 1 row to change — rolled back';\n",
                ),
                Expect::AtMostOne => script.push_str(
                    "ASSERT @@row_count <= 1 AS 'expected at most 1 row to change — rolled back';\n",
                ),
            }
        }
        script.push_str("COMMIT TRANSACTION;");
        self.statement(&script, 0).await?;
        Ok(stmts.len() as u64)
    }

    pub async fn datasets(&self) -> Result<Vec<String>> {
        let mut out = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let mut p = format!("/projects/{}/datasets?maxResults=1000", self.project);
            if let Some(t) = &token {
                p.push_str(&format!("&pageToken={t}"));
            }
            let v = self.get(&p).await?;
            for d in v["datasets"].as_array().into_iter().flatten() {
                if let Some(id) = d["datasetReference"]["datasetId"].as_str() {
                    out.push(id.to_string());
                }
            }
            token = v["nextPageToken"].as_str().map(str::to_string);
            if token.is_none() {
                break;
            }
        }
        out.sort();
        Ok(out)
    }

    pub async fn tables(&self) -> Result<Vec<TableInfo>> {
        let mut out = Vec::new();
        for ds in self.datasets().await? {
            let mut token: Option<String> = None;
            loop {
                let mut p = format!(
                    "/projects/{}/datasets/{ds}/tables?maxResults=1000",
                    self.project
                );
                if let Some(t) = &token {
                    p.push_str(&format!("&pageToken={t}"));
                }
                let v = self.get(&p).await?;
                for t in v["tables"].as_array().into_iter().flatten() {
                    let ty = t["type"].as_str().unwrap_or("TABLE");
                    out.push(TableInfo {
                        schema: ds.clone(),
                        name: t["tableReference"]["tableId"]
                            .as_str()
                            .unwrap_or("")
                            .to_string(),
                        kind: if ty.contains("VIEW") { "view" } else { "table" }.into(),
                    });
                }
                token = v["nextPageToken"].as_str().map(str::to_string);
                if token.is_none() || out.len() > 20_000 {
                    break;
                }
            }
        }
        Ok(out)
    }

    pub async fn describe(&self, dataset: &str, table: &str) -> Result<Vec<ColumnInfo>> {
        let v = self
            .get(&format!(
                "/projects/{}/datasets/{dataset}/tables/{table}",
                self.project
            ))
            .await?;
        Ok(v["schema"]["fields"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|f| ColumnInfo {
                name: f["name"].as_str().unwrap_or("").to_string(),
                data_type: type_name(f),
                nullable: f["mode"].as_str() != Some("REQUIRED"),
                default: f["defaultValueExpression"].as_str().map(str::to_string),
                primary_key: false,
            })
            .collect())
    }
}

/// `INT64`, `ARRAY<STRING>`, `STRUCT<a INT64, b STRING>` …
fn type_name(f: &Value) -> String {
    let base = match f["type"].as_str().unwrap_or("") {
        "INTEGER" => "INT64".to_string(),
        "FLOAT" => "FLOAT64".to_string(),
        "BOOLEAN" => "BOOL".to_string(),
        "RECORD" | "STRUCT" => format!(
            "STRUCT<{}>",
            f["fields"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|c| format!("{} {}", c["name"].as_str().unwrap_or(""), type_name(c)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        other => other.to_string(),
    };
    if f["mode"].as_str() == Some("REPEATED") {
        format!("ARRAY<{base}>")
    } else {
        base
    }
}

fn row_values(fields: &[Value], row: &Value) -> Vec<Value> {
    let cells = row["f"].as_array().cloned().unwrap_or_default();
    fields
        .iter()
        .zip(cells)
        .map(|(f, c)| cell(f, &c["v"]))
        .collect()
}

/// BigQuery's REST rows are strings; turn them back into JSON types.
pub fn cell(f: &Value, v: &Value) -> Value {
    if v.is_null() {
        return Value::Null;
    }
    if f["mode"].as_str() == Some("REPEATED") {
        let mut one = f.clone();
        one["mode"] = json!("NULLABLE");
        return Value::Array(
            v.as_array()
                .into_iter()
                .flatten()
                .map(|x| cell(&one, &x["v"]))
                .collect(),
        );
    }
    let s = v.as_str();
    match f["type"].as_str().unwrap_or("") {
        "INTEGER" | "INT64" => s
            .and_then(|s| s.parse::<i64>().ok())
            // keep big values exact for JS
            .filter(|n| n.unsigned_abs() < (1u64 << 53))
            .map(Value::from)
            .unwrap_or_else(|| v.clone()),
        "FLOAT" | "FLOAT64" => s
            .and_then(|s| s.parse::<f64>().ok())
            .and_then(serde_json::Number::from_f64)
            .map(Value::Number)
            .unwrap_or_else(|| v.clone()),
        "BOOLEAN" | "BOOL" => s
            .map(|s| Value::Bool(s == "true"))
            .unwrap_or_else(|| v.clone()),
        "TIMESTAMP" => s
            .and_then(|s| s.parse::<f64>().ok())
            .and_then(|secs| {
                let micros = (secs * 1e6).round() as i64;
                chrono::DateTime::from_timestamp_micros(micros)
            })
            .map(|t| Value::String(t.format("%Y-%m-%d %H:%M:%S%.f UTC").to_string()))
            .unwrap_or_else(|| v.clone()),
        "RECORD" | "STRUCT" => {
            let sub = f["fields"].as_array().cloned().unwrap_or_default();
            let vals = row_values(&sub, v);
            Value::Object(
                sub.iter()
                    .zip(vals)
                    .map(|(c, x)| (c["name"].as_str().unwrap_or("").to_string(), x))
                    .collect(),
            )
        }
        _ => v.clone(),
    }
}
