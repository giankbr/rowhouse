//! Apache Cassandra / ScyllaDB (beta) over the native CQL protocol (`scylla` crate).
//!
//! CQL has no transactions and no OFFSET, and only filters / sorts on key
//! columns cheaply, so: grid paging skips rows client-side, filters on other
//! columns add `ALLOW FILTERING`, sorting is applied to the fetched page, and
//! keyed grid edits use `IF EXISTS` (a lightweight transaction) so they never
//! create rows by accident.

use std::sync::Arc;

use futures_util::StreamExt;
use scylla::{
    client::{session::Session, session_builder::SessionBuilder},
    statement::Statement,
    value::{CqlValue, Row},
};
use serde_json::{Map, Number, Value};

use super::Expect;
use crate::{
    browse::{FilterOp, PageRequest},
    config::{TlsConfig, TlsMode},
    guard, ColumnInfo, Error, QueryResult, Result, ResultColumn, TableInfo,
};

pub struct Client {
    session: Session,
    keyspace: Option<String>,
}

fn err(e: impl std::fmt::Display) -> Error {
    Error::Other(format!("cql: {e}"))
}

const SYSTEM_KEYSPACES: &[&str] = &[
    "system",
    "system_auth",
    "system_schema",
    "system_distributed",
    "system_traces",
    "system_views",
    "system_virtual_schema",
    "system_distributed_everywhere",
    "system_replicated_keys",
];

/// First words allowed on read-only connections.
const READ_FIRST: &[&str] = &["select", "describe", "desc", "list", "use"];

impl Client {
    pub async fn connect(
        host: &str,
        port: u16,
        user: Option<&str>,
        password: Option<&str>,
        keyspace: Option<&str>,
        tls: Option<&TlsConfig>,
    ) -> Result<Self> {
        let nodes: Vec<String> = host
            .split(',')
            .map(str::trim)
            .filter(|h| !h.is_empty())
            .map(|h| {
                if h.contains(':') {
                    h.to_string()
                } else {
                    format!("{h}:{port}")
                }
            })
            .collect();
        let mut b = SessionBuilder::new()
            .known_nodes(&nodes)
            .connection_timeout(std::time::Duration::from_secs(10));
        if let Some(u) = user.filter(|u| !u.is_empty()) {
            b = b.user(u, password.unwrap_or(""));
        }
        let keyspace = keyspace
            .map(str::trim)
            .filter(|k| !k.is_empty())
            .map(str::to_string);
        if let Some(ks) = &keyspace {
            b = b.use_keyspace(ks, false);
        }
        if let Some(t) = tls.filter(|t| !matches!(t.mode, TlsMode::Disable | TlsMode::Prefer)) {
            b = b.tls_context(Some(rustls_config(t)?));
        }
        let session = b.build().await.map_err(err)?;
        Ok(Self { session, keyspace })
    }

    async fn fetch(&self, cql: &str, limit: usize) -> Result<QueryResult> {
        let mut st = Statement::new(cql.to_string());
        st.set_page_size(limit.clamp(1, 5_000) as i32);
        let pager = self.session.query_iter(st, &[]).await.map_err(err)?;
        let columns: Vec<ResultColumn> = pager
            .column_specs()
            .iter()
            .map(|c| ResultColumn {
                name: c.name().to_string(),
                data_type: type_name(&format!("{:?}", c.typ())),
            })
            .collect();
        let mut rows = Vec::new();
        let mut truncated = false;
        if !columns.is_empty() {
            let mut stream = pager.rows_stream::<Row>().map_err(err)?;
            while let Some(r) = stream.next().await {
                if rows.len() >= limit {
                    truncated = true;
                    break;
                }
                let r = r.map_err(err)?;
                rows.push(
                    r.columns
                        .iter()
                        .map(|v| v.as_ref().map(to_json).unwrap_or(Value::Null))
                        .collect(),
                );
            }
        }
        Ok(QueryResult {
            columns,
            rows,
            rows_affected: 0,
            truncated,
        })
    }

    pub async fn run(
        &self,
        script: &str,
        limit: usize,
        read_only: bool,
    ) -> Result<Vec<QueryResult>> {
        let stmts = statements(script);
        if read_only {
            for s in &stmts {
                check_read_only(s).map_err(Error::ReadOnly)?;
            }
        }
        let mut out = Vec::new();
        for s in stmts {
            out.push(self.fetch(&s, limit).await?);
        }
        Ok(out)
    }

