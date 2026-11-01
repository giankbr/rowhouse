//! Snowflake through the SQL API v2 (`/api/v2/statements`).
//!
//! Auth: key-pair (RS256 JWT signed with the user's PKCS#8 private key) or a
//! programmatic access token. Each statement is one request; long ones come
//! back `202` with a handle that we poll, big results arrive in partitions.

use serde_json::{json, Value};

use super::Expect;
use crate::{guard, Error, QueryResult, Result, ResultColumn};

pub struct Client {
    http: reqwest::Client,
    base: String,
    auth: Auth,
    context: Context,
}

#[derive(Default)]
struct Context {
    database: Option<String>,
    schema: Option<String>,
    warehouse: Option<String>,
    role: Option<String>,
}

enum Auth {
    KeyPair {
        key: ring::signature::RsaKeyPair,
        /// `ACCOUNT.USER`
        sub: String,
        /// `ACCOUNT.USER.SHA256:<fingerprint>`
        iss: String,
    },
    Pat(String),
}

fn err(msg: impl std::fmt::Display) -> Error {
    Error::Other(format!("snowflake: {msg}"))
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

/// DER length prefix.
fn der_len(n: usize, out: &mut Vec<u8>) {
    if n < 0x80 {
        out.push(n as u8);
    } else {
        let bytes: Vec<u8> = n
            .to_be_bytes()
            .into_iter()
            .skip_while(|b| *b == 0)
            .collect();
        out.push(0x80 | bytes.len() as u8);
        out.extend(bytes);
    }
}

fn der(tag: u8, body: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    der_len(body.len(), &mut out);
    out.extend_from_slice(body);
    out
}

/// PKCS#1 RSAPublicKey → SubjectPublicKeyInfo DER (what Snowflake fingerprints).
pub fn spki(pkcs1: &[u8]) -> Vec<u8> {
    // SEQUENCE { OID 1.2.840.113549.1.1.1, NULL }
    let alg = der(
        0x30,
        &[
            0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01, 0x05, 0x00,
        ],
    );
    let mut bits = vec![0u8];
    bits.extend_from_slice(pkcs1);
    let mut body = alg;
    body.extend(der(0x03, &bits));
    der(0x30, &body)
}

/// The account part of the URL, upper-cased, as Snowflake wants it in the JWT
/// (`xy12345.ap-southeast-1` → `XY12345`; `myorg-acct` stays whole).
pub fn jwt_account(account: &str) -> String {
    let a = account
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/')
        .trim_end_matches(".snowflakecomputing.com");
    let a = if a.contains(".global") {
        a.split('-').next().unwrap_or(a)
    } else {
        a.split('.').next().unwrap_or(a)
    };
    a.to_uppercase()
}

fn pem_der(pem: &str) -> Result<Vec<u8>> {
    if pem.contains("ENCRYPTED PRIVATE KEY") {
        return Err(err(
            "the private key is encrypted — export it unencrypted (openssl pkcs8 -topk8 -nocrypt)",
        ));
    }
    if pem.contains("BEGIN RSA PRIVATE KEY") {
        return Err(err(
            "convert the key to PKCS#8 first: openssl pkcs8 -topk8 -nocrypt -in key.pem -out rsa_key.p8",
        ));
    }
    let body: String = pem
        .lines()
        .filter(|l| !l.starts_with("-----"))
        .collect::<Vec<_>>()
        .join("");
    super::libsql::base64_decode(&body).ok_or_else(|| err("the private key isn't valid PEM"))
}

pub struct Options<'a> {
    pub account: &'a str,
    pub user: &'a str,
    pub private_key_pem: Option<&'a str>,
    pub token: Option<&'a str>,
    pub database: Option<&'a str>,
    pub warehouse: Option<&'a str>,
    pub role: Option<&'a str>,
}

