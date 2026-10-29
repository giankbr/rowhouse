//! Import connection profiles from other clients so switching is painless.
//!
//! Supported: DBeaver (`data-sources.json`), TablePlus (`Connections.plist`
//! or an exported `.tableplusconnection`/JSON), Navicat (`.ncx` export).
//! Passwords are *not* imported — they live encrypted in each tool's own
//! store — so the user re-enters them once in Rowhouse.

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

use crate::{
    config::{ConnectionConfig, DbKind, Environment, SshAuth, SshConfig},
    Error, Result,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Dbeaver,
    Tableplus,
    Navicat,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub config: ConnectionConfig,
    /// e.g. "Oracle is not supported yet" — shown greyed out in the UI.
    pub skipped_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectedFile {
    pub source: Source,
    pub path: String,
}

/// Map a free-form driver/provider name to a [`DbKind`].
fn kind_from(name: &str) -> Option<DbKind> {
    let n = name.to_lowercase().replace([' ', '-', '_'], "");
    let has = |s: &str| n.contains(s);
    Some(if has("cockroach") {
        DbKind::Cockroach
    } else if has("redshift") {
        DbKind::Redshift
    } else if has("greenplum") {
        DbKind::Greenplum
    } else if has("supabase") {
        DbKind::Supabase
    } else if has("neon") {
        DbKind::Neon
    } else if has("postgres") || n == "pg" || has("timescale") {
        DbKind::Postgres
    } else if has("mariadb") {
        DbKind::Mariadb
    } else if has("tidb") {
        DbKind::Tidb
    } else if has("singlestore") || has("memsql") {
        DbKind::Singlestore
    } else if has("mysql") || has("planetscale") {
        DbKind::Mysql
    } else if has("sqlite") {
        DbKind::Sqlite
    } else if has("duckdb") {
        DbKind::Duckdb
    } else if has("sqlserver") || has("mssql") || has("azuresql") || has("microsoftsql") {
        DbKind::Mssql
    } else if has("clickhouse") {
        DbKind::Clickhouse
    } else {
        return None;
    })
}

fn env_from(name: &str) -> Environment {
    match name.to_lowercase().as_str() {
        s if s.starts_with("prod") => Environment::Production,
        s if s.starts_with("stag") => Environment::Staging,
        s if s.starts_with("dev") || s.starts_with("test") => Environment::Development,
        s if s.starts_with("local") => Environment::Local,
        _ => Environment::None,
    }
}

fn candidate(
    name: &str,
    kind: Option<DbKind>,
    raw_kind: &str,
) -> (ConnectionConfig, Option<String>) {
    let k = kind.unwrap_or(DbKind::Postgres);
    let mut c = ConnectionConfig::new(name, k);
    c.host = None;
    c.user = None;
    let skipped = kind
        .is_none()
        .then(|| format!("{raw_kind} is not supported yet"));
    (c, skipped)
}

fn finish(mut c: ConnectionConfig) -> ConnectionConfig {
    if c.port.is_none() {
        c.port = c.kind.default_port();
    }
    if c.kind.is_file() {
        c.host = None;
        c.port = None;
    }
    // production profiles start read-only, like a fresh Rowhouse connection
    if c.env == Environment::Production {
        c.read_only = true;
    }
    c
}

fn opt(s: Option<&str>) -> Option<String> {
    s.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn port(v: Option<&Value>) -> Option<u16> {
    match v? {
        Value::Number(n) => n.as_u64().and_then(|p| u16::try_from(p).ok()),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// `jdbc:sqlite:/path/to.db` → `/path/to.db`
fn path_from_jdbc(url: &str) -> Option<String> {
    let rest = url.strip_prefix("jdbc:")?;
    let (_, path) = rest.split_once(':')?;
    Some(path.trim_start_matches("//").to_string())
}

// ---------------------------------------------------------------- DBeaver ---

pub fn parse_dbeaver(json: &str) -> Result<Vec<Candidate>> {
    let v: Value = serde_json::from_str(json)?;
    let conns = v["connections"]
        .as_object()
        .ok_or_else(|| Error::Other("not a DBeaver data-sources.json".into()))?;
    let mut out = Vec::new();
    for (_, c) in conns {
        let cfg = &c["configuration"];
        let raw = format!(
            "{} {}",
            c["provider"].as_str().unwrap_or(""),
            c["driver"].as_str().unwrap_or("")
        );
        let kind = kind_from(&raw);
        let name = c["name"].as_str().unwrap_or("DBeaver connection");
        let (mut p, skipped) = candidate(name, kind, c["provider"].as_str().unwrap_or("?"));
        p.host = opt(cfg["host"].as_str());
        p.port = port(cfg.get("port"));
        p.database = opt(cfg["database"].as_str());
        p.user = opt(cfg["user"].as_str()).or_else(|| opt(c["user"].as_str()));
        if p.kind.is_file() {
            p.file = opt(cfg["database"].as_str())
                .or_else(|| cfg["url"].as_str().and_then(path_from_jdbc));
            p.database = None;
        }
        p.env = env_from(cfg["type"].as_str().unwrap_or(""));
        p.read_only = c["read-only"].as_bool().unwrap_or(false);
        if let Some(ssh) = cfg["handlers"]["ssh_tunnel"].as_object() {
            if ssh.get("enabled").and_then(Value::as_bool).unwrap_or(false) {
                let props = &ssh["properties"];
                let auth = props["authType"].as_str().unwrap_or("PASSWORD");
                p.ssh = Some(SshConfig {
                    host: props["host"].as_str().unwrap_or_default().to_string(),
                    port: port(props.get("port")),
                    user: props["user"]
                        .as_str()
                        .or(ssh.get("user").and_then(Value::as_str))
                        .unwrap_or_default()
                        .to_string(),
                    auth: match auth {
                        "PUBLIC_KEY" => SshAuth::Key,
                        "AGENT" => SshAuth::Agent,
                        _ => SshAuth::Password,
                    },
                    key_path: opt(props["keyPath"].as_str()),
                    agent_socket: None,
                    host_key: None,
                });
            }
        }
        out.push(Candidate {
            config: finish(p),
            skipped_reason: skipped,
        });
    }
    Ok(out)
}

// --------------------------------------------------------------- TablePlus ---

fn first_str<'a>(m: &'a serde_json::Map<String, Value>, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|k| m.get(*k).and_then(Value::as_str).filter(|s| !s.is_empty()))
}

fn first_bool(m: &serde_json::Map<String, Value>, keys: &[&str]) -> bool {
    keys.iter().any(|k| match m.get(*k) {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_i64() == Some(1),
        Some(Value::String(s)) => s == "1" || s.eq_ignore_ascii_case("true"),
        _ => false,
    })
}

fn parse_tableplus_value(v: Value) -> Result<Vec<Candidate>> {
    let list = match v {
        Value::Array(a) => a,
        Value::Object(o) if o.contains_key("Driver") || o.contains_key("ConnectionName") => {
            vec![Value::Object(o)]
        }
        _ => return Err(Error::Other("not a TablePlus connections file".into())),
    };
    let mut out = Vec::new();
    for item in list {
        let Value::Object(m) = item else { continue };
        let driver = first_str(&m, &["Driver", "DriverName", "driver"]).unwrap_or("?");
        let name =
            first_str(&m, &["ConnectionName", "Name", "name"]).unwrap_or("TablePlus connection");
        let (mut p, skipped) = candidate(name, kind_from(driver), driver);
        p.host = opt(first_str(&m, &["DatabaseHost", "Host"]));
        p.port = port(m.get("DatabasePort").or(m.get("Port")));
        p.user = opt(first_str(&m, &["DatabaseUser", "User"]));
        p.database = opt(first_str(&m, &["DatabaseName", "Database"]));
        if p.kind.is_file() {
            p.file = opt(first_str(
                &m,
                &["DatabasePath", "DatabaseFile", "DatabaseHost"],
            ));
            p.database = None;
        }
        // TablePlus spells it "Enviroment"
        p.env = env_from(first_str(&m, &["Enviroment", "Environment"]).unwrap_or(""));
        if first_bool(&m, &["isOverSSH", "OverSSH", "UseSSH"]) {
            let key = opt(first_str(
                &m,
                &[
                    "ServerPrivateKeyPath",
                    "PrivateKeyPath",
                    "SSHPrivateKeyPath",
                ],
            ));
            p.ssh = Some(SshConfig {
                host: first_str(&m, &["ServerAddress", "SSHHost"])
                    .unwrap_or_default()
                    .to_string(),
                port: port(m.get("ServerPort").or(m.get("SSHPort"))),
                user: first_str(&m, &["ServerUser", "SSHUser"])
                    .unwrap_or_default()
                    .to_string(),
                auth: if first_bool(&m, &["isUsePrivateKey", "UsePrivateKey"]) || key.is_some() {
                    SshAuth::Key
                } else {
                    SshAuth::Password
                },
                key_path: key,
                agent_socket: None,
                host_key: None,
            });
        }
        out.push(Candidate {
            config: finish(p),
            skipped_reason: skipped,
        });
    }
    Ok(out)
}

pub fn parse_tableplus(bytes: &[u8]) -> Result<Vec<Candidate>> {
    // JSON export, else XML/binary plist
    if let Ok(v) = serde_json::from_slice::<Value>(bytes) {
        return parse_tableplus_value(v);
    }
    let v: Value =
        plist::from_bytes(bytes).map_err(|e| Error::Other(format!("TablePlus plist: {e}")))?;
    parse_tableplus_value(v)
}

// ----------------------------------------------------------------- Navicat ---

pub fn parse_navicat(xml: &str) -> Result<Vec<Candidate>> {
    let doc =
        roxmltree::Document::parse(xml).map_err(|e| Error::Other(format!("Navicat .ncx: {e}")))?;
    let mut out = Vec::new();
    for n in doc.descendants().filter(|n| n.has_tag_name("Connection")) {
        let a = |k: &str| n.attribute(k).map(str::trim).filter(|s| !s.is_empty());
        let conn_type = a("ConnType").unwrap_or("?");
        let (mut p, skipped) = candidate(
            a("ConnectionName").unwrap_or("Navicat connection"),
            kind_from(conn_type),
            conn_type,
        );
        p.host = a("Host").map(str::to_string);
        p.port = a("Port").and_then(|s| s.parse().ok());
        p.user = a("UserName").map(str::to_string);
        p.database = a("Database").or(a("InitialDatabase")).map(str::to_string);
        if p.kind.is_file() {
            p.file = a("DatabaseFileName")
                .or(a("DatabaseFile"))
                .map(str::to_string);
        }
        if a("SSH")
            .map(|s| s.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
        {
            let auth = a("SSH_AuthenMethod").unwrap_or("PASSWORD");
            p.ssh = Some(SshConfig {
                host: a("SSH_Host").unwrap_or_default().to_string(),
                port: a("SSH_Port").and_then(|s| s.parse().ok()),
                user: a("SSH_UserName").unwrap_or_default().to_string(),
                auth: if auth.to_uppercase().contains("PUBLIC") {
                    SshAuth::Key
                } else {
                    SshAuth::Password
                },
                key_path: a("SSH_PrivateKey").map(str::to_string),
                agent_socket: None,
                host_key: None,
            });
        }
        out.push(Candidate {
            config: finish(p),
            skipped_reason: skipped,
        });
    }
    if out.is_empty() {
        return Err(Error::Other(
            "no <Connection> entries found in the .ncx file".into(),
        ));
    }
    Ok(out)
}

// ------------------------------------------------------------------ public ---

/// Parse a file, guessing the format from its name / content.
pub fn parse_file(path: &Path) -> Result<(Source, Vec<Candidate>)> {
    let bytes = std::fs::read(path)?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let text = String::from_utf8_lossy(&bytes);
    if name.ends_with(".ncx")
        || text.trim_start().starts_with("<?xml") && text.contains("<Connections")
    {
        return Ok((Source::Navicat, parse_navicat(&text)?));
    }
    if name == "data-sources.json"
        || text.contains("\"connections\"") && text.contains("\"provider\"")
    {
        return Ok((Source::Dbeaver, parse_dbeaver(&text)?));
    }
    Ok((Source::Tableplus, parse_tableplus(&bytes)?))
}

/// Default locations of other clients' connection files on this machine.
pub fn detect() -> Vec<DetectedFile> {
    let Some(base) = directories::BaseDirs::new() else {
        return vec![];
    };
    let home = base.home_dir();
    let mut c: Vec<(Source, PathBuf)> = vec![
        // DBeaver (macOS, Windows, Linux)
        (
            Source::Dbeaver,
            home.join("Library/DBeaverData/workspace6/General/.dbeaver/data-sources.json"),
        ),
        (
            Source::Dbeaver,
            base.data_dir()
                .join("DBeaverData/workspace6/General/.dbeaver/data-sources.json"),
        ),
        (
            Source::Dbeaver,
            home.join(".local/share/DBeaverData/workspace6/General/.dbeaver/data-sources.json"),
        ),
        // TablePlus (macOS app + Setapp)
        (
            Source::Tableplus,
            home.join("Library/Application Support/com.tinyapp.TablePlus/Data/Connections.plist"),
        ),
        (
            Source::Tableplus,
            home.join(
                "Library/Application Support/com.tinyapp.TablePlus-setapp/Data/Connections.plist",
            ),
        ),
    ];
    // TablePlus for Windows
    c.push((
        Source::Tableplus,
        base.data_dir()
            .join("com.tinyapp.TablePlus/Data/Connections.json"),
    ));
    let mut seen = std::collections::HashSet::new();
    c.into_iter()
        .filter(|(_, p)| p.is_file() && seen.insert(p.clone()))
        .map(|(source, p)| DetectedFile {
            source,
            path: p.to_string_lossy().into_owned(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dbeaver() {
        let json = r#"{"folders":{},"connections":{
          "postgres-jdbc-1":{"provider":"postgresql","driver":"postgres-jdbc","name":"Prod PG","read-only":false,
            "configuration":{"host":"db.prod","port":"5432","database":"app","user":"app","type":"prod",
              "handlers":{"ssh_tunnel":{"type":"TUNNEL","enabled":true,"properties":{"host":"bastion","port":"2222","user":"ubuntu","authType":"PUBLIC_KEY","keyPath":"~/.ssh/id_ed25519"}}}}},
          "sqlite-1":{"provider":"generic","driver":"sqlite_jdbc","name":"Local file","configuration":{"url":"jdbc:sqlite:/data/app.db","type":"dev"}},
          "oracle-1":{"provider":"oracle","driver":"oracle_thin","name":"Ora","configuration":{"host":"o"}},
          "ch-1":{"provider":"clickhouse","driver":"com_clickhouse","name":"CH","configuration":{"host":"ch","port":8123}}
        }}"#;
        let c = parse_dbeaver(json).unwrap();
        let pg = c.iter().find(|x| x.config.name == "Prod PG").unwrap();
        assert_eq!(pg.config.kind, DbKind::Postgres);
        assert_eq!(
            (
                pg.config.host.as_deref(),
                pg.config.port,
                pg.config.user.as_deref()
            ),
            (Some("db.prod"), Some(5432), Some("app"))
        );
        assert_eq!(pg.config.env, Environment::Production);
        assert!(pg.config.read_only, "production imports start read-only");
        let ssh = pg.config.ssh.as_ref().unwrap();
        assert_eq!(
            (ssh.host.as_str(), ssh.port, ssh.auth),
            ("bastion", Some(2222), SshAuth::Key)
        );
        let lite = c.iter().find(|x| x.config.name == "Local file").unwrap();
        assert_eq!(
            (lite.config.kind, lite.config.file.as_deref()),
            (DbKind::Sqlite, Some("/data/app.db"))
        );
        assert!(c
            .iter()
            .find(|x| x.config.name == "Ora")
            .unwrap()
            .skipped_reason
            .is_some());
        assert_eq!(
            c.iter()
                .find(|x| x.config.name == "CH")
                .unwrap()
                .config
                .kind,
            DbKind::Clickhouse
        );
    }

    #[test]
    fn tableplus_plist_and_json() {
        let plist = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><array>
 <dict><key>ConnectionName</key><string>Staging MySQL</string><key>Driver</key><string>MySQL</string>
   <key>DatabaseHost</key><string>10.0.0.5</string><key>DatabasePort</key><string>3307</string>
   <key>DatabaseUser</key><string>root</string><key>DatabaseName</key><string>shop</string>
   <key>Enviroment</key><string>staging</string><key>isOverSSH</key><true/>
   <key>ServerAddress</key><string>jump</string><key>ServerPort</key><string>22</string><key>ServerUser</key><string>me</string></dict>
 <dict><key>ConnectionName</key><string>Notes</string><key>Driver</key><string>SQLite</string>
   <key>DatabasePath</key><string>/Users/me/notes.db</string></dict>
</array></plist>"#;
        let c = parse_tableplus(plist.as_bytes()).unwrap();
        assert_eq!(c.len(), 2);
        let my = &c[0].config;
        assert_eq!(
            (my.kind, my.port, my.database.as_deref(), my.env),
            (
                DbKind::Mysql,
                Some(3307),
                Some("shop"),
                Environment::Staging
            )
        );
        assert_eq!(my.ssh.as_ref().unwrap().user, "me");
        assert_eq!(c[1].config.file.as_deref(), Some("/Users/me/notes.db"));

        let json = r#"[{"ConnectionName":"Redshift DW","Driver":"Redshift","DatabaseHost":"rs.aws","DatabasePort":5439}]"#;
        let c = parse_tableplus(json.as_bytes()).unwrap();
        assert_eq!(
            (c[0].config.kind, c[0].config.port),
            (DbKind::Redshift, Some(5439))
        );
    }

    #[test]
    fn navicat() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<Connections Ver="1.5">
  <Connection ConnectionName="Main PG" ConnType="POSTGRESQL" Host="pg.local" Port="5433" UserName="postgres" Database="main"
     SSH="true" SSH_Host="bastion" SSH_Port="22" SSH_UserName="ops" SSH_AuthenMethod="PUBLICKEY" SSH_PrivateKey="/k/id_rsa"/>
  <Connection ConnectionName="Maria" ConnType="MARIADB" Host="m" Port="3306" UserName="root" SSH="false"/>
  <Connection ConnectionName="Mongo" ConnType="MONGODB" Host="mg" Port="27017"/>
</Connections>"#;
        let c = parse_navicat(xml).unwrap();
        assert_eq!(c[0].config.kind, DbKind::Postgres);
        assert_eq!(c[0].config.port, Some(5433));
        assert_eq!(
            c[0].config.ssh.as_ref().unwrap().key_path.as_deref(),
            Some("/k/id_rsa")
        );
        assert_eq!(c[1].config.kind, DbKind::Mariadb);
        assert!(c[1].config.ssh.is_none());
        assert!(c[2].skipped_reason.as_deref().unwrap().contains("MONGODB"));
    }
}
