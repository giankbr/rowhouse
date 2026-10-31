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

    /// SQL, or Kibana Dev Tools requests (`GET /orders/_search` + a JSON body).
    pub async fn run(&self, src: &str, limit: usize, read_only: bool) -> Result<Vec<QueryResult>> {
        if let Some(reqs) = parse_requests(src) {
            let mut out = Vec::new();
            for r in reqs? {
                if read_only && !r.is_read() {
                    return Err(Error::ReadOnly(format!(
                        "`{} {}` changes data — not allowed in read-only mode",
                        r.method, r.path
                    )));
                }
                out.extend(self.rest(&r, limit).await?);
            }
            return Ok(out);
        }
        let mut out = Vec::new();
        for stmt in guard::split_statements(src) {
            if let Err(e) = guard::check_read_only(&stmt) {
                return Err(err(format!(
                    "SQL here is read-only (SELECT, SHOW, DESCRIBE); use Dev Tools requests \
                     like `POST /index/_doc` to change data — {e}"
                )));
            }
            out.push(self.statement(&stmt, limit).await?);
        }
        Ok(out)
    }

    async fn rest(&self, r: &RestRequest, limit: usize) -> Result<Vec<QueryResult>> {
        let mut path = if r.path.starts_with('/') {
            r.path.clone()
        } else {
            format!("/{}", r.path)
        };
        // _cat APIs: ask for JSON so they become tables
        if path.starts_with("/_cat") && !path.contains("format=") {
            path.push_str(if path.contains('?') {
                "&format=json"
            } else {
                "?format=json"
            });
        }
        let method = reqwest::Method::from_bytes(r.method.as_bytes()).map_err(err)?;
        let mut req = self.http.request(method, format!("{}{path}", self.base));
        req = match &self.auth {
            Auth::None => req,
            Auth::Basic(u, p) => req.basic_auth(u, Some(p)),
            Auth::ApiKey(k) => req.header("Authorization", format!("ApiKey {k}")),
        };
        if let Some(body) = &r.body {
            let ndjson = path.contains("_bulk") || path.contains("_msearch");
            req = req
                .header(
                    "Content-Type",
                    if ndjson {
                        "application/x-ndjson"
                    } else {
                        "application/json"
                    },
                )
                .body(if ndjson && !body.ends_with('\n') {
                    format!("{body}\n")
                } else {
                    body.clone()
                });
        }
        let res = req.send().await.map_err(err)?;
        let status = res.status();
        let text = res.text().await.map_err(err)?;
        let v: Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(_) => {
                if !status.is_success() {
                    return Err(err(format!("{status}: {}", text.trim())));
                }
                return Ok(vec![text_result(&text, limit)]);
            }
        };
        if !status.is_success() {
            let reason = v["error"]["root_cause"][0]["reason"]
                .as_str()
                .or_else(|| v["error"]["reason"].as_str())
                .or_else(|| v["error"].as_str())
                .map(str::to_string)
                .unwrap_or_else(|| text.trim().to_string());
            return Err(err(format!("{status}: {reason}")));
        }
        Ok(shape(&v, limit))
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

/// One Dev Tools request.
#[derive(Debug, Clone, PartialEq)]
pub struct RestRequest {
    pub method: String,
    pub path: String,
    pub body: Option<String>,
}

const READ_POST: [&str; 11] = [
    "_search",
    "_count",
    "_msearch",
    "_mget",
    "_sql",
    "_field_caps",
    "_validate",
    "_explain",
    "_termsenum",
    "_terms_enum",
    "_eql",
];

impl RestRequest {
    /// Reads: GET/HEAD, and POSTs to search-style endpoints.
    pub fn is_read(&self) -> bool {
        match self.method.as_str() {
            "GET" | "HEAD" => true,
            "POST" => {
                let p = self.path.split('?').next().unwrap_or("");
                // `_sql` is fine; `_sql/close` too; `_search/template` too
                p.split('/').any(|seg| READ_POST.contains(&seg))
                    && !p.contains("_delete_by_query")
                    && !p.contains("_update_by_query")
            }
            _ => false,
        }
    }

    /// `DELETE /index` (a whole index or pattern), the one that needs a typed confirmation.
    pub fn drops_index(&self) -> Option<String> {
        if self.method != "DELETE" {
            return None;
        }
        let p = self.path.split('?').next().unwrap_or("").trim_matches('/');
        (!p.is_empty() && !p.contains('/') && !p.starts_with('_')).then(|| p.to_string())
    }
}

