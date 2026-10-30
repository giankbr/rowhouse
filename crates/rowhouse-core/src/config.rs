//! Connection profiles.
//!
//! Profiles (everything except the password) live in `connections.json` inside the
//! platform config dir, so the desktop app and `rowhouse-mcp` share the same list:
//!
//! * macOS:   ~/Library/Application Support/id.novagen.rowhouse/connections.json
//! * Windows: %APPDATA%\novagen\rowhouse\config\connections.json
//! * Linux:   ~/.config/rowhouse/connections.json
//!
//! Override with the `ROWHOUSE_CONFIG_DIR` env var. Passwords go to the OS keychain
//! (service `rowhouse`, account = profile id).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

const KEYRING_SERVICE: &str = "rowhouse";

/// What the user picks in the UI. Several kinds share a wire protocol / driver.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DbKind {
    Postgres,
    Neon,
    Supabase,
    Cockroach,
    Redshift,
    Greenplum,
    Mysql,
    Mariadb,
    Tidb,
    Singlestore,
    Sqlite,
    Turso,
    D1,
    Duckdb,
    Mssql,
    Clickhouse,
    Redis,
    Upstash,
    Mongodb,
    Cassandra,
    Scylla,
}

/// The engine actually used to talk to a [`DbKind`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Driver {
    Postgres,
    Mysql,
    Sqlite,
    Duckdb,
    Mssql,
    Clickhouse,
    Redis,
    Mongo,
    Cassandra,
}

impl DbKind {
    pub const ALL: [DbKind; 21] = [
        DbKind::Postgres,
        DbKind::Neon,
        DbKind::Supabase,
        DbKind::Cockroach,
        DbKind::Redshift,
        DbKind::Greenplum,
        DbKind::Mysql,
        DbKind::Mariadb,
        DbKind::Tidb,
        DbKind::Singlestore,
        DbKind::Sqlite,
        DbKind::Turso,
        DbKind::D1,
        DbKind::Duckdb,
        DbKind::Mssql,
        DbKind::Clickhouse,
        DbKind::Redis,
        DbKind::Upstash,
        DbKind::Mongodb,
        DbKind::Cassandra,
        DbKind::Scylla,
    ];

    pub fn driver(self) -> Driver {
        use DbKind::*;
        match self {
            Postgres | Neon | Supabase | Cockroach | Redshift | Greenplum => Driver::Postgres,
            Mysql | Mariadb | Tidb | Singlestore => Driver::Mysql,
            // remote SQLite: same dialect, different transport (see Connection::connect)
            Sqlite | Turso | D1 => Driver::Sqlite,
            Duckdb => Driver::Duckdb,
            Mssql => Driver::Mssql,
            Clickhouse => Driver::Clickhouse,
            Redis | Upstash => Driver::Redis,
            Mongodb => Driver::Mongo,
            Cassandra | Scylla => Driver::Cassandra,
        }
    }

    /// Embedded databases open a local file instead of a host/port.
    pub fn is_file(self) -> bool {
        matches!(self, DbKind::Sqlite | DbKind::Duckdb)
    }

    /// Talks HTTP to a hosted API (no host/port, no SSH tunnel).
    pub fn is_http_api(self) -> bool {
        matches!(self, DbKind::Turso | DbKind::D1)
    }

