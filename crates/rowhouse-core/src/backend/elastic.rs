//! Elasticsearch and OpenSearch through their SQL endpoints.
//!
//! - Elasticsearch: `POST /_sql?format=json` → `{columns, rows, cursor}`
//! - OpenSearch: `POST /_plugins/_sql` → `{schema, datarows, cursor}`
//!
//! Indices (and aliases) show up as tables. SQL there is read-only by design,
//! so writes are refused before they reach the server.

use serde_json::{json, Value};

use crate::{guard, Error, QueryResult, Result, ResultColumn, TableInfo};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flavor {
    Elasticsearch,
    OpenSearch,
}

pub struct Client {
    http: reqwest::Client,
    base: String,
    auth: Auth,
    pub flavor: Flavor,
}

enum Auth {
    None,
    Basic(String, String),
    ApiKey(String),
}

fn err(msg: impl std::fmt::Display) -> Error {
    Error::Other(format!("elasticsearch: {msg}"))
}

/// Elasticsearch SQL doesn't take a trailing `;`.
fn clean(sql: &str) -> String {
    sql.trim().trim_end_matches(';').trim().to_string()
}

impl Client {
    pub async fn connect(
        host: &str,
        port: u16,
        user: Option<&str>,
        password: Option<&str>,
        tls: Option<&crate::config::TlsConfig>,
    ) -> Result<Self> {
        use crate::config::TlsMode;
        let mode = tls.map(|t| t.mode).unwrap_or(TlsMode::Prefer);
        // a full URL in the host field wins (Elastic Cloud endpoints, custom paths)
        let base = if host.starts_with("http://") || host.starts_with("https://") {
            host.trim_end_matches('/').to_string()
        } else {
            let https = matches!(
                mode,
                TlsMode::Require | TlsMode::VerifyCa | TlsMode::VerifyFull
            ) || matches!(port, 443 | 9243);
            format!("{}://{host}:{port}", if https { "https" } else { "http" })
        };
        let mut builder = reqwest::Client::builder().timeout(std::time::Duration::from_secs(300));
        if mode == TlsMode::Require {
            builder = builder.danger_accept_invalid_certs(true);
        }
        if let Some(ca) = tls
            .and_then(|t| t.ca_cert.as_deref())
            .filter(|s| !s.is_empty())
        {
            let pem = std::fs::read(crate::tunnel::expand_home(ca))?;
            let cert = reqwest::Certificate::from_pem(&pem)
                .map_err(|e| err(format!("CA certificate: {e}")))?;
            builder = builder.add_root_certificate(cert);
        }
        let user = user.map(str::trim).filter(|u| !u.is_empty());
        let password = password.filter(|p| !p.is_empty());
        let auth = match (user, password) {
            (Some(u), p) => Auth::Basic(u.to_string(), p.unwrap_or_default().to_string()),
            // no user + a secret = an API key (base64 "id:key", as Kibana shows it)
            (None, Some(k)) => Auth::ApiKey(k.trim().to_string()),
            (None, None) => Auth::None,
        };
        let mut c = Self {
            http: builder.build().map_err(err)?,
            base,
            auth,
            flavor: Flavor::Elasticsearch,
        };
        let info = c.request(reqwest::Method::GET, "/", None).await?;
        if info["version"]["distribution"].as_str() == Some("opensearch") {
            c.flavor = Flavor::OpenSearch;
        }
        Ok(c)
    }

    async fn request(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value> {
        let mut req = self.http.request(method, format!("{}{path}", self.base));
        req = match &self.auth {
            Auth::None => req,
            Auth::Basic(u, p) => req.basic_auth(u, Some(p)),
            Auth::ApiKey(k) => req.header("Authorization", format!("ApiKey {k}")),
        };
        if let Some(b) = body {
            req = req.json(&b);
        }
        let res = req.send().await.map_err(err)?;
        let status = res.status();
        let text = res.text().await.map_err(err)?;
        let v: Value = serde_json::from_str(&text).unwrap_or(Value::String(text.clone()));
        if !status.is_success() {
            let reason = v["error"]["reason"]
                .as_str()
                .or_else(|| v["error"]["details"].as_str())
                .or_else(|| v["error"].as_str())
                .map(str::to_string)
                .unwrap_or_else(|| text.trim().to_string());
            return Err(err(format!("{status}: {reason}")));
        }
        Ok(v)
    }

    fn sql_path(&self) -> &'static str {
        match self.flavor {
            Flavor::Elasticsearch => "/_sql?format=json",
            Flavor::OpenSearch => "/_plugins/_sql?format=jdbc",
        }
    }

