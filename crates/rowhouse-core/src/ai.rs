//! Bring-your-own-model AI helpers: natural language → SQL, fix a failing
//! query, explain a query.
//!
//! Privacy: by default the model only sees the database *schema* (table and
//! column names/types) and the SQL/error text you're working on — never rows.
//!
//! Providers:
//! * `openai` — any OpenAI-compatible `/chat/completions` endpoint
//!   (OpenAI, OpenRouter, Groq, Together, Ollama, LM Studio, vLLM, …)
//! * `anthropic` — the Anthropic Messages API

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{config::DbKind, Connection, ConnectionStore, Error, Result, TableInfo};

const KEYRING_SERVICE: &str = "rowhouse";
const KEYRING_ACCOUNT: &str = "ai:api-key";
/// Max tables whose columns are sent; the rest are listed by name only.
const MAX_DETAILED_TABLES: usize = 40;
const MAX_TABLE_NAMES: usize = 400;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    #[default]
    Openai,
    Anthropic,
    /// a coding-agent CLI on this machine (Claude Code, Codex, …) — see `ai_local`
    Local,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSettings {
    /// AI is opt-in.
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub provider: Provider,
    /// e.g. `https://api.openai.com/v1`, `http://localhost:11434/v1`, `https://api.anthropic.com/v1`
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub model: String,
    /// Ghost-text completions while typing (sends a request per pause).
    #[serde(default)]
    pub inline_completions: bool,
    /// `Provider::Local`: which CLI (`claude`, `codex`, `opencode`, …)
    #[serde(default)]
    pub agent: String,
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: Provider::Openai,
            base_url: "https://api.openai.com/v1".into(),
            model: String::new(),
            inline_completions: false,
            agent: String::new(),
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct SettingsFile {
    #[serde(default)]
    ai: AiSettings,
}

impl AiSettings {
    fn path() -> std::path::PathBuf {
        ConnectionStore::default_dir().join("settings.json")
    }

    pub fn load() -> Result<Self> {
        match std::fs::read_to_string(Self::path()) {
            Ok(s) => Ok(serde_json::from_str::<SettingsFile>(&s)?.ai),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(
            &path,
            serde_json::to_vec_pretty(&SettingsFile { ai: self.clone() })?,
        )?;
        Ok(())
    }

    pub fn api_key() -> Result<Option<String>> {
        match keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT)?.get_password() {
            Ok(k) => Ok(Some(k)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn set_api_key(key: Option<&str>) -> Result<()> {
        let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT)?;
        match key {
            Some(k) if !k.is_empty() => entry.set_password(k)?,
            _ => match entry.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => {}
                Err(e) => return Err(e.into()),
            },
        }
        Ok(())
    }
}

/// What the assistant proposes. The UI shows it as a diff; nothing runs until
/// the user accepts.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AiAnswer {
    /// Proposed SQL (empty for `explain`).
    pub sql: String,
    /// Short human explanation.
    pub explanation: String,
}

pub struct AiClient {
    settings: AiSettings,
    api_key: Option<String>,
    http: reqwest::Client,
}

fn err(msg: impl std::fmt::Display) -> Error {
    Error::Other(format!("ai: {msg}"))
}

fn dialect_name(kind: DbKind) -> &'static str {
    use DbKind::*;
    match kind {
        Postgres | Neon | Supabase => "PostgreSQL",
        Cockroach => "CockroachDB (PostgreSQL dialect)",
        Redshift => "Amazon Redshift",
        Greenplum => "Greenplum (PostgreSQL dialect)",
        Mysql => "MySQL",
        Mariadb => "MariaDB",
        Tidb => "TiDB (MySQL dialect)",
        Singlestore => "SingleStore (MySQL dialect)",
        Sqlite => "SQLite",
        Turso => "libSQL / Turso (SQLite dialect)",
        D1 => "Cloudflare D1 (SQLite dialect)",
        Duckdb => "DuckDB",
        Mssql => "Microsoft SQL Server (T-SQL)",
        Clickhouse => "ClickHouse",
        Oracle => "Oracle Database (Oracle SQL: FETCH FIRST n ROWS ONLY, no LIMIT; DUAL for scalar selects)",
        Redis | Upstash => "Redis (reply with Redis commands, one per line, instead of SQL)",
        Cassandra => "Apache Cassandra (CQL; filter on partition/clustering keys, no JOINs)",
        Scylla => "ScyllaDB (CQL, Cassandra-compatible; filter on partition/clustering keys, no JOINs)",
        Elasticsearch => "Elasticsearch SQL (read-only SELECT; indices are tables, quote names with dots or dashes in double quotes, use LIMIT)",
        Opensearch => "OpenSearch SQL (read-only SELECT; indices are tables, use LIMIT)",
        Bigquery => "Google BigQuery GoogleSQL (tables as `dataset.table` in backticks, LIMIT, SAFE_CAST, DATE_TRUNC(col, DAY))",
        Mongodb => "MongoDB (reply with mongosh statements such as db.orders.find({ status: \"paid\" }).sort({ total: -1 }).limit(10) or db.orders.aggregate([...]) instead of SQL; the schema lists collections and sampled fields)",
    }
}

