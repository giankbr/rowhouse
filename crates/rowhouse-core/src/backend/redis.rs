//! Redis. The editor takes one command per line (`HGETALL user:1`,
//! `SCAN 0 MATCH user:* COUNT 100`); keys show up in the sidebar grouped by
//! their prefix, and opening a key shows its value in the shape of its type.

use redis::{aio::MultiplexedConnection, Value as RV};
use serde_json::{Number, Value};

use crate::{QueryResult, Result, ResultColumn, TableInfo};

#[derive(Clone)]
pub struct Client {
    conn: MultiplexedConnection,
}

/// Commands allowed on read-only connections and for MCP.
const READ_COMMANDS: &[&str] = &[
    "GET",
    "MGET",
    "GETRANGE",
    "STRLEN",
    "EXISTS",
    "TYPE",
    "TTL",
    "PTTL",
    "KEYS",
    "SCAN",
    "RANDOMKEY",
    "DBSIZE",
    "HGET",
    "HMGET",
    "HGETALL",
    "HKEYS",
    "HVALS",
    "HLEN",
    "HEXISTS",
    "HSCAN",
    "HSTRLEN",
    "LRANGE",
    "LLEN",
    "LINDEX",
    "LPOS",
    "SMEMBERS",
    "SCARD",
    "SISMEMBER",
    "SMISMEMBER",
    "SRANDMEMBER",
    "SSCAN",
    "SINTER",
    "SUNION",
    "SDIFF",
    "ZRANGE",
    "ZRANGEBYSCORE",
    "ZREVRANGE",
    "ZREVRANGEBYSCORE",
    "ZRANGEBYLEX",
    "ZSCORE",
    "ZMSCORE",
    "ZCARD",
    "ZCOUNT",
    "ZRANK",
    "ZREVRANK",
    "ZSCAN",
    "XRANGE",
    "XREVRANGE",
    "XLEN",
    "XINFO",
    "PING",
    "INFO",
    "TIME",
    "OBJECT",
    "MEMORY",
    "JSON.GET",
    "JSON.TYPE",
    "PFCOUNT",
    "GETBIT",
    "BITCOUNT",
    "GEOPOS",
    "GEODIST",
];

fn err(e: redis::RedisError) -> crate::Error {
    crate::Error::Other(format!("redis: {e}"))
}

/// Split a command line into arguments, honouring "double" and 'single' quotes.
pub fn split_args(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut started = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match quote {
            Some(q) if c == q => quote = None,
            Some('"') if c == '\\' => {
                if let Some(n) = chars.next() {
                    cur.push(match n {
                        'n' => '\n',
                        't' => '\t',
                        o => o,
                    });
                }
            }
            Some(_) => cur.push(c),
            None if c == '"' || c == '\'' => {
                quote = Some(c);
                started = true;
            }
            None if c.is_whitespace() => {
                if started {
                    out.push(std::mem::take(&mut cur));
                    started = false;
                }
            }
            None => {
                cur.push(c);
                started = true;
            }
        }
    }
    if started {
        out.push(cur);
    }
    out
}

fn scalar(v: &RV) -> Value {
    match v {
        RV::Nil => Value::Null,
        RV::Int(i) => (*i).into(),
        RV::Double(f) => Number::from_f64(*f)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        RV::Boolean(b) => Value::Bool(*b),
        RV::Okay => "OK".into(),
        RV::SimpleString(s) => s.clone().into(),
        RV::BulkString(b) => match std::str::from_utf8(b) {
            Ok(s) => s.into(),
            Err(_) => format!(
                "0x{}",
                b.iter().map(|x| format!("{x:02x}")).collect::<String>()
            )
            .into(),
        },
        RV::Array(a) | RV::Set(a) => Value::Array(a.iter().map(scalar).collect()),
        RV::Map(m) => Value::Object(m.iter().map(|(k, v)| (text(k), scalar(v))).collect()),
        other => format!("{other:?}").into(),
    }
}

fn text(v: &RV) -> String {
    match scalar(v) {
        Value::String(s) => s,
        o => o.to_string(),
    }
}

fn col(name: &str) -> ResultColumn {
    ResultColumn {
        name: name.into(),
        data_type: "redis".into(),
    }
}