    /// One statement, following the cursor until `limit` rows (+1 to detect truncation).
    async fn statement(&self, sql: &str, limit: usize) -> Result<QueryResult> {
        let fetch = limit.saturating_add(1).clamp(1, 1000);
        let mut page = self
            .request(
                reqwest::Method::POST,
                self.sql_path(),
                Some(json!({ "query": clean(sql), "fetch_size": fetch })),
            )
            .await?;
        let columns: Vec<ResultColumn> = page
            .get("columns")
            .or_else(|| page.get("schema"))
            .and_then(Value::as_array)
            .map(|cols| {
                cols.iter()
                    .map(|c| ResultColumn {
                        name: c["alias"]
                            .as_str()
                            .or_else(|| c["name"].as_str())
                            .unwrap_or("")
                            .to_string(),
                        data_type: c["type"].as_str().unwrap_or("").to_string(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let mut rows: Vec<Vec<Value>> = Vec::new();
        loop {
            let key = if page.get("rows").is_some() {
                "rows"
            } else {
                "datarows"
            };
            let chunk = page.get_mut(key).map(Value::take);
            if let Some(Value::Array(r)) = chunk {
                rows.extend(r.into_iter().map(|row| match row {
                    Value::Array(a) => a,
                    other => vec![other],
                }));
            }
            let cursor = page["cursor"]
                .as_str()
                .filter(|c| !c.is_empty())
                .map(str::to_string);
            let Some(cursor) = cursor else { break };
            if rows.len() > limit {
                let close = match self.flavor {
                    Flavor::Elasticsearch => "/_sql/close",
                    Flavor::OpenSearch => "/_plugins/_sql/close",
                };
                let _ = self
                    .request(
                        reqwest::Method::POST,
                        close,
                        Some(json!({ "cursor": cursor })),
                    )
                    .await;
                break;
            }
            page = self
                .request(
                    reqwest::Method::POST,
                    self.sql_path(),
                    Some(json!({ "cursor": cursor })),
                )
                .await?;
        }
        let truncated = rows.len() > limit;
        rows.truncate(limit);
        Ok(QueryResult {
            columns,
            rows,
            rows_affected: 0,
            truncated,
        })
    }

    pub async fn run(&self, sql: &str, limit: usize) -> Result<Vec<QueryResult>> {
        let mut out = Vec::new();
        for stmt in guard::split_statements(sql) {
            if let Err(e) = guard::check_read_only(&stmt) {
                return Err(err(format!(
                    "SQL here is read-only (SELECT, SHOW, DESCRIBE) — {e}"
                )));
            }
            out.push(self.statement(&stmt, limit).await?);
        }
        Ok(out)
    }

    pub async fn tables(&self) -> Result<Vec<TableInfo>> {
        let sql = match self.flavor {
            Flavor::Elasticsearch => "SHOW TABLES",
            Flavor::OpenSearch => "SHOW TABLES LIKE %",
        };
        let r = self.statement(sql, 10_000).await?;
        let idx = |n: &str| {
            r.columns
                .iter()
                .position(|c| c.name.eq_ignore_ascii_case(n))
        };
        let (name_i, kind_i) = match self.flavor {
            Flavor::Elasticsearch => (idx("name").unwrap_or(1), idx("type")),
            Flavor::OpenSearch => (idx("TABLE_NAME").unwrap_or(2), idx("TABLE_TYPE")),
        };
        let mut out: Vec<TableInfo> = r
            .rows
            .iter()
            .filter_map(|row| {
                let name = row.get(name_i)?.as_str()?.to_string();
                if name.starts_with('.') {
                    return None; // system indices
                }
                let kind = kind_i
                    .and_then(|i| row.get(i))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_lowercase();
                Some(TableInfo {
                    // display only: SQL addresses indices by name (see Connection::target)
                    schema: "indices".into(),
                    name,
                    kind: if kind.contains("view") || kind.contains("alias") {
                        "view"
                    } else {
                        "table"
                    }
                    .into(),
                })
            })
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out.dedup_by(|a, b| a.name == b.name);
        Ok(out)
    }

    /// Columns from the index mapping (flattened `a.b.c` names, like SQL sees them).
    pub async fn describe(&self, index: &str) -> Result<Vec<crate::ColumnInfo>> {
        let path = format!("/{}/_mapping", urlencode(index));
        let v = self.request(reqwest::Method::GET, &path, None).await?;
        let mut cols = Vec::new();
        if let Some(obj) = v.as_object() {
            // an alias or pattern can resolve to several indices; merge by name
            for idx in obj.values() {
                if let Some(props) = idx["mappings"]["properties"].as_object() {
                    flatten("", props, &mut cols);
                }
            }
        }
        let mut seen = std::collections::HashSet::new();
        cols.retain(|c: &crate::ColumnInfo| seen.insert(c.name.clone()));
        Ok(cols)
    }
}

fn flatten(prefix: &str, props: &serde_json::Map<String, Value>, out: &mut Vec<crate::ColumnInfo>) {
    for (k, v) in props {
        let name = if prefix.is_empty() {
            k.clone()
        } else {
            format!("{prefix}.{k}")
        };
        if let Some(inner) = v["properties"].as_object() {
            flatten(&name, inner, out);
            continue;
        }
        out.push(crate::ColumnInfo {
            name: name.clone(),
            data_type: v["type"].as_str().unwrap_or("object").to_string(),
            nullable: true,
            default: None,
            primary_key: false,
        });
        if let Some(fields) = v["fields"].as_object() {
            for (fk, fv) in fields {
                out.push(crate::ColumnInfo {
                    name: format!("{name}.{fk}"),
                    data_type: fv["type"].as_str().unwrap_or("").to_string(),
                    nullable: true,
                    default: None,
                    primary_key: false,
                });
            }
        }
    }
}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'*' | b',' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}