const METHODS: [&str; 5] = ["GET", "POST", "PUT", "DELETE", "HEAD"];

fn method_line(line: &str) -> Option<(String, String)> {
    let (m, rest) = line.trim().split_once(char::is_whitespace)?;
    let m = m.to_uppercase();
    let path = rest.trim();
    // a path is one token (`DELETE FROM orders` is SQL, not a request)
    (METHODS.contains(&m.as_str()) && !path.is_empty() && !path.contains(char::is_whitespace))
        .then(|| (m, path.to_string()))
}

/// `None` when `src` is SQL. Otherwise each `METHOD path` line starts a request
/// and the lines below it (up to the next one) are its body. `#` and `//` lines are comments.
pub fn parse_requests(src: &str) -> Option<Result<Vec<RestRequest>>> {
    let lines: Vec<&str> = src
        .lines()
        .filter(|l| {
            let t = l.trim_start();
            !t.starts_with('#') && !t.starts_with("//")
        })
        .collect();
    let first = lines.iter().find(|l| !l.trim().is_empty())?;
    method_line(first)?;
    let mut out: Vec<RestRequest> = Vec::new();
    let mut body: Vec<&str> = Vec::new();
    let flush = |out: &mut Vec<RestRequest>, body: &mut Vec<&str>| -> Result<()> {
        if let Some(last) = out.last_mut() {
            let b = body.join("\n").trim().to_string();
            if !b.is_empty() {
                let ndjson = last.path.contains("_bulk") || last.path.contains("_msearch");
                if !ndjson {
                    serde_json::from_str::<Value>(&b).map_err(|e| {
                        err(format!(
                            "the body of `{} {}` isn't valid JSON: {e}",
                            last.method, last.path
                        ))
                    })?;
                }
                last.body = Some(b);
            }
        }
        body.clear();
        Ok(())
    };
    for l in lines {
        // a method line at the start of a line begins the next request
        if !l.starts_with(char::is_whitespace) {
            if let Some((method, path)) = method_line(l) {
                if let Err(e) = flush(&mut out, &mut body) {
                    return Some(Err(e));
                }
                out.push(RestRequest {
                    method,
                    path,
                    body: None,
                });
                continue;
            }
        }
        body.push(l);
    }
    if let Err(e) = flush(&mut out, &mut body) {
        return Some(Err(e));
    }
    Some(Ok(out))
}

fn text_result(text: &str, limit: usize) -> QueryResult {
    let lines: Vec<Vec<Value>> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| vec![Value::String(l.to_string())])
        .collect();
    QueryResult {
        columns: vec![ResultColumn {
            name: "response".into(),
            data_type: "text".into(),
        }],
        truncated: lines.len() > limit,
        rows: lines.into_iter().take(limit).collect(),
        rows_affected: 0,
    }
}

/// Flatten nested objects into `a.b` keys; arrays stay JSON.
fn flat(prefix: &str, v: &Value, out: &mut Vec<(String, Value)>) {
    match v.as_object() {
        Some(o) if !o.is_empty() => {
            for (k, x) in o {
                let key = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                flat(&key, x, out);
            }
        }
        _ => out.push((prefix.to_string(), v.clone())),
    }
}

/// Rows of objects → one table, columns in order of first appearance.
fn objects_table(items: &[Vec<(String, Value)>], limit: usize, total: Option<u64>) -> QueryResult {
    let mut cols: Vec<String> = Vec::new();
    for it in items {
        for (k, _) in it {
            if !cols.contains(k) {
                cols.push(k.clone());
            }
        }
    }
    let rows: Vec<Vec<Value>> = items
        .iter()
        .take(limit)
        .map(|it| {
            cols.iter()
                .map(|c| {
                    it.iter()
                        .find(|(k, _)| k == c)
                        .map(|(_, v)| v.clone())
                        .unwrap_or(Value::Null)
                })
                .collect()
        })
        .collect();
    QueryResult {
        columns: cols
            .into_iter()
            .map(|name| ResultColumn {
                name,
                data_type: String::new(),
            })
            .collect(),
        truncated: items.len() > limit || total.is_some_and(|t| t > items.len().min(limit) as u64),
        rows,
        rows_affected: 0,
    }
}