fn opt(s: Option<&str>) -> Option<String> {
    s.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

impl Client {
    pub async fn connect(o: Options<'_>) -> Result<Self> {
        let account = o.account.trim();
        if account.is_empty() {
            return Err(err("enter the account identifier (e.g. myorg-myaccount)"));
        }
        // ROWHOUSE_SNOWFLAKE_ENDPOINT: point at a proxy or a test server
        let base = if let Ok(e) = std::env::var("ROWHOUSE_SNOWFLAKE_ENDPOINT") {
            e.trim_end_matches('/').to_string()
        } else if account.starts_with("http://") || account.starts_with("https://") {
            account.trim_end_matches('/').to_string()
        } else {
            format!(
                "https://{}.snowflakecomputing.com",
                account.trim_end_matches(".snowflakecomputing.com")
            )
        };
        let user = o.user.trim().to_uppercase();
        let auth = match (o.private_key_pem.filter(|k| !k.trim().is_empty()), o.token) {
            (Some(pem), _) => {
                if user.is_empty() {
                    return Err(err("enter the user the key belongs to"));
                }
                let key = ring::signature::RsaKeyPair::from_pkcs8(&pem_der(pem)?)
                    .map_err(|e| err(format!("unreadable private key: {e}")))?;
                let fp = ring::digest::digest(
                    &ring::digest::SHA256,
                    &spki(ring::signature::KeyPair::public_key(&key).as_ref()),
                );
                let acct = jwt_account(account);
                let sub = format!("{acct}.{user}");
                let iss = format!("{sub}.SHA256:{}", crate::sync::base64_encode(fp.as_ref()));
                Auth::KeyPair { key, sub, iss }
            }
            (None, Some(t)) if !t.trim().is_empty() => Auth::Pat(t.trim().to_string()),
            _ => return Err(err(
                "pick the private key file (key-pair auth) or paste a programmatic access token",
            )),
        };
        // DATABASE or DATABASE.SCHEMA
        let (database, schema) = match opt(o.database) {
            Some(d) => match d.split_once('.') {
                Some((db, sc)) => (Some(db.to_string()), opt(Some(sc))),
                None => (Some(d), None),
            },
            None => (None, None),
        };
        let c = Self {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(300))
                .build()
                .map_err(err)?,
            base,
            auth,
            context: Context {
                database,
                schema,
                warehouse: opt(o.warehouse),
                role: opt(o.role),
            },
        };
        c.statement("SELECT 1", 1).await?;
        Ok(c)
    }

    fn authorize(&self, req: reqwest::RequestBuilder) -> Result<reqwest::RequestBuilder> {
        Ok(match &self.auth {
            Auth::Pat(t) => req.bearer_auth(t).header(
                "X-Snowflake-Authorization-Token-Type",
                "PROGRAMMATIC_ACCESS_TOKEN",
            ),
            Auth::KeyPair { key, sub, iss } => {
                let iat = now();
                let header = b64url(br#"{"alg":"RS256","typ":"JWT"}"#);
                let claims = b64url(
                    json!({ "iss": iss, "sub": sub, "iat": iat, "exp": iat + 3000 })
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
                .map_err(|_| err("couldn't sign the request"))?;
                req.bearer_auth(format!("{input}.{}", b64url(&sig)))
                    .header("X-Snowflake-Authorization-Token-Type", "KEYPAIR_JWT")
            }
        }
        .header("Accept", "application/json")
        .header("User-Agent", "Rowhouse"))
    }

    async fn send(&self, req: reqwest::RequestBuilder) -> Result<(u16, Value)> {
        let res = self.authorize(req)?.send().await.map_err(err)?;
        let status = res.status().as_u16();
        let text = res.text().await.map_err(err)?;
        let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        if status >= 400 {
            let msg = v["message"]
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| format!("HTTP {status}: {}", text.trim()));
            return Err(err(msg));
        }
        Ok((status, v))
    }

    async fn statement(&self, sql: &str, limit: usize) -> Result<QueryResult> {
        let mut body = json!({ "statement": sql, "timeout": 300 });
        let c = &self.context;
        for (k, v) in [
            ("database", &c.database),
            ("schema", &c.schema),
            ("warehouse", &c.warehouse),
            ("role", &c.role),
        ] {
            if let Some(v) = v {
                body[k] = json!(v.to_uppercase());
            }
        }
        let (mut status, mut v) = self
            .send(
                self.http
                    .post(format!("{}/api/v2/statements", self.base))
                    .json(&body),
            )
            .await?;
        let handle = v["statementHandle"].as_str().unwrap_or("").to_string();
        let url = format!("{}/api/v2/statements/{handle}", self.base);
        while status == 202 {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            (status, v) = self.send(self.http.get(&url)).await?;
        }
        let meta = &v["resultSetMetaData"];
        let types = meta["rowType"].as_array().cloned().unwrap_or_default();
        let columns = types
            .iter()
            .map(|t| ResultColumn {
                name: t["name"].as_str().unwrap_or("").to_string(),
                data_type: type_name(t),
            })
            .collect();
        let mut raw: Vec<Value> = v["data"].as_array().cloned().unwrap_or_default();
        let parts = meta["partitionInfo"].as_array().map(Vec::len).unwrap_or(1);
        let mut p = 1;
        while p < parts && raw.len() <= limit {
            let (_, page) = self
                .send(self.http.get(format!("{url}?partition={p}")))
                .await?;
            raw.extend(page["data"].as_array().cloned().unwrap_or_default());
            p += 1;
        }
        let total = meta["numRows"].as_u64().unwrap_or(raw.len() as u64) as usize;
        let truncated = total > limit || raw.len() > limit;
        raw.truncate(limit);
        let rows = raw
            .iter()
            .map(|r| {
                r.as_array()
                    .into_iter()
                    .flatten()
                    .zip(&types)
                    .map(|(cell, t)| convert(t, cell))
                    .collect()
            })
            .collect();
        // DML reports counts as a one-row result ("number of rows updated")
        let affected = v["stats"]["numRowsInserted"].as_u64().unwrap_or(0)
            + v["stats"]["numRowsUpdated"].as_u64().unwrap_or(0)
            + v["stats"]["numRowsDeleted"].as_u64().unwrap_or(0);
        Ok(QueryResult {
            columns,
            rows,
            rows_affected: affected,
            truncated,
        })
    }

    pub async fn run(&self, sql: &str, limit: usize, read_only: bool) -> Result<Vec<QueryResult>> {
        if read_only {
            guard::check_read_only(sql).map_err(Error::ReadOnly)?;
        }
        let mut out = Vec::new();
        for stmt in guard::split_statements(sql) {
            out.push(self.statement(&stmt, limit).await?);
        }
        Ok(out)
    }

    /// One Snowflake Scripting block: a transaction where each statement's
    /// `SQLROWCOUNT` is checked, rolling back and raising on a mismatch.
    pub async fn exec_tx(&self, stmts: Vec<(Expect, String)>) -> Result<u64> {
        if stmts.iter().any(|(_, s)| s.contains("$$")) {
            return Err(err("statements containing $$ can't run in an edit batch"));
        }
        let mut script = String::from(
            "EXECUTE IMMEDIATE $$\nDECLARE\n  wrong_count EXCEPTION (-20001, 'unexpected number of rows changed — rolled back');\nBEGIN\n  BEGIN TRANSACTION;\n",
        );
        for (expect, s) in &stmts {
            script.push_str("  ");
            script.push_str(s.trim().trim_end_matches(';'));
            script.push_str(";\n");
            let cond = match expect {
                Expect::Any => None,
                Expect::One => Some("SQLROWCOUNT <> 1".to_string()),
                Expect::AtMostOne => Some("SQLROWCOUNT > 1".to_string()),
                Expect::Exactly(n) => Some(format!("SQLROWCOUNT <> {n}")),
            };
            if let Some(c) = cond {
                script.push_str(&format!(
                    "  IF ({c}) THEN ROLLBACK; RAISE wrong_count; END IF;\n"
                ));
            }
        }
        script.push_str("  COMMIT;\nEND;\n$$");
        self.statement(&script, 0).await?;
        Ok(stmts.len() as u64)
    }
}

fn type_name(t: &Value) -> String {
    let ty = t["type"].as_str().unwrap_or("").to_uppercase();
    match ty.as_str() {
        "FIXED" => match (t["precision"].as_u64(), t["scale"].as_u64()) {
            (Some(p), Some(s)) => format!("NUMBER({p},{s})"),
            _ => "NUMBER".into(),
        },
        "TEXT" => match t["length"].as_u64() {
            Some(n) if n < 16_777_216 => format!("VARCHAR({n})"),
            _ => "VARCHAR".into(),
        },
        "REAL" => "FLOAT".into(),
        other => other.into(),
    }
}

/// `seconds.nanos` → `YYYY-MM-DD HH:MM:SS[.fff]`
fn epoch_text(s: &str, with_offset_minutes: Option<i64>) -> Option<String> {
    let (secs, frac) = s.split_once('.').unwrap_or((s, "0"));
    let secs: i64 = secs.parse().ok()?;
    let nanos: u32 = format!("{frac:0<9}")[..9].parse().ok()?;
    let t = chrono::DateTime::from_timestamp(secs, nanos)?;
    let frac = if nanos == 0 { "" } else { "%.f" };
    Some(match with_offset_minutes {
        Some(off) => {
            let tz = chrono::FixedOffset::east_opt((off * 60) as i32)?;
            t.with_timezone(&tz)
                .format(&format!("%Y-%m-%d %H:%M:%S{frac} %:z"))
                .to_string()
        }
        None => t
            .naive_utc()
            .format(&format!("%Y-%m-%d %H:%M:%S{frac}"))
            .to_string(),
    })
}

/// The SQL API returns every value as a string; restore JSON types.
pub fn convert(t: &Value, v: &Value) -> Value {
    let Some(s) = v.as_str() else {
        return v.clone();
    };
    let ty = t["type"].as_str().unwrap_or("").to_lowercase();
    let parsed = match ty.as_str() {
        "fixed" if t["scale"].as_u64().unwrap_or(0) == 0 => s
            .parse::<i64>()
            .ok()
            .filter(|n| n.unsigned_abs() < (1u64 << 53))
            .map(Value::from),
        "real" => s
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map(Value::Number),
        "boolean" => Some(Value::Bool(s.eq_ignore_ascii_case("true"))),
        "date" => s
            .parse::<i64>()
            .ok()
            .and_then(|d| chrono::DateTime::from_timestamp(d * 86_400, 0))
            .map(|d| Value::String(d.format("%Y-%m-%d").to_string())),
        "time" => epoch_text(s, None).map(|t| Value::String(t[11..].to_string())),
        "timestamp_ntz" => epoch_text(s, None).map(Value::String),
        "timestamp_ltz" => epoch_text(s, Some(0)).map(Value::String),
        "timestamp_tz" => {
            // "seconds.nanos offset" where offset is minutes + 1440
            let (ts, off) = s.split_once(' ').unwrap_or((s, "1440"));
            let off = off.parse::<i64>().unwrap_or(1440) - 1440;
            epoch_text(ts, Some(off)).map(Value::String)
        }
        "variant" | "object" | "array" => serde_json::from_str(s).ok(),
        _ => None,
    };
    parsed.unwrap_or_else(|| v.clone())
}
