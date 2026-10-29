//! ClickHouse over its HTTP interface (port 8123 / 8443).
//! Uses the `JSONCompact` output format, which every ClickHouse version supports.

use serde_json::Value;

use crate::{guard, Error, QueryResult, Result, ResultColumn};

pub struct Client {
    http: reqwest::Client,
    url: String,
    user: Option<String>,
    password: Option<String>,
    database: Option<String>,
}

#[derive(serde::Deserialize)]
struct Compact {
    meta: Vec<Meta>,
    data: Vec<Vec<Value>>,
}

#[derive(serde::Deserialize)]
struct Meta {
    name: String,
    #[serde(rename = "type")]
    ty: String,
}

impl Client {
    pub async fn connect(
        host: &str,
        port: u16,
        user: Option<&str>,
        password: Option<&str>,
        database: Option<&str>,
    ) -> Result<Self> {
        let scheme = if matches!(port, 443 | 8443) {
            "https"
        } else {
            "http"
        };
        let c = Self {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(300))
                .build()
                .map_err(|e| Error::Other(e.to_string()))?,
            url: format!("{scheme}://{host}:{port}/"),
            user: user.map(str::to_string),
            password: password.map(str::to_string),
            database: database.filter(|d| !d.is_empty()).map(str::to_string),
        };
        c.statement("SELECT 1", false).await?;
        Ok(c)
    }

    async fn statement(&self, sql: &str, read_only: bool) -> Result<String> {
        let mut q: Vec<(&str, &str)> = vec![
            ("default_format", "JSONCompact"),
            ("output_format_json_quote_64bit_integers", "0"),
        ];
        if let Some(db) = &self.database {
            q.push(("database", db));
        }
        if read_only {
            q.push(("readonly", "1"));
        }
        let url = reqwest::Url::parse_with_params(&self.url, &q)
            .map_err(|e| Error::Other(e.to_string()))?;
        let mut req = self.http.post(url).body(sql.to_string());
        if let Some(u) = &self.user {
            req = req.header("X-ClickHouse-User", u);
        }
        if let Some(p) = &self.password {
            req = req.header("X-ClickHouse-Key", p);
        }
        let res = req
            .send()
            .await
            .map_err(|e| Error::Other(format!("clickhouse: {e}")))?;
        let status = res.status();
        let body = res
            .text()
            .await
            .map_err(|e| Error::Other(format!("clickhouse: {e}")))?;
        if !status.is_success() {
            return Err(Error::Other(format!("clickhouse: {}", body.trim())));
        }
        Ok(body)
    }

    pub async fn run(&self, sql: &str, limit: usize, read_only: bool) -> Result<Vec<QueryResult>> {
        let mut out = Vec::new();
        for stmt in guard::split_statements(sql) {
            let body = self.statement(&stmt, read_only).await?;
            if body.trim().is_empty() {
                out.push(QueryResult::default());
                continue;
            }
            let parsed: Compact = serde_json::from_str(&body)?;
            let truncated = parsed.data.len() > limit;
            out.push(QueryResult {
                columns: parsed
                    .meta
                    .into_iter()
                    .map(|m| ResultColumn {
                        name: m.name,
                        data_type: m.ty,
                    })
                    .collect(),
                rows: parsed.data.into_iter().take(limit).collect(),
                rows_affected: 0,
                truncated,
            });
        }
        Ok(out)
    }

    /// ClickHouse has no multi-statement transactions; statements run in order.
    pub async fn exec_all(&self, stmts: &[String]) -> Result<u64> {
        for s in stmts {
            self.statement(s, false).await?;
        }
        Ok(stmts.len() as u64)
    }
}