    /// No transactions in CQL: statements run in order. Keyed UPDATE / DELETE get
    /// `IF EXISTS`, so a stale row is reported instead of silently upserted.
    pub async fn exec_tx(&self, stmts: Vec<(Expect, String)>) -> Result<u64> {
        let mut done = 0u64;
        for (e, s) in &stmts {
            let first = guard::first_keyword(s);
            let lwt = *e != Expect::Any && (first == "update" || first == "delete");
            let cql = if lwt {
                format!("{s} IF EXISTS")
            } else {
                s.clone()
            };
            let r = self.fetch(&cql, 10).await.map_err(|x| {
                Error::Other(format!(
                    "{x}\n{cql}\n({done} earlier statement(s) were already applied — Cassandra has no transactions)"
                ))
            })?;
            if lwt {
                let applied = r
                    .columns
                    .iter()
                    .position(|c| c.name == "[applied]")
                    .and_then(|i| r.rows.first().and_then(|row| row.get(i)))
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                if !applied {
                    return Err(Error::Other(format!(
                        "row not found, nothing changed by:\n{s}\n({done} earlier statement(s) were already applied — Cassandra has no transactions)"
                    )));
                }
            }
            done += 1;
        }
        Ok(done)
    }

    pub async fn tables(&self) -> Result<Vec<TableInfo>> {
        let mut out = Vec::new();
        for (sql, kind) in [
            (
                "SELECT keyspace_name, table_name FROM system_schema.tables",
                "table",
            ),
            (
                "SELECT keyspace_name, view_name FROM system_schema.views",
                "view",
            ),
        ] {
            let r = self.fetch(sql, 100_000).await?;
            for row in r.rows {
                let ks = row[0].as_str().unwrap_or_default().to_string();
                if SYSTEM_KEYSPACES.contains(&ks.as_str()) {
                    continue;
                }
                if self.keyspace.as_ref().is_some_and(|k| *k != ks) {
                    continue;
                }
                out.push(TableInfo {
                    schema: ks,
                    name: row[1].as_str().unwrap_or_default().to_string(),
                    kind: kind.into(),
                });
            }
        }
        out.sort_by(|a, b| (&a.schema, &a.name).cmp(&(&b.schema, &b.name)));
        Ok(out)
    }

    /// Partition key, then clustering columns, then the rest by name.
    pub async fn describe(&self, keyspace: &str, table: &str) -> Result<Vec<ColumnInfo>> {
        let q = |s: &str| s.replace('\'', "''");
        let r = self
            .fetch(
                &format!(
                    "SELECT column_name, type, kind, position FROM system_schema.columns \
                     WHERE keyspace_name = '{}' AND table_name = '{}'",
                    q(keyspace),
                    q(table)
                ),
                10_000,
            )
            .await?;
        let mut cols: Vec<(u8, i64, ColumnInfo)> = r
            .rows
            .into_iter()
            .map(|row| {
                let kind = row[2].as_str().unwrap_or("regular");
                let rank = match kind {
                    "partition_key" => 0,
                    "clustering" => 1,
                    "static" => 2,
                    _ => 3,
                };
                let name = row[0].as_str().unwrap_or_default().to_string();
                (
                    rank,
                    row[3].as_i64().unwrap_or(0),
                    ColumnInfo {
                        primary_key: rank < 2,
                        nullable: rank >= 2,
                        data_type: row[1].as_str().unwrap_or_default().to_string(),
                        default: None,
                        name,
                    },
                )
            })
            .collect();
        cols.sort_by(|a, b| (a.0, a.1, &a.2.name).cmp(&(b.0, b.1, &b.2.name)));
        Ok(cols.into_iter().map(|c| c.2).collect())
    }