fn agg_table(name: &str, a: &Value, limit: usize) -> Option<QueryResult> {
    let buckets: Vec<Value> = match &a["buckets"] {
        Value::Array(b) => b.clone(),
        // keyed buckets: { "key": { doc_count, ... } }
        Value::Object(o) => o
            .iter()
            .map(|(k, v)| {
                let mut v = v.clone();
                v["key"] = json!(k);
                v
            })
            .collect(),
        _ => {
            // a metric: { value } or { values: {...} } or stats
            let mut kv = Vec::new();
            flat("", a, &mut kv);
            kv.retain(|(k, _)| k != "meta");
            return Some(objects_table(
                &[kv.into_iter()
                    .map(|(k, v)| (format!("{name}.{k}"), v))
                    .collect()],
                limit,
                None,
            ));
        }
    };
    let items: Vec<Vec<(String, Value)>> = buckets
        .iter()
        .map(|b| {
            let mut row = vec![(
                name.to_string(),
                b.get("key_as_string")
                    .cloned()
                    .unwrap_or_else(|| b["key"].clone()),
            )];
            row.push(("doc_count".into(), b["doc_count"].clone()));
            if let Some(o) = b.as_object() {
                for (k, v) in o {
                    if matches!(k.as_str(), "key" | "key_as_string" | "doc_count") {
                        continue;
                    }
                    // sub-metrics become columns; sub-bucket aggs stay JSON
                    match v.get("value") {
                        Some(x) => row.push((k.clone(), x.clone())),
                        None => row.push((k.clone(), v.clone())),
                    }
                }
            }
            row
        })
        .collect();
    Some(objects_table(&items, limit, None))
}

/// A REST response → result sets.
pub fn shape(v: &Value, limit: usize) -> Vec<QueryResult> {
    if let Some(hits) = v["hits"]["hits"].as_array() {
        let mut out = Vec::new();
        let total = v["hits"]["total"]["value"]
            .as_u64()
            .or_else(|| v["hits"]["total"].as_u64());
        let aggs = v["aggregations"].as_object();
        if !hits.is_empty() || aggs.is_none() {
            let items: Vec<Vec<(String, Value)>> = hits
                .iter()
                .map(|h| {
                    let mut row = vec![
                        ("_index".to_string(), h["_index"].clone()),
                        ("_id".to_string(), h["_id"].clone()),
                    ];
                    if !h["_score"].is_null() {
                        row.push(("_score".into(), h["_score"].clone()));
                    }
                    let src = h.get("_source").or_else(|| h.get("fields"));
                    if let Some(src) = src {
                        flat("", src, &mut row);
                    }
                    row
                })
                .collect();
            out.push(objects_table(&items, limit, total));
        }
        for (name, a) in aggs.into_iter().flatten() {
            out.extend(agg_table(name, a, limit));
        }
        return out;
    }
    if let Some(arr) = v.as_array() {
        if arr.iter().all(Value::is_object) {
            let items: Vec<Vec<(String, Value)>> = arr
                .iter()
                .map(|o| {
                    let mut row = Vec::new();
                    flat("", o, &mut row);
                    row
                })
                .collect();
            return vec![objects_table(&items, limit, None)];
        }
    }
    if let Some(n) = v["count"].as_u64().filter(|_| v.get("_shards").is_some()) {
        return vec![objects_table(&[vec![("count".into(), json!(n))]], 1, None)];
    }
    // anything else: field / value pairs
    let mut kv = Vec::new();
    flat("", v, &mut kv);
    let rows: Vec<Vec<Value>> = kv
        .into_iter()
        .map(|(k, x)| vec![Value::String(k), x])
        .collect();
    let affected = v["result"]
        .as_str()
        .map(|r| u64::from(matches!(r, "created" | "updated" | "deleted")))
        .or_else(|| v["deleted"].as_u64())
        .or_else(|| v["updated"].as_u64())
        .unwrap_or(0);
    vec![QueryResult {
        columns: vec![
            ResultColumn {
                name: "field".into(),
                data_type: "text".into(),
            },
            ResultColumn {
                name: "value".into(),
                data_type: String::new(),
            },
        ],
        truncated: rows.len() > limit,
        rows: rows.into_iter().take(limit).collect(),
        rows_affected: affected,
    }]
}
