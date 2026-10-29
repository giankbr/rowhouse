//! Microsoft SQL Server / Azure SQL via tiberius (TDS 7.3+).
//! One connection guarded by a mutex: TDS needs each result stream fully
//! drained before the next request.

use std::sync::Arc;

use futures_util::TryStreamExt;
use serde_json::{Number, Value};
use tiberius::{AuthMethod, ColumnData, Config, FromSql, QueryItem};
use tokio::{net::TcpStream, sync::Mutex};
use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};

use super::Expect;
use crate::{Error, QueryResult, Result, ResultColumn};

type Tds = tiberius::Client<Compat<TcpStream>>;

#[derive(Clone)]
pub struct Client {
    inner: Arc<Mutex<Tds>>,
}

fn err(e: tiberius::error::Error) -> Error {
    Error::Other(format!("sqlserver: {e}"))
}

fn num_f64(f: f64) -> Value {
    Number::from_f64(f)
        .map(Value::Number)
        .unwrap_or(Value::String(f.to_string()))
}

fn to_json(d: ColumnData<'static>) -> Value {
    use ColumnData::*;
    let s = |o: Option<std::string::String>| o.map(Value::String).unwrap_or(Value::Null);
    match &d {
        U8(v) => v.map(Value::from).unwrap_or(Value::Null),
        I16(v) => v.map(Value::from).unwrap_or(Value::Null),
        I32(v) => v.map(Value::from).unwrap_or(Value::Null),
        I64(v) => v.map(Value::from).unwrap_or(Value::Null),
        F32(v) => v.map(|f| num_f64(f as f64)).unwrap_or(Value::Null),
        F64(v) => v.map(num_f64).unwrap_or(Value::Null),
        Bit(v) => v.map(Value::Bool).unwrap_or(Value::Null),
        String(v) => s(v.as_ref().map(|c| c.to_string())),
        Guid(v) => s(v.map(|g| g.to_string())),
        Binary(v) => s(v.as_ref().map(|b| {
            format!(
                "0x{}",
                b.iter()
                    .map(|x| format!("{x:02x}"))
                    .collect::<std::string::String>()
            )
        })),
        Numeric(v) => s(v.map(|n| n.to_string())),
        Xml(v) => s(v.as_ref().map(|x| x.to_string())),
        DateTime(_) | SmallDateTime(_) | DateTime2(_) => s(chrono::NaiveDateTime::from_sql(&d)
            .ok()
            .flatten()
            .map(|t| t.to_string())),
        Date(_) => s(chrono::NaiveDate::from_sql(&d)
            .ok()
            .flatten()
            .map(|t| t.to_string())),
        Time(_) => s(chrono::NaiveTime::from_sql(&d)
            .ok()
            .flatten()
            .map(|t| t.to_string())),
        DateTimeOffset(_) => s(chrono::DateTime::<chrono::FixedOffset>::from_sql(&d)
            .ok()
            .flatten()
            .map(|t| t.to_rfc3339())),
    }
}