    /// Grid page: filters become CQL relations (+ ALLOW FILTERING); there is no
    /// OFFSET, so earlier rows are skipped client-side; sort applies to the page.
    pub async fn page(
        &self,
        r: &PageRequest,
        target: &str,
        ident: impl Fn(&str) -> String,
    ) -> Result<(QueryResult, Option<u64>, String)> {
        let mut rel = Vec::new();
        for f in r.filters.iter().filter(|f| !f.column.is_empty()) {
            let c = ident(&f.column);
            let v = |s: &str| from_json_literal(&Value::String(s.trim().to_string()));
            rel.push(match f.op {
                FilterOp::Eq => format!("{c} = {}", v(&f.value)),
                FilterOp::Lt => format!("{c} < {}", v(&f.value)),
                FilterOp::Le => format!("{c} <= {}", v(&f.value)),
                FilterOp::Gt => format!("{c} > {}", v(&f.value)),
                FilterOp::Ge => format!("{c} >= {}", v(&f.value)),
                FilterOp::In => format!(
                    "{c} IN ({})",
                    f.value
                        .split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(v)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                other => {
                    return Err(Error::Other(format!(
                        "CQL can't filter with `{other:?}` — use =, <, >, or IN"
                    )))
                }
            });
        }
        let w = if rel.is_empty() {
            String::new()
        } else {
            format!(" WHERE {} ALLOW FILTERING", rel.join(" AND "))
        };
        let window = r.offset + r.limit;
        let sql = format!("SELECT * FROM {target}{w} LIMIT {window}");
        let mut res = self.fetch(&sql, window).await?;
        res.rows = res.rows.into_iter().skip(r.offset).collect();
        res.truncated = false;
        if let Some(s) = &r.sort {
            if let Some(i) = res.columns.iter().position(|c| c.name == s.column) {
                res.rows.sort_by(|a, b| {
                    let o = cmp_json(&a[i], &b[i]);
                    if s.desc {
                        o.reverse()
                    } else {
                        o
                    }
                });
            }
        }
        let total = if r.count {
            // counting a big table is a full scan; give up quietly if it times out
            self.fetch(&format!("SELECT COUNT(*) FROM {target}{w}"), 1)
                .await
                .ok()
                .and_then(|c| {
                    c.rows
                        .first()
                        .and_then(|row| row.first())
                        .and_then(Value::as_u64)
                })
        } else {
            None
        };
        let shown = if r.sort.is_some() {
            format!("{sql} -- page sorted in Rowhouse")
        } else {
            sql
        };
        Ok((res, total, shown))
    }
}

fn cmp_json(a: &Value, b: &Value) -> std::cmp::Ordering {
    match (a, b) {
        (Value::Null, Value::Null) => std::cmp::Ordering::Equal,
        (Value::Null, _) => std::cmp::Ordering::Less,
        (_, Value::Null) => std::cmp::Ordering::Greater,
        (Value::Number(x), Value::Number(y)) => x
            .as_f64()
            .partial_cmp(&y.as_f64())
            .unwrap_or(std::cmp::Ordering::Equal),
        _ => a.to_string().cmp(&b.to_string()),
    }
}

/// `fromJson('…')`: Cassandra converts the JSON to the column's type (strings are
/// accepted for every type), so the grid never needs to know CQL literal syntax.
pub fn from_json_literal(v: &Value) -> String {
    if v.is_null() {
        return "NULL".into();
    }
    let json = serde_json::to_string(v).unwrap_or_default();
    format!("fromJson('{}')", json.replace('\'', "''"))
}

/// Split a script, keeping `BEGIN BATCH … APPLY BATCH;` together.
pub fn statements(script: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut batch: Option<String> = None;
    for s in guard::split_statements(script) {
        let lower = s.trim().to_lowercase();
        match &mut batch {
            Some(b) => {
                b.push_str(";\n");
                b.push_str(s.trim());
                if lower.starts_with("apply batch") || lower.ends_with("apply batch") {
                    out.push(batch.take().unwrap_or_default());
                }
            }
            None if lower.starts_with("begin") && lower.contains("batch") => {
                if lower.ends_with("apply batch") {
                    out.push(s.trim().to_string());
                } else {
                    batch = Some(s.trim().to_string());
                }
            }
            None => out.push(s.trim().to_string()),
        }
    }
    if let Some(b) = batch {
        out.push(b);
    }
    out.into_iter().filter(|s| !s.is_empty()).collect()
}

pub fn check_read_only(stmt: &str) -> std::result::Result<(), String> {
    let first = guard::first_keyword(stmt);
    if READ_FIRST.contains(&first.as_str()) {
        Ok(())
    } else {
        Err(format!(
            "`{first}` statements are not allowed in read-only mode"
        ))
    }
}

/// `Native(Int)` / `Collection { frozen: false, typ: List(Native(Text)) }` → `int` / `list`
fn type_name(debug: &str) -> String {
    let lower = debug.to_lowercase();
    for k in ["list", "set", "map", "tuple", "userdefinedtype", "vector"] {
        if lower.contains(&format!("{k}(")) || lower.contains(&format!("{k} {{")) {
            return if k == "userdefinedtype" {
                "udt".into()
            } else {
                k.into()
            };
        }
    }
    lower
        .trim_start_matches("native(")
        .trim_end_matches(')')
        .to_string()
}

fn hex(b: &[u8]) -> String {
    let mut s = String::from("0x");
    for x in b {
        s.push_str(&format!("{x:02x}"));
    }
    s
}

fn display(v: &CqlValue) -> Value {
    Value::String(v.to_string().trim_matches('\'').to_string())
}

/// CQL value → JSON for the grid.
pub fn to_json(v: &CqlValue) -> Value {
    match v {
        CqlValue::Ascii(s) | CqlValue::Text(s) => Value::String(s.clone()),
        CqlValue::Boolean(b) => Value::Bool(*b),
        CqlValue::Blob(b) => Value::String(hex(b)),
        CqlValue::Counter(c) => Value::from(c.0),
        CqlValue::Double(f) => Number::from_f64(*f)
            .map(Value::Number)
            .unwrap_or(Value::String(f.to_string())),
        CqlValue::Float(f) => Number::from_f64(f64::from(*f))
            .map(Value::Number)
            .unwrap_or(Value::String(f.to_string())),
        CqlValue::Int(n) => Value::from(*n),
        CqlValue::BigInt(n) => Value::from(*n),
        CqlValue::SmallInt(n) => Value::from(*n),
        CqlValue::TinyInt(n) => Value::from(*n),
        CqlValue::Timestamp(t) => chrono::DateTime::from_timestamp_millis(t.0)
            .map(|d| Value::String(d.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)))
            .unwrap_or(Value::from(t.0)),
        CqlValue::Inet(ip) => Value::String(ip.to_string()),
        CqlValue::Uuid(u) => Value::String(u.to_string()),
        CqlValue::Timeuuid(_) => display(v),
        CqlValue::List(items) | CqlValue::Set(items) => {
            Value::Array(items.iter().map(to_json).collect())
        }
        CqlValue::Map(pairs) => {
            if pairs
                .iter()
                .all(|(k, _)| matches!(k, CqlValue::Text(_) | CqlValue::Ascii(_)))
            {
                Value::Object(
                    pairs
                        .iter()
                        .map(|(k, v)| {
                            (
                                to_json(k).as_str().unwrap_or_default().to_string(),
                                to_json(v),
                            )
                        })
                        .collect::<Map<_, _>>(),
                )
            } else {
                Value::Array(
                    pairs
                        .iter()
                        .map(|(k, v)| Value::Array(vec![to_json(k), to_json(v)]))
                        .collect(),
                )
            }
        }
        CqlValue::UserDefinedType { fields, .. } => Value::Object(
            fields
                .iter()
                .map(|(k, v)| (k.clone(), v.as_ref().map(to_json).unwrap_or(Value::Null)))
                .collect(),
        ),
        CqlValue::Tuple(items) => Value::Array(
            items
                .iter()
                .map(|v| v.as_ref().map(to_json).unwrap_or(Value::Null))
                .collect(),
        ),
        CqlValue::Empty => Value::Null,
        other => display(other),
    }
}