/// Shape a reply as a grid, using the command to pick a sensible layout.
fn to_result(cmd: &str, v: RV, limit: usize, withscores: bool) -> QueryResult {
    let mut r = QueryResult::default();
    let pairs = |items: &[RV]| -> Vec<Vec<Value>> {
        items
            .chunks(2)
            .map(|p| p.iter().map(scalar).collect())
            .collect()
    };
    match (cmd, v) {
        ("HGETALL" | "CONFIG", RV::Array(a)) => {
            r.columns = vec![col("field"), col("value")];
            r.rows = pairs(&a);
        }
        ("ZRANGE" | "ZREVRANGE" | "ZRANGEBYSCORE" | "ZREVRANGEBYSCORE", RV::Array(a))
            if a.len() % 2 == 0
                && a.iter()
                    .skip(1)
                    .step_by(2)
                    .all(|x| matches!(x, RV::Double(_)) || text(x).parse::<f64>().is_ok())
                && !a.is_empty()
                && withscores =>
        {
            r.columns = vec![col("member"), col("score")];
            r.rows = pairs(&a);
        }
        (_, RV::Map(m)) => {
            r.columns = vec![col("field"), col("value")];
            r.rows = m.iter().map(|(k, v)| vec![scalar(k), scalar(v)]).collect();
        }
        ("SCAN" | "HSCAN" | "SSCAN" | "ZSCAN", RV::Array(a)) if a.len() == 2 => {
            let cursor = text(&a[0]);
            r.columns = vec![col("item"), col(&format!("next cursor: {cursor}"))];
            if let RV::Array(items) = &a[1] {
                r.rows = items.iter().map(|i| vec![scalar(i), Value::Null]).collect();
            }
        }
        ("XRANGE" | "XREVRANGE", RV::Array(a)) => {
            r.columns = vec![col("id"), col("fields")];
            r.rows = a
                .iter()
                .map(|e| match e {
                    RV::Array(p) if p.len() == 2 => {
                        let fields = match &p[1] {
                            RV::Array(f) => Value::Object(
                                f.chunks(2)
                                    .map(|kv| (text(&kv[0]), scalar(kv.get(1).unwrap_or(&RV::Nil))))
                                    .collect(),
                            ),
                            other => scalar(other),
                        };
                        vec![scalar(&p[0]), fields]
                    }
                    other => vec![scalar(other), Value::Null],
                })
                .collect();
        }
        (_, RV::Array(a)) | (_, RV::Set(a)) => {
            r.columns = vec![col("#"), col("value")];
            r.rows = a
                .iter()
                .enumerate()
                .map(|(i, v)| vec![(i as u64).into(), scalar(v)])
                .collect();
        }
        (_, v) => {
            r.columns = vec![col("value")];
            r.rows = vec![vec![scalar(&v)]];
        }
    }
    r.truncated = r.rows.len() > limit;
    r.rows.truncate(limit);
    r
}

impl Client {
    pub async fn connect(
        host: &str,
        port: u16,
        user: Option<&str>,
        password: Option<&str>,
        database: Option<&str>,
    ) -> Result<Self> {
        let enc = |s: &str| url::form_urlencoded::byte_serialize(s.as_bytes()).collect::<String>();
        let auth = match (
            user.filter(|u| !u.is_empty()),
            password.filter(|p| !p.is_empty()),
        ) {
            (Some(u), Some(p)) => format!("{}:{}@", enc(u), enc(p)),
            (None, Some(p)) => format!(":{}@", enc(p)),
            (Some(u), None) => format!("{}@", enc(u)),
            _ => String::new(),
        };
        let db = database
            .and_then(|d| d.trim().parse::<u32>().ok())
            .unwrap_or(0);
        let client =
            redis::Client::open(format!("redis://{auth}{host}:{port}/{db}")).map_err(err)?;
        let mut conn = client
            .get_multiplexed_async_connection()
            .await
            .map_err(err)?;
        redis::cmd("PING")
            .query_async::<RV>(&mut conn)
            .await
            .map_err(err)?;
        Ok(Self { conn })
    }

    pub async fn raw(&self, args: &[String]) -> Result<RV> {
        let mut c = self.conn.clone();
        let mut cmd = redis::cmd(&args[0]);
        for a in &args[1..] {
            cmd.arg(a);
        }
        cmd.query_async::<RV>(&mut c).await.map_err(err)
    }