impl Client {
    pub async fn connect(
        host: &str,
        port: u16,
        user: Option<&str>,
        password: Option<&str>,
        database: Option<&str>,
        tls: Option<&crate::config::TlsConfig>,
    ) -> Result<Self> {
        use crate::config::TlsMode;
        let mut cfg = Config::new();
        cfg.host(host);
        cfg.port(port);
        if let Some(db) = database.filter(|d| !d.is_empty()) {
            cfg.database(db);
        }
        cfg.authentication(AuthMethod::sql_server(
            user.unwrap_or("sa"),
            password.unwrap_or_default(),
        ));
        // Default like most DB GUIs: encrypt, but accept self-signed server certs.
        // verify-ca / verify-full validate against the given CA (or the system store).
        let mode = tls.map(|t| t.mode).unwrap_or(TlsMode::Prefer);
        cfg.encryption(match mode {
            TlsMode::Disable => tiberius::EncryptionLevel::NotSupported,
            TlsMode::Prefer => tiberius::EncryptionLevel::On,
            _ => tiberius::EncryptionLevel::Required,
        });
        match (
            mode,
            tls.and_then(|t| t.ca_cert.as_deref())
                .filter(|s| !s.is_empty()),
        ) {
            (TlsMode::VerifyCa | TlsMode::VerifyFull, Some(ca)) => {
                cfg.trust_cert_ca(crate::tunnel::expand_home(ca))
            }
            (TlsMode::VerifyCa | TlsMode::VerifyFull, None) => {}
            _ => cfg.trust_cert(),
        }
        let tcp = TcpStream::connect(cfg.get_addr()).await?;
        tcp.set_nodelay(true)?;
        let client = match tiberius::Client::connect(cfg.clone(), tcp.compat_write()).await {
            Ok(c) => c,
            // Azure SQL may redirect to another gateway.
            Err(tiberius::error::Error::Routing { host, port }) => {
                cfg.host(&host);
                cfg.port(port);
                let tcp = TcpStream::connect(cfg.get_addr()).await?;
                tcp.set_nodelay(true)?;
                tiberius::Client::connect(cfg, tcp.compat_write())
                    .await
                    .map_err(err)?
            }
            Err(e) => return Err(err(e)),
        };
        Ok(Self {
            inner: Arc::new(Mutex::new(client)),
        })
    }

    async fn run_on(c: &mut Tds, sql: &str, limit: usize) -> Result<Vec<QueryResult>> {
        let mut stream = c.simple_query(sql).await.map_err(err)?;
        let mut out: Vec<QueryResult> = Vec::new();
        while let Some(item) = stream.try_next().await.map_err(err)? {
            match item {
                QueryItem::Metadata(meta) => out.push(QueryResult {
                    columns: meta
                        .columns()
                        .iter()
                        .map(|c| ResultColumn {
                            name: c.name().to_string(),
                            data_type: format!("{:?}", c.column_type()),
                        })
                        .collect(),
                    ..Default::default()
                }),
                QueryItem::Row(row) => {
                    if let Some(cur) = out.last_mut() {
                        // keep draining the stream (TDS requirement) but stop storing
                        if cur.rows.len() >= limit {
                            cur.truncated = true;
                        } else {
                            cur.rows.push(row.into_iter().map(to_json).collect());
                        }
                    }
                }
            }
        }
        if out.is_empty() {
            out.push(QueryResult::default());
        }
        Ok(out)
    }

    pub async fn run(&self, sql: &str, limit: usize, read_only: bool) -> Result<Vec<QueryResult>> {
        let mut c = self.inner.lock().await;
        if !read_only {
            return Self::run_on(&mut c, sql, limit).await;
        }
        // Writes inside the transaction are rolled back.
        c.simple_query("BEGIN TRANSACTION")
            .await
            .map_err(err)?
            .into_results()
            .await
            .map_err(err)?;
        let r = Self::run_on(&mut c, sql, limit).await;
        c.simple_query("IF @@TRANCOUNT > 0 ROLLBACK TRANSACTION")
            .await
            .map_err(err)?
            .into_results()
            .await
            .map_err(err)?;
        r
    }

    pub async fn exec_tx(&self, stmts: Vec<(Expect, String)>) -> Result<u64> {
        let mut c = self.inner.lock().await;
        c.simple_query("BEGIN TRANSACTION")
            .await
            .map_err(err)?
            .into_results()
            .await
            .map_err(err)?;
        let mut total = 0;
        for (expect, s) in &stmts {
            let n = match c.execute(s.as_str(), &[]).await {
                Ok(r) => r.total(),
                Err(e) => {
                    let _ = c.simple_query("ROLLBACK TRANSACTION").await;
                    return Err(err(e));
                }
            };
            if !expect.check(n) {
                c.simple_query("ROLLBACK TRANSACTION")
                    .await
                    .map_err(err)?
                    .into_results()
                    .await
                    .map_err(err)?;
                return Err(Error::Other(format!(
                    "expected 1 row, {n} affected — rolled back.\n{s}"
                )));
            }
            total += n;
        }
        c.simple_query("COMMIT TRANSACTION")
            .await
            .map_err(err)?
            .into_results()
            .await
            .map_err(err)?;
        Ok(total)
    }
}
