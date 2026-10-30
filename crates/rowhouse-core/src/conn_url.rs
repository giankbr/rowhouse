//! Paste a connection string (`postgresql://user:pass@host:5432/db?sslmode=require`)
//! and get a profile. Neon / Supabase hosts are recognised automatically.

use url::Url;

use crate::{
    config::{ConnectionConfig, DbKind, TlsConfig, TlsMode},
    Error, Result,
};

fn decode(s: &str) -> String {
    url::form_urlencoded::parse(format!("x={s}").as_bytes())
        .next()
        .map(|(_, v)| v.into_owned())
        .unwrap_or_else(|| s.to_string())
}

/// Returns the profile and the password found in the URL (store it in the keychain, not the profile).
pub fn parse_connection_url(input: &str) -> Result<(ConnectionConfig, Option<String>)> {
    let input = input.trim();
    // jdbc:postgresql://… → postgresql://…
    let input = input.strip_prefix("jdbc:").unwrap_or(input);
    if input.starts_with("mongodb://") || input.starts_with("mongodb+srv://") {
        return Ok(mongo_url(input));
    }
    let url = Url::parse(input).map_err(|e| Error::Other(format!("not a connection URL: {e}")))?;
    let host = url.host_str().unwrap_or_default().to_string();
    let kind = match url.scheme() {
        "postgres" | "postgresql" | "pg" => {
            if host.ends_with(".neon.tech") {
                DbKind::Neon
            } else if host.ends_with(".supabase.co") || host.ends_with(".supabase.com") {
                DbKind::Supabase
            } else {
                DbKind::Postgres
            }
        }
        "cockroachdb" | "cockroach" => DbKind::Cockroach,
        "redshift" => DbKind::Redshift,
        "mysql" | "mysqlx" => {
            if host.ends_with(".tidbcloud.com") {
                DbKind::Tidb
            } else {
                DbKind::Mysql
            }
        }
        "mariadb" => DbKind::Mariadb,
        "sqlserver" | "mssql" => DbKind::Mssql,
        "clickhouse" | "ch" => DbKind::Clickhouse,
        "sqlite" | "file" => DbKind::Sqlite,
        "duckdb" => DbKind::Duckdb,
        other => return Err(Error::Other(format!("unsupported scheme `{other}://`"))),
    };

    let mut c = ConnectionConfig::new("", kind);
    if kind.is_file() {
        // sqlite:///abs/path.db  or sqlite://./rel.db
        let path = format!("{}{}", url.host_str().unwrap_or(""), url.path());
        c.file = Some(path);
        c.host = None;
        c.port = None;
        c.user = None;
        c.name = c
            .file
            .as_deref()
            .and_then(|f| f.rsplit('/').next())
            .unwrap_or("database")
            .to_string();
        return Ok((c, None));
    }

    c.host = Some(host.clone());
    if let Some(p) = url.port() {
        c.port = Some(p);
    }
    if !url.username().is_empty() {
        c.user = Some(decode(url.username()));
    }
    let db = url.path().trim_start_matches('/');
    if !db.is_empty() {
        c.database = Some(decode(db));
    }

    let mut tls = c.tls.clone().unwrap_or_default();
    let mut tls_set = c.tls.is_some();
    for (k, v) in url.query_pairs() {
        match k.as_ref() {
            "sslmode" | "ssl-mode" | "ssl_mode" => {
                tls_set = true;
                tls.mode = match v.to_lowercase().replace('_', "-").as_str() {
                    "disable" | "disabled" => TlsMode::Disable,
                    "allow" | "prefer" | "preferred" => TlsMode::Prefer,
                    "require" | "required" => TlsMode::Require,
                    "verify-ca" => TlsMode::VerifyCa,
                    "verify-full" | "verify-identity" => TlsMode::VerifyFull,
                    _ => tls.mode,
                };
            }
            "ssl" | "tls" | "encrypt" if v == "true" || v == "1" => {
                tls_set = true;
                tls.mode = TlsMode::Require;
            }
            "sslrootcert" | "ssl-ca" => (tls_set, tls.ca_cert) = (true, Some(v.into_owned())),
            "sslcert" | "ssl-cert" => (tls_set, tls.client_cert) = (true, Some(v.into_owned())),
            "sslkey" | "ssl-key" => (tls_set, tls.client_key) = (true, Some(v.into_owned())),
            "database" | "dbname" if c.database.is_none() => c.database = Some(v.into_owned()),
            _ => {}
        }
    }
    c.tls = tls_set
        .then_some(tls)
        .filter(|t| *t != TlsConfig::default() || matches!(kind, DbKind::Neon | DbKind::Supabase));

    c.name = match kind {
        DbKind::Neon => format!("Neon · {}", c.database.as_deref().unwrap_or("neondb")),
        DbKind::Supabase => format!("Supabase · {}", host.split('.').next().unwrap_or("project")),
        _ => format!(
            "{}@{host}",
            c.database.as_deref().unwrap_or(kind_label(kind))
        ),
    };
    let password = url.password().map(decode).filter(|p| !p.is_empty());
    Ok((c, password))
}