    pub async fn run(
        &self,
        script: &str,
        limit: usize,
        read_only: bool,
    ) -> Result<Vec<QueryResult>> {
        let mut out = Vec::new();
        for line in script
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with("--"))
        {
            let args = split_args(line);
            let Some(first) = args.first() else { continue };
            let cmd = first.to_uppercase();
            if read_only && !READ_COMMANDS.contains(&cmd.as_str()) {
                return Err(crate::Error::ReadOnly(format!(
                    "`{cmd}` is not allowed in read-only mode"
                )));
            }
            if cmd == "SELECT" {
                // would silently switch the database for every tab sharing this connection
                return Err(crate::Error::Other(
                    "SELECT isn't supported — set the database number in the connection instead"
                        .into(),
                ));
            }
            let withscores = args.iter().any(|a| a.eq_ignore_ascii_case("WITHSCORES"));
            let v = self.raw(&args).await?;
            out.push(to_result(&cmd, v, limit, withscores));
        }
        Ok(out)
    }

    /// Up to `max` keys, grouped by prefix (`user:1` → schema `user`).
    pub async fn keys(&self, max: usize) -> Result<Vec<TableInfo>> {
        let mut cursor = "0".to_string();
        let mut keys = Vec::new();
        loop {
            let v = self
                .raw(&["SCAN".into(), cursor.clone(), "COUNT".into(), "500".into()])
                .await?;
            let RV::Array(a) = v else { break };
            cursor = text(&a[0]);
            if let Some(RV::Array(items)) = a.get(1) {
                keys.extend(items.iter().map(text));
            }
            if cursor == "0" || keys.len() >= max {
                break;
            }
        }
        keys.sort();
        keys.truncate(max);
        Ok(keys
            .into_iter()
            .map(|k| TableInfo {
                schema: k
                    .split_once(':')
                    .map(|(p, _)| p.to_string())
                    .unwrap_or_else(|| "keys".into()),
                name: k,
                kind: "table".into(),
            })
            .collect())
    }

    /// One page of a key's value, plus its total size.
    pub async fn key_page(
        &self,
        key: &str,
        offset: usize,
        limit: usize,
    ) -> Result<(QueryResult, Option<u64>, String)> {
        let ty = text(&self.raw(&["TYPE".into(), key.into()]).await?);
        let (start, stop) = (
            offset.to_string(),
            (offset + limit).saturating_sub(1).to_string(),
        );
        let q = |s: &str| {
            if s.chars().any(|c| c.is_whitespace() || c == '"') {
                format!("\"{}\"", s.replace('"', "\\\""))
            } else {
                s.to_string()
            }
        };
        let (cmd, len_cmd): (Vec<String>, Option<&str>) = match ty.as_str() {
            "hash" => (vec!["HGETALL".into(), key.into()], Some("HLEN")),
            "list" => (vec!["LRANGE".into(), key.into(), start, stop], Some("LLEN")),
            "set" => (vec!["SMEMBERS".into(), key.into()], Some("SCARD")),
            "zset" => (
                vec![
                    "ZRANGE".into(),
                    key.into(),
                    start,
                    stop,
                    "WITHSCORES".into(),
                ],
                Some("ZCARD"),
            ),
            "stream" => (
                vec![
                    "XRANGE".into(),
                    key.into(),
                    "-".into(),
                    "+".into(),
                    "COUNT".into(),
                    (offset + limit).to_string(),
                ],
                Some("XLEN"),
            ),
            "none" => return Err(crate::Error::Other(format!("key `{key}` doesn't exist"))),
            _ => (vec!["GET".into(), key.into()], None),
        };
        let shown = cmd.iter().map(|a| q(a)).collect::<Vec<_>>().join(" ");
        let mut r = to_result(&cmd[0], self.raw(&cmd).await?, usize::MAX, ty == "zset");
        if matches!(ty.as_str(), "hash" | "set" | "stream") {
            // these are fetched whole; page in memory
            r.rows = r.rows.into_iter().skip(offset).take(limit).collect();
        }
        if ty == "list" {
            for (i, row) in r.rows.iter_mut().enumerate() {
                row[0] = ((offset + i) as u64).into();
            }
        }
        let total = match len_cmd {
            Some(c) => match self.raw(&[c.into(), key.into()]).await? {
                RV::Int(n) => Some(n as u64),
                _ => None,
            },
            None => Some(1),
        };
        Ok((r, total, shown))
    }
}

#[cfg(test)]
mod tests {
    use super::split_args;

    #[test]
    fn args() {
        assert_eq!(
            split_args(r#"SET "user name" 'a b' plain"#),
            vec!["SET", "user name", "a b", "plain"]
        );
        assert_eq!(split_args(r#"SET k "x\"y\n""#), vec!["SET", "k", "x\"y\n"]);
        assert_eq!(split_args("  HGETALL   user:1 "), vec!["HGETALL", "user:1"]);
        assert_eq!(split_args(r#"SET k """#), vec!["SET", "k", ""]);
    }
}