    pub fn default_port(self) -> Option<u16> {
        use DbKind::*;
        match self {
            Postgres | Greenplum | Neon | Supabase => Some(5432),
            Cockroach => Some(26257),
            Redshift => Some(5439),
            Mysql | Mariadb | Singlestore => Some(3306),
            Tidb => Some(4000),
            Mssql => Some(1433),
            Clickhouse => Some(8123),
            Redis | Upstash => Some(6379),
            Mongodb => Some(27017),
            Cassandra | Scylla => Some(9042),
            Sqlite | Duckdb | Turso | D1 => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionConfig {
    pub id: String,
    pub name: String,
    pub kind: DbKind,
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub database: Option<String>,
    /// File path for embedded databases (SQLite, DuckDB)
    #[serde(default)]
    pub file: Option<String>,
    /// Block every non-read statement for this profile (UI + MCP).
    #[serde(default)]
    pub read_only: bool,
    /// Allow the MCP server to see / use this profile at all.
    #[serde(default = "yes")]
    pub mcp_enabled: bool,
    /// Let MCP clients request writes; each one still needs approval in the app.
    #[serde(default)]
    pub mcp_writes: bool,
    /// Tag color shown in the sidebar (e.g. "red" for production).
    #[serde(default)]
    pub color: Option<String>,
    /// TLS settings (Postgres family, MySQL family, SQL Server, ClickHouse).
    #[serde(default)]
    pub tls: Option<TlsConfig>,
    /// Environment label; `production` turns on the safety rails in the UI.
    #[serde(default)]
    pub env: Environment,
    /// Optional SSH tunnel (bastion / jump host).
    #[serde(default)]
    pub ssh: Option<SshConfig>,
}

/// Same names as libpq's `sslmode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TlsMode {
    Disable,
    #[default]
    Prefer,
    Require,
    /// verify the server certificate against the CA
    VerifyCa,
    /// … and that the host name matches
    VerifyFull,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TlsConfig {
    #[serde(default)]
    pub mode: TlsMode,
    /// PEM file with the CA certificate(s)
    #[serde(default)]
    pub ca_cert: Option<String>,
    /// PEM client certificate (mutual TLS)
    #[serde(default)]
    pub client_cert: Option<String>,
    /// PEM private key for the client certificate
    #[serde(default)]
    pub client_key: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Environment {
    #[default]
    None,
    Local,
    Development,
    Staging,
    Production,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SshAuth {
    #[default]
    Key,
    Password,
    /// ssh-agent / 1Password / Pageant / Windows OpenSSH agent
    Agent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshConfig {
    pub host: String,
    #[serde(default)]
    pub port: Option<u16>,
    pub user: String,
    #[serde(default)]
    pub auth: SshAuth,
    /// Private key path (for `auth = key`). `~` is expanded.
    #[serde(default)]
    pub key_path: Option<String>,
    /// Custom agent socket (e.g. 1Password's `agent.sock`); defaults to `$SSH_AUTH_SOCK`
    /// on macOS/Linux and the OpenSSH agent pipe (then Pageant) on Windows.
    #[serde(default)]
    pub agent_socket: Option<String>,
    /// Pinned server key fingerprint (`SHA256:...`). Filled on first connect
    /// when the host is not in ~/.ssh/known_hosts (trust on first use).
    #[serde(default)]
    pub host_key: Option<String>,
}

fn yes() -> bool {
    true
}

impl ConnectionConfig {
    pub fn new(name: impl Into<String>, kind: DbKind) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.into(),
            kind,
            host: None,
            port: kind.default_port(),
            user: None,
            database: None,
            file: None,
            read_only: false,
            mcp_enabled: true,
            mcp_writes: false,
            color: None,
            env: Environment::None,
            tls: match kind {
                DbKind::Neon | DbKind::Supabase => Some(TlsConfig {
                    mode: TlsMode::Require,
                    ..Default::default()
                }),
                _ => None,
            },
            ssh: None,
        }
    }

    pub fn password(&self) -> Result<Option<String>> {
        if self.kind.is_file() {
            return Ok(None);
        }
        let entry = keyring::Entry::new(KEYRING_SERVICE, &self.id)?;
        match entry.get_password() {
            Ok(p) => Ok(Some(p)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// SSH password, or the private key's passphrase.
    pub fn ssh_secret(&self) -> Result<Option<String>> {
        if self.ssh.is_none() {
            return Ok(None);
        }
        let entry = keyring::Entry::new(KEYRING_SERVICE, &format!("{}:ssh", self.id))?;
        match entry.get_password() {
            Ok(p) => Ok(Some(p)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn set_ssh_secret(&self, secret: Option<&str>) -> Result<()> {
        Self::store_secret(&format!("{}:ssh", self.id), secret)
    }

    pub fn set_password(&self, password: Option<&str>) -> Result<()> {
        Self::store_secret(&self.id, password)
    }

    fn store_secret(account: &str, secret: Option<&str>) -> Result<()> {
        let entry = keyring::Entry::new(KEYRING_SERVICE, account)?;
        let password = secret;
        match password {
            Some(p) if !p.is_empty() => entry.set_password(p)?,
            _ => match entry.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => {}
                Err(e) => return Err(e.into()),
            },
        }
        Ok(())
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct StoreFile {
    #[serde(default)]
    connections: Vec<ConnectionConfig>,
}

/// JSON file of connection profiles.
pub struct ConnectionStore {
    path: PathBuf,
}

impl ConnectionStore {
    pub fn default_dir() -> PathBuf {
        if let Ok(dir) = std::env::var("ROWHOUSE_CONFIG_DIR") {
            return PathBuf::from(dir);
        }
        directories::ProjectDirs::from("id", "novagen", "rowhouse")
            .map(|d| d.config_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from(".rowhouse"))
    }

    pub fn open_default() -> Self {
        Self::at(Self::default_dir().join("connections.json"))
    }

    pub fn at(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    pub fn list(&self) -> Result<Vec<ConnectionConfig>> {
        match std::fs::read_to_string(&self.path) {
            Ok(s) if s.trim().is_empty() => Ok(vec![]),
            Ok(s) => Ok(serde_json::from_str::<StoreFile>(&s)?.connections),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
            Err(e) => Err(e.into()),
        }
    }

    pub fn get(&self, id_or_name: &str) -> Result<ConnectionConfig> {
        self.list()?
            .into_iter()
            .find(|c| c.id == id_or_name || c.name.eq_ignore_ascii_case(id_or_name))
            .ok_or_else(|| Error::NotFound(id_or_name.to_string()))
    }

    /// Insert or replace (matched by id).
    pub fn save(&self, cfg: ConnectionConfig) -> Result<ConnectionConfig> {
        let mut all = self.list()?;
        match all.iter_mut().find(|c| c.id == cfg.id) {
            Some(slot) => *slot = cfg.clone(),
            None => all.push(cfg.clone()),
        }
        self.write(all)?;
        Ok(cfg)
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        let all: Vec<_> = self.list()?.into_iter().filter(|c| c.id != id).collect();
        self.write(all)
    }

    fn write(&self, connections: Vec<ConnectionConfig>) -> Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(&StoreFile { connections })?)?;
        std::fs::rename(tmp, &self.path)?;
        Ok(())
    }
}
