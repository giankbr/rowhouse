//! Connection profiles.
//!
//! Profiles (everything except the password) live in `connections.json` inside the
//! platform config dir, so the desktop app and `rowhouse-mcp` share the same list:
//!
//! * macOS:   ~/Library/Application Support/id.morfotech.rowhouse/connections.json
//! * Windows: %APPDATA%\morfotech\rowhouse\config\connections.json
//! * Linux:   ~/.config/rowhouse/connections.json
//!
//! Override with the `ROWHOUSE_CONFIG_DIR` env var. Passwords go to the OS keychain
//! (service `rowhouse`, account = profile id).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

const KEYRING_SERVICE: &str = "rowhouse";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DbKind {
    Postgres,
    Mysql,
    Sqlite,
}

impl DbKind {
    pub fn default_port(self) -> Option<u16> {
        match self {
            DbKind::Postgres => Some(5432),
            DbKind::Mysql => Some(3306),
            DbKind::Sqlite => None,
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
    /// SQLite file path
    #[serde(default)]
    pub file: Option<String>,
    /// Block every non-read statement for this profile (UI + MCP).
    #[serde(default)]
    pub read_only: bool,
    /// Allow the MCP server to see / use this profile at all.
    #[serde(default = "yes")]
    pub mcp_enabled: bool,
    /// Tag color shown in the sidebar (e.g. "red" for production).
    #[serde(default)]
    pub color: Option<String>,
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
            color: None,
        }
    }

    pub fn password(&self) -> Result<Option<String>> {
        if self.kind == DbKind::Sqlite {
            return Ok(None);
        }
        let entry = keyring::Entry::new(KEYRING_SERVICE, &self.id)?;
        match entry.get_password() {
            Ok(p) => Ok(Some(p)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn set_password(&self, password: Option<&str>) -> Result<()> {
        let entry = keyring::Entry::new(KEYRING_SERVICE, &self.id)?;
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
        directories::ProjectDirs::from("id", "morfotech", "rowhouse")
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