/// Words in `hint` (lowercased, split on non-identifier chars).
fn tokens(hint: &str) -> Vec<String> {
    hint.to_lowercase()
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|w| w.len() > 1)
        .map(str::to_string)
        .collect()
}

/// Rank tables by how strongly the prompt/SQL mentions them.
fn relevance(t: &TableInfo, words: &[String]) -> i32 {
    let name = t.name.to_lowercase();
    let singular = name.trim_end_matches('s');
    let mut score = 0;
    for w in words {
        let ws = w.trim_end_matches('s');
        if *w == name {
            score += 10;
        } else if ws == singular && !ws.is_empty() {
            score += 8;
        } else if name.contains(w.as_str()) || (ws.len() > 3 && name.contains(ws)) {
            score += 3;
        }
    }
    score
}

/// Compact schema description for the prompt — names and types only, no data.
pub async fn schema_context(conn: &Connection, hint: &str) -> Result<String> {
    let mut tables = conn.list_tables().await?;
    let words = tokens(hint);
    tables.sort_by_key(|t| -relevance(t, &words));
    let multi_schema = tables.iter().any(|t| t.schema != tables[0].schema);

    let mut out = String::new();
    for (i, t) in tables.iter().take(MAX_TABLE_NAMES).enumerate() {
        let name = if multi_schema {
            format!("{}.{}", t.schema, t.name)
        } else {
            t.name.clone()
        };
        if i < MAX_DETAILED_TABLES {
            let cols = conn
                .describe_table(&t.schema, &t.name)
                .await
                .unwrap_or_default();
            let cols = cols
                .iter()
                .map(|c| {
                    let mut s = format!("{} {}", c.name, c.data_type);
                    if c.primary_key {
                        s.push_str(" PK");
                    }
                    if !c.nullable {
                        s.push_str(" NOT NULL");
                    }
                    s
                })
                .collect::<Vec<_>>()
                .join(", ");
            let kind = if t.kind == "view" { " (view)" } else { "" };
            out.push_str(&format!("{name}{kind}({cols})\n"));
        } else {
            out.push_str(&format!("{name}\n"));
        }
    }
    if tables.len() > MAX_TABLE_NAMES {
        out.push_str(&format!(
            "… and {} more tables\n",
            tables.len() - MAX_TABLE_NAMES
        ));
    }
    Ok(out)
}

fn system_prompt(kind: DbKind, schema: &str) -> String {
    format!(
        "You are an expert {d} assistant inside a database client.\n\
         Rules:\n\
         - Write SQL for {d} only, using exactly the table and column names in the schema below.\n\
         - Never invent tables or columns. If the request is impossible with this schema, say so in the explanation and return no SQL.\n\
         - Prefer readable SQL: explicit column lists, clear aliases, one statement unless asked otherwise.\n\
         - Never write destructive statements (DROP, TRUNCATE, DELETE/UPDATE without WHERE) unless the user explicitly asks.\n\
         - Reply with ONE ```sql fenced block followed by at most two short sentences of explanation.\n\n\
         Schema (table(column type …)):\n{schema}",
        d = dialect_name(kind)
    )
}