// ---------------------------------------------------------------- TLS

fn rustls_config(t: &TlsConfig) -> Result<Arc<rustls::ClientConfig>> {
    use rustls::pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer};
    crate::init_crypto();
    let path = |p: &Option<String>| {
        p.as_deref()
            .filter(|s| !s.trim().is_empty())
            .map(crate::tunnel::expand_home)
    };
    let builder = rustls::ClientConfig::builder();
    let builder = if t.mode == TlsMode::Require {
        // encrypted, not verified (self-signed clusters)
        builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoVerify(
                rustls::crypto::ring::default_provider(),
            )))
    } else {
        let mut roots = rustls::RootCertStore::empty();
        match path(&t.ca_cert) {
            Some(ca) => {
                for c in CertificateDer::pem_file_iter(&ca)
                    .map_err(|e| err(format!("CA certificate: {e}")))?
                {
                    roots
                        .add(c.map_err(|e| err(format!("CA certificate: {e}")))?)
                        .map_err(err)?;
                }
            }
            None => {
                for c in rustls_native_certs::load_native_certs().certs {
                    let _ = roots.add(c);
                }
            }
        }
        builder.with_root_certificates(roots)
    };
    let cfg = match (path(&t.client_cert), path(&t.client_key)) {
        (Some(cert), Some(key)) => {
            let chain = CertificateDer::pem_file_iter(&cert)
                .map_err(|e| err(format!("client certificate: {e}")))?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|e| err(format!("client certificate: {e}")))?;
            let key =
                PrivateKeyDer::from_pem_file(&key).map_err(|e| err(format!("client key: {e}")))?;
            builder.with_client_auth_cert(chain, key).map_err(err)?
        }
        _ => builder.with_no_client_auth(),
    };
    Ok(Arc::new(cfg))
}

#[derive(Debug)]
struct NoVerify(rustls::crypto::CryptoProvider);

impl rustls::client::danger::ServerCertVerifier for NoVerify {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> std::result::Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}