/// MongoDB URIs can list several hosts (`a:1,b:2`) and use `+srv`, so the whole
/// URI (minus the password) is kept as the host and handed to the driver.
fn mongo_url(input: &str) -> (ConnectionConfig, Option<String>) {
    let (scheme, rest) = input.split_once("://").unwrap_or(("mongodb", input));
    let auth_end = rest.find(['/', '?']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(auth_end);
    let (userinfo, hosts) = match authority.rfind('@') {
        Some(i) => (Some(&authority[..i]), &authority[i + 1..]),
        None => (None, authority),
    };
    let (user, password) = match userinfo {
        Some(u) => match u.split_once(':') {
            Some((u, p)) => (Some(u), Some(p)),
            None => (Some(u), None),
        },
        None => (None, None),
    };
    let db = tail
        .strip_prefix('/')
        .map(|t| t.split('?').next().unwrap_or(""))
        .filter(|d| !d.is_empty())
        .map(decode);
    let mut c = ConnectionConfig::new("", DbKind::Mongodb);
    c.host = Some(match user {
        Some(u) => format!("{scheme}://{u}@{hosts}{tail}"),
        None => format!("{scheme}://{hosts}{tail}"),
    });
    c.port = None;
    c.user = user.map(decode);
    c.database = db.clone();
    let first_host = hosts.split(',').next().unwrap_or(hosts);
    c.name = format!(
        "{}@{}",
        db.as_deref().unwrap_or("mongodb"),
        first_host.split(':').next().unwrap_or(first_host)
    );
    (c, password.map(decode).filter(|p| !p.is_empty()))
}

fn kind_label(k: DbKind) -> &'static str {
    match k {
        DbKind::Mssql => "sqlserver",
        DbKind::Clickhouse => "clickhouse",
        DbKind::Mysql | DbKind::Mariadb | DbKind::Tidb | DbKind::Singlestore => "mysql",
        _ => "postgres",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neon() {
        let (c, pw) = parse_connection_url(
            "postgresql://alex:AbC123%40x@ep-cool-darkness-123456.us-east-2.aws.neon.tech/neondb?sslmode=require&channel_binding=require",
        )
        .unwrap();
        assert_eq!(c.kind, DbKind::Neon);
        assert_eq!(
            c.host.as_deref(),
            Some("ep-cool-darkness-123456.us-east-2.aws.neon.tech")
        );
        assert_eq!(
            (c.port, c.user.as_deref(), c.database.as_deref()),
            (Some(5432), Some("alex"), Some("neondb"))
        );
        assert_eq!(pw.as_deref(), Some("AbC123@x"));
        assert_eq!(c.tls.unwrap().mode, TlsMode::Require);
        assert_eq!(c.name, "Neon · neondb");
    }

    #[test]
    fn supabase_pooler_and_plain() {
        let (c, pw) = parse_connection_url("postgres://postgres.abcdefgh:secret@aws-0-ap-southeast-1.pooler.supabase.com:6543/postgres").unwrap();
        assert_eq!((c.kind, c.port), (DbKind::Supabase, Some(6543)));
        assert_eq!(c.user.as_deref(), Some("postgres.abcdefgh"));
        assert_eq!(pw.as_deref(), Some("secret"));
        assert_eq!(
            c.tls.unwrap().mode,
            TlsMode::Require,
            "supabase defaults to TLS"
        );
        let (c, _) =
            parse_connection_url("postgresql://postgres@db.abcdefgh.supabase.co:5432/postgres")
                .unwrap();
        assert_eq!(c.kind, DbKind::Supabase);
        assert_eq!(c.name, "Supabase · db");
    }

    #[test]
    fn others() {
        let (c, _) = parse_connection_url(
            "mysql://root@127.0.0.1:3307/shop?ssl-mode=VERIFY_IDENTITY&ssl-ca=/etc/ca.pem",
        )
        .unwrap();
        assert_eq!((c.kind, c.port), (DbKind::Mysql, Some(3307)));
        let t = c.tls.unwrap();
        assert_eq!(
            (t.mode, t.ca_cert.as_deref()),
            (TlsMode::VerifyFull, Some("/etc/ca.pem"))
        );
        let (c, _) = parse_connection_url("jdbc:postgresql://localhost/app").unwrap();
        assert_eq!(
            (c.kind, c.port, c.tls),
            (DbKind::Postgres, Some(5432), None)
        );
        let (c, _) = parse_connection_url("sqlite:///Users/me/app.db").unwrap();
        assert_eq!(
            (c.kind, c.file.as_deref()),
            (DbKind::Sqlite, Some("/Users/me/app.db"))
        );
        assert!(parse_connection_url("oracle://x").is_err());
    }

    #[test]
    fn mongodb_uri_keeps_hosts_and_drops_password() {
        let (c, pw) = parse_connection_url(
            "mongodb+srv://app%40x:s3cr%2Ft@cluster0.abcde.mongodb.net/shop?retryWrites=true&w=majority",
        )
        .unwrap();
        assert_eq!(c.kind, DbKind::Mongodb);
        assert_eq!(
            c.host.as_deref(),
            Some(
                "mongodb+srv://app%40x@cluster0.abcde.mongodb.net/shop?retryWrites=true&w=majority"
            )
        );
        assert_eq!(c.user.as_deref(), Some("app@x"));
        assert_eq!(pw.as_deref(), Some("s3cr/t"));
        assert_eq!(c.database.as_deref(), Some("shop"));
        let (c, pw) = parse_connection_url("mongodb://h1:27017,h2:27018/?replicaSet=rs0").unwrap();
        assert_eq!(
            c.host.as_deref(),
            Some("mongodb://h1:27017,h2:27018/?replicaSet=rs0")
        );
        assert!(pw.is_none() && c.database.is_none());
    }
}