/// Optional streaming callback: receives each text delta.
pub type Delta<'a> = Option<&'a mut (dyn FnMut(&str) + Send)>;

/// Strip fences / echoed prefix from an inline completion.
pub fn clean_inline(reply: &str, before: &str) -> String {
    let mut s = reply.trim_end().to_string();
    if s.trim_start().starts_with("```") {
        s = extract_sql(&s).sql;
    }
    s = s.replace("<CURSOR>", "");
    // models sometimes repeat the current line; drop the overlap
    let last_line = before.rsplit('\n').next().unwrap_or("");
    let trimmed = last_line.trim_start();
    if !trimmed.is_empty() && s.trim_start().starts_with(trimmed) {
        s = s.trim_start()[trimmed.len()..].to_string();
    }
    if before.ends_with(char::is_whitespace) {
        s = s.trim_start().to_string();
    }
    s
}

/// Pull the first ```sql (or ```) block out of a model reply.
pub fn extract_sql(reply: &str) -> AiAnswer {
    let lower = reply.to_lowercase();
    let start = lower
        .find("```sql")
        .map(|i| i + 6)
        .or_else(|| lower.find("```").map(|i| i + 3));
    if let Some(start) = start {
        let rest = &reply[start..];
        let end = rest.find("```").unwrap_or(rest.len());
        let sql = rest[..end].trim().to_string();
        let after = rest.get(end + 3..).unwrap_or("").trim();
        let before = reply[..start]
            .rsplit_once("```")
            .map(|(b, _)| b)
            .unwrap_or("")
            .trim();
        let explanation = if after.is_empty() { before } else { after };
        return AiAnswer {
            sql,
            explanation: explanation.to_string(),
        };
    }
    AiAnswer {
        sql: String::new(),
        explanation: reply.trim().to_string(),
    }
}

impl AiClient {
    pub fn new(settings: AiSettings, api_key: Option<String>) -> Result<Self> {
        crate::init_crypto();
        if settings.provider == Provider::Local {
            if settings.agent.trim().is_empty() {
                return Err(err("pick a local agent in Settings → AI"));
            }
        } else if settings.model.trim().is_empty() {
            return Err(err("no model configured — open Settings → AI"));
        }
        Ok(Self {
            settings,
            api_key,
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .map_err(err)?,
        })
    }

    /// Settings from disk + key from the keychain; errors if AI is disabled.
    pub fn from_saved() -> Result<Self> {
        let s = AiSettings::load()?;
        if !s.enabled {
            return Err(err("AI is turned off — enable it in Settings → AI"));
        }
        Self::new(s, AiSettings::api_key()?)
    }

    fn url(&self, path: &str) -> String {
        format!("{}/{}", self.settings.base_url.trim_end_matches('/'), path)
    }

    async fn post(&self, url: String, body: Value, headers: &[(&str, String)]) -> Result<Value> {
        let mut req = self
            .http
            .post(url)
            .header("content-type", "application/json")
            .body(body.to_string());
        for (k, v) in headers {
            req = req.header(*k, v);
        }
        let res = req.send().await.map_err(err)?;
        let status = res.status();
        let text = res.text().await.map_err(err)?;
        if !status.is_success() {
            // surface the provider's error message when there is one
            let msg = serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|v| v["error"]["message"].as_str().map(str::to_string))
                .unwrap_or(text);
            return Err(err(format!("{status}: {}", msg.trim())));
        }
        serde_json::from_str(&text).map_err(err)
    }

    /// One system + one user message → assistant text.
    pub async fn complete(&self, system: &str, user: &str) -> Result<String> {
        let key = self.api_key.clone().unwrap_or_default();
        match self.settings.provider {
            Provider::Local => {
                crate::ai_local::run(&self.settings.agent, &self.settings.model, system, user).await
            }
            Provider::Openai => {
                let mut headers = vec![];
                if !key.is_empty() {
                    headers.push(("authorization", format!("Bearer {key}")));
                }
                let v = self
                    .post(
                        self.url("chat/completions"),
                        json!({
                            "model": self.settings.model,
                            "temperature": 0,
                            "messages": [
                                {"role": "system", "content": system},
                                {"role": "user", "content": user},
                            ],
                        }),
                        &headers,
                    )
                    .await?;
                v["choices"][0]["message"]["content"]
                    .as_str()
                    .map(str::to_string)
                    .ok_or_else(|| err("unexpected response from provider"))
            }
            Provider::Anthropic => {
                let v = self
                    .post(
                        self.url("messages"),
                        json!({
                            "model": self.settings.model,
                            "max_tokens": 2048,
                            "temperature": 0,
                            "system": system,
                            "messages": [{"role": "user", "content": user}],
                        }),
                        &[
                            ("x-api-key", key),
                            ("anthropic-version", "2023-06-01".into()),
                        ],
                    )
                    .await?;
                let text: String = v["content"]
                    .as_array()
                    .ok_or_else(|| err("unexpected response from provider"))?
                    .iter()
                    .filter_map(|b| b["text"].as_str())
                    .collect();
                Ok(text)
            }
        }
    }

    /// Model ids offered by the endpoint (`GET /models`), for the settings picker.
    pub async fn list_models(&self) -> Result<Vec<String>> {
        if self.settings.provider == Provider::Local {
            // the CLIs pick a default; these are the common aliases
            return Ok(match self.settings.agent.as_str() {
                "claude" => vec!["sonnet", "opus", "haiku"],
                "gemini" => vec!["gemini-2.5-pro", "gemini-2.5-flash"],
                "codex" => vec!["gpt-5-codex", "gpt-5"],
                _ => vec![],
            }
            .into_iter()
            .map(String::from)
            .collect());
        }
        let key = self.api_key.clone().unwrap_or_default();
        let mut req = self.http.get(self.url("models"));
        req = match self.settings.provider {
            Provider::Openai if !key.is_empty() => {
                req.header("authorization", format!("Bearer {key}"))
            }
            Provider::Openai | Provider::Local => req,
            Provider::Anthropic => req
                .header("x-api-key", key)
                .header("anthropic-version", "2023-06-01"),
        };
        let res = req.send().await.map_err(err)?;
        let status = res.status();
        let text = res.text().await.map_err(err)?;
        if !status.is_success() {
            return Err(err(format!("{status}: {}", text.trim())));
        }
        let v: Value = serde_json::from_str(&text).map_err(err)?;
        let mut ids: Vec<String> = v["data"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|m| m["id"].as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        ids.sort();
        Ok(ids)
    }

    /// Stream when the caller wants deltas, otherwise one request.
    async fn ask(&self, system: &str, user: &str, on_delta: Delta<'_>) -> Result<String> {
        match on_delta {
            Some(f) => self.complete_stream(system, user, f).await,
            None => self.complete(system, user).await,
        }
    }

    /// Like [`Self::complete`], but calls `on_delta` with each text chunk as it
    /// arrives (server-sent events). Returns the full text.
    pub async fn complete_stream(
        &self,
        system: &str,
        user: &str,
        on_delta: &mut (dyn FnMut(&str) + Send),
    ) -> Result<String> {
        use futures_util::StreamExt;
        if self.settings.provider == Provider::Local {
            let full = self.complete(system, user).await?;
            on_delta(&full);
            return Ok(full);
        }
        let key = self.api_key.clone().unwrap_or_default();
        let (url, body, headers): (String, Value, Vec<(&str, String)>) = match self
            .settings
            .provider
        {
            Provider::Local => unreachable!("handled above"),
            Provider::Openai => (
                self.url("chat/completions"),
                json!({
                    "model": self.settings.model, "temperature": 0, "stream": true,
                    "messages": [{"role": "system", "content": system}, {"role": "user", "content": user}],
                }),
                if key.is_empty() {
                    vec![]
                } else {
                    vec![("authorization", format!("Bearer {key}"))]
                },
            ),
            Provider::Anthropic => (
                self.url("messages"),
                json!({
                    "model": self.settings.model, "max_tokens": 2048, "temperature": 0, "stream": true,
                    "system": system, "messages": [{"role": "user", "content": user}],
                }),
                vec![
                    ("x-api-key", key),
                    ("anthropic-version", "2023-06-01".into()),
                ],
            ),
        };
        let mut req = self
            .http
            .post(url)
            .header("content-type", "application/json")
            .body(body.to_string());
        for (k, v) in &headers {
            req = req.header(*k, v);
        }
        let res = req.send().await.map_err(err)?;
        let status = res.status();
        if !status.is_success() {
            let text = res.text().await.map_err(err)?;
            let msg = serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|v| v["error"]["message"].as_str().map(str::to_string))
                .unwrap_or(text);
            return Err(err(format!("{status}: {}", msg.trim())));
        }
        let mut stream = res.bytes_stream();
        let mut buf = String::new();
        let mut full = String::new();
        'read: while let Some(chunk) = stream.next().await {
            buf.push_str(&String::from_utf8_lossy(&chunk.map_err(err)?));
            // SSE events are separated by a blank line
            while let Some(end) = buf.find("\n\n").or_else(|| buf.find("\r\n\r\n")) {
                let event: String = buf.drain(..end).collect();
                buf = buf.trim_start_matches(['\r', '\n']).to_string();
                for line in event.lines() {
                    let Some(data) = line.strip_prefix("data:").map(str::trim) else {
                        continue;
                    };
                    if data == "[DONE]" {
                        break 'read;
                    }
                    let Ok(v) = serde_json::from_str::<Value>(data) else {
                        continue;
                    };
                    if let Some(e) = v.get("error") {
                        return Err(err(e["message"].as_str().unwrap_or("stream error")));
                    }
                    let piece = match self.settings.provider {
                        Provider::Openai | Provider::Local => {
                            v["choices"][0]["delta"]["content"].as_str()
                        }
                        Provider::Anthropic => match v["type"].as_str() {
                            Some("content_block_delta") => v["delta"]["text"].as_str(),
                            Some("message_stop") => break 'read,
                            _ => None,
                        },
                    };
                    if let Some(p) = piece.filter(|p| !p.is_empty()) {
                        full.push_str(p);
                        on_delta(p);
                    }
                }
            }
        }
        Ok(full)
    }

    /// Ghost-text completion: what to insert at the cursor. Empty when there's
    /// nothing useful. Short and cheap: small schema context, few tokens.
    pub async fn complete_inline(
        &self,
        conn: &Connection,
        before: &str,
        after: &str,
    ) -> Result<String> {
        if self.settings.provider == Provider::Local {
            return Ok(String::new()); // a CLI round-trip is too slow for ghost text
        }
        let before: String = before
            .chars()
            .rev()
            .take(2000)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        let after: String = after.chars().take(500).collect();
        let schema = schema_context(conn, &before).await?;
        let schema: String = schema.lines().take(25).collect::<Vec<_>>().join("\n");
        let system = format!(
            "You are an autocomplete engine for {d} in a database client. Return ONLY the text to insert at <CURSOR> \
             — no explanations, no code fences, no repetition of text before the cursor. Complete at most one statement. \
             Use only tables and columns from this schema:\n{schema}\n\nIf there is nothing useful to add, return an empty reply.",
            d = dialect_name(conn.kind())
        );
        let reply = self
            .complete(&system, &format!("{before}<CURSOR>{after}"))
            .await?;
        Ok(clean_inline(&reply, &before))
    }

    /// Natural language → SQL. `current_sql` gives the model the editor context.
    pub async fn generate(
        &self,
        conn: &Connection,
        request: &str,
        current_sql: Option<&str>,
        on_delta: Delta<'_>,
    ) -> Result<AiAnswer> {
        let schema = schema_context(
            conn,
            &format!("{request} {}", current_sql.unwrap_or_default()),
        )
        .await?;
        let mut user = format!("Request: {request}");
        if let Some(sql) = current_sql.filter(|s| !s.trim().is_empty()) {
            user.push_str(&format!("\n\nCurrent editor SQL (modify it if the request refers to it):\n```sql\n{sql}\n```"));
        }
        let reply = self
            .ask(&system_prompt(conn.kind(), &schema), &user, on_delta)
            .await?;
        Ok(extract_sql(&reply))
    }

    /// Inline edit: rewrite `selection` (or write new SQL at the cursor when it's
    /// empty) following `instruction`. `before` / `after` are the surrounding text.
    pub async fn edit(
        &self,
        conn: &Connection,
        instruction: &str,
        selection: &str,
        before: &str,
        after: &str,
        on_delta: Delta<'_>,
    ) -> Result<AiAnswer> {
        let tail = |s: &str, n: usize| -> String {
            let v: Vec<char> = s.chars().collect();
            v[v.len().saturating_sub(n)..].iter().collect()
        };
        let before = tail(before, 1500);
        let after: String = after.chars().take(800).collect();
        let schema = schema_context(conn, &format!("{instruction} {selection} {before}")).await?;
        let user = if selection.trim().is_empty() {
            format!(
                "Write SQL to insert at <CURSOR>: {instruction}\n\nEditor:\n```sql\n{before}<CURSOR>{after}\n```\n\n\
                 Reply with only the SQL to insert in one ```sql block (no surrounding text from the editor)."
            )
        } else {
            format!(
                "Rewrite the selected SQL: {instruction}\n\nSelected:\n```sql\n{selection}\n```\n\n\
                 Context before:\n```sql\n{before}\n```\nContext after:\n```sql\n{after}\n```\n\n\
                 Reply with only the replacement for the selected text in one ```sql block, then at most one sentence on what changed."
            )
        };
        let reply = self
            .ask(&system_prompt(conn.kind(), &schema), &user, on_delta)
            .await?;
        Ok(extract_sql(&reply))
    }

    /// Propose a corrected query for an error.
    pub async fn fix(
        &self,
        conn: &Connection,
        sql: &str,
        error: &str,
        on_delta: Delta<'_>,
    ) -> Result<AiAnswer> {
        let schema = schema_context(conn, &format!("{sql} {error}")).await?;
        let user = format!(
            "This query fails. Return the corrected query and say in one sentence what was wrong.\n\n\
             ```sql\n{sql}\n```\n\nError:\n{error}"
        );
        let reply = self
            .ask(&system_prompt(conn.kind(), &schema), &user, on_delta)
            .await?;
        Ok(extract_sql(&reply))
    }

    /// Plain-language explanation of a query (no SQL returned).
    pub async fn explain(
        &self,
        conn: &Connection,
        sql: &str,
        on_delta: Delta<'_>,
    ) -> Result<AiAnswer> {
        let schema = schema_context(conn, sql).await?;
        let user = format!(
            "Explain what this query does in 2–4 short sentences for a developer, and mention any \
             obvious performance or correctness risk. Do not rewrite it.\n\n```sql\n{sql}\n```"
        );
        let reply = self
            .ask(&system_prompt(conn.kind(), &schema), &user, on_delta)
            .await?;
        Ok(AiAnswer {
            sql: String::new(),
            explanation: reply.trim().to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_inline_completions() {
        assert_eq!(
            clean_inline("```sql\nFROM users\n```", "SELECT * "),
            "FROM users"
        );
        assert_eq!(
            clean_inline("SELECT id FROM users", "SELECT id"),
            " FROM users"
        );
        assert_eq!(
            clean_inline("  WHERE active", "SELECT * FROM t "),
            "WHERE active"
        );
        assert_eq!(clean_inline("", "x"), "");
    }

    #[test]
    fn extracts_sql_block() {
        let a = extract_sql("```sql\nSELECT 1;\n```\nReturns one.");
        assert_eq!(a.sql, "SELECT 1;");
        assert_eq!(a.explanation, "Returns one.");
        let b = extract_sql("Here you go:\n```\nselect 2\n```");
        assert_eq!(b.sql, "select 2");
        assert_eq!(b.explanation, "Here you go:");
        let c = extract_sql("No such table.");
        assert!(c.sql.is_empty() && c.explanation == "No such table.");
    }

    #[test]
    fn ranks_mentioned_tables_first() {
        let t = |n: &str| TableInfo {
            schema: "public".into(),
            name: n.into(),
            kind: "table".into(),
        };
        let words = tokens("total revenue per user from orders");
        assert!(relevance(&t("orders"), &words) > relevance(&t("sessions"), &words));
        assert!(relevance(&t("users"), &words) > 0);
    }
}
