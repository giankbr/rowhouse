//! Folder sync (Pro): connections and saved queries travel between machines
//! through a folder the user already syncs (iCloud Drive, Dropbox, OneDrive,
//! Syncthing, a network share…). There is no Rowhouse server.
//!
//! Everything goes into one file, `rowhouse.rhsync`, encrypted with a
//! passphrase: PBKDF2-HMAC-SHA256 (600k rounds) → AES-256-GCM. The folder
//! provider only ever sees ciphertext.
//!
//! Merging is per item with a three-way check against what this device saw
//! at the last sync: an item changed only on the other side is taken, an item
//! changed here is kept (and pushed). Deleting is local only, for now.

use std::{
    collections::HashMap,
    num::NonZeroU32,
    path::{Path, PathBuf},
    sync::Mutex,
};

use ring::{
    aead, pbkdf2,
    rand::{SecureRandom, SystemRandom},
};
use serde::{Deserialize, Serialize};

use crate::{
    config::{ConnectionConfig, ConnectionStore},
    history::{History, Snippet},
    Error, Result,
};

pub const FILE_NAME: &str = "rowhouse.rhsync";
const ROUNDS: u32 = 600_000;

// ---------------------------------------------------------------------------
// envelope
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    pub format: String,
    pub v: u32,
    pub kdf: String,
    pub rounds: u32,
    pub salt: String,
    pub nonce: String,
    pub data: String,
}

pub fn base64_encode(b: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(b.len().div_ceil(3) * 4);
    for c in b.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if c.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}
fn b64d(s: &str) -> Result<Vec<u8>> {
    crate::backend::libsql::base64_decode(s)
        .ok_or_else(|| Error::Other("sync file is damaged".into()))
}

/// Derived keys are cached per (passphrase, salt): PBKDF2 is deliberately slow.
static KEYS: Mutex<Vec<(String, Vec<u8>, [u8; 32])>> = Mutex::new(Vec::new());

fn derive(pass: &str, salt: &[u8], rounds: u32) -> [u8; 32] {
    if let Some(k) = KEYS
        .lock()
        .unwrap()
        .iter()
        .find(|(p, s, _)| p == pass && s == salt)
    {
        return k.2;
    }
    let mut key = [0u8; 32];
    pbkdf2::derive(
        pbkdf2::PBKDF2_HMAC_SHA256,
        NonZeroU32::new(rounds.max(1)).unwrap(),
        salt,
        pass.as_bytes(),
        &mut key,
    );
    let mut cache = KEYS.lock().unwrap();
    cache.retain(|(p, _, _)| p == pass);
    cache.push((pass.to_string(), salt.to_vec(), key));
    key
}

/// Encrypt `plain`; reuses `salt` (from the existing file) when given so the key cache hits.
pub fn seal(pass: &str, plain: &[u8], salt: Option<Vec<u8>>) -> Result<Envelope> {
    if pass.chars().count() < 8 {
        return Err(Error::Other(
            "use a passphrase of at least 8 characters".into(),
        ));
    }
    let rng = SystemRandom::new();
    let salt = match salt {
        Some(s) => s,
        None => {
            let mut s = vec![0u8; 16];
            rng.fill(&mut s)
                .map_err(|_| Error::Other("no randomness available".into()))?;
            s
        }
    };
    let mut nonce = [0u8; 12];
    rng.fill(&mut nonce)
        .map_err(|_| Error::Other("no randomness available".into()))?;
    let key = aead::LessSafeKey::new(
        aead::UnboundKey::new(&aead::AES_256_GCM, &derive(pass, &salt, ROUNDS))
            .expect("32-byte key"),
    );
    let mut buf = plain.to_vec();
    key.seal_in_place_append_tag(
        aead::Nonce::assume_unique_for_key(nonce),
        aead::Aad::from(b"rowhouse-sync-v1"),
        &mut buf,
    )
    .map_err(|_| Error::Other("encryption failed".into()))?;
    Ok(Envelope {
        format: "rowhouse-sync".into(),
        v: 1,
        kdf: "pbkdf2-sha256".into(),
        rounds: ROUNDS,
        salt: base64_encode(&salt),
        nonce: base64_encode(&nonce),
        data: base64_encode(&buf),
    })
}

pub fn open(pass: &str, env: &Envelope) -> Result<Vec<u8>> {
    if env.format != "rowhouse-sync" || env.v != 1 {
        return Err(Error::Other(
            "not a Rowhouse sync file (or from a newer version)".into(),
        ));
    }
    let salt = b64d(&env.salt)?;
    let nonce: [u8; 12] = b64d(&env.nonce)?
        .try_into()
        .map_err(|_| Error::Other("sync file is damaged".into()))?;
    let key = aead::LessSafeKey::new(
        aead::UnboundKey::new(&aead::AES_256_GCM, &derive(pass, &salt, env.rounds))
            .expect("32-byte key"),
    );
    let mut buf = b64d(&env.data)?;
    let plain = key
        .open_in_place(
            aead::Nonce::assume_unique_for_key(nonce),
            aead::Aad::from(b"rowhouse-sync-v1"),
            &mut buf,
        )
        .map_err(|_| Error::Other("wrong passphrase (or the sync file was changed)".into()))?;
    Ok(plain.to_vec())
}

// ---------------------------------------------------------------------------
// payload
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Payload {
    pub device: String,
    pub at: i64,
    pub connections: Vec<SyncedConnection>,
    pub snippets: Vec<Snippet>,
    #[serde(default)]
    pub dashboards: Vec<crate::dashboards::Dashboard>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncedConnection {
    pub config: ConnectionConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_secret: Option<String>,
}

/// What this device saw at its last sync (for three-way merges).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncState {
    #[serde(default)]
    pub folder: Option<String>,
    #[serde(default)]
    pub include_secrets: bool,
    #[serde(default)]
    pub last_sync: Option<i64>,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub remote_device: Option<String>,
    #[serde(default)]
    pub remote_at: Option<i64>,
    /// connection id → fingerprint at last sync
    #[serde(default)]
    pub conns: HashMap<String, String>,
    /// snippet key → fingerprint at last sync
    #[serde(default)]
    pub snippets: HashMap<String, String>,
    /// dashboard id → fingerprint at last sync
    #[serde(default)]
    pub dashboards: HashMap<String, String>,
}

impl SyncState {
    pub fn path() -> PathBuf {
        ConnectionStore::default_dir().join("sync.json")
    }
    pub fn load_from(p: &Path) -> SyncState {
        std::fs::read_to_string(p)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }
    pub fn save_to(&self, p: &Path) -> Result<()> {
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::write(p, serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncReport {
    pub pulled_connections: usize,
    pub pulled_snippets: usize,
    pub pulled_dashboards: usize,
    pub pushed: bool,
    pub remote_device: Option<String>,
}

fn fp<T: Serialize>(v: &T) -> String {
    let s = serde_json::to_vec(v).unwrap_or_default();
    let d = ring::digest::digest(&ring::digest::SHA256, &s);
    base64_encode(&d.as_ref()[..12])
}
fn snippet_key(s: &Snippet) -> String {
    format!(
        "{}\u{1f}{}",
        s.connection_id.as_deref().unwrap_or(""),
        s.name
    )
}
fn dash_fp(d: &crate::dashboards::Dashboard) -> String {
    fp(&(&d.name, &d.connection_id, &d.tiles, d.refresh_secs))
}
fn snippet_fp(s: &Snippet) -> String {
    fp(&(&s.name, &s.sql, &s.connection_id))
}
/// Settings only — secrets ride along but don't decide who wins.
fn conn_fp(c: &SyncedConnection) -> String {
    fp(&c.config)
}

pub fn device_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            std::fs::read_to_string("/etc/hostname")
                .ok()
                .map(|s| s.trim().to_string())
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "another device".into())
}

fn now() -> i64 {
    crate::schedules::now()
}

// ---------------------------------------------------------------------------
// the sync itself
// ---------------------------------------------------------------------------

const KEYCHAIN_ACCOUNT: &str = "sync-passphrase";

/// The passphrase lives in the OS keychain, next to the connection passwords.
pub fn saved_passphrase() -> Option<String> {
    keyring::Entry::new("rowhouse", KEYCHAIN_ACCOUNT)
        .ok()?
        .get_password()
        .ok()
}
pub fn save_passphrase(p: Option<&str>) -> Result<()> {
    let e = keyring::Entry::new("rowhouse", KEYCHAIN_ACCOUNT)?;
    match p {
        Some(p) => e.set_password(p)?,
        None => match e.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {}
            Err(err) => return Err(err.into()),
        },
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    pub state: SyncState,
    pub has_passphrase: bool,
    pub file_exists: bool,
}

pub fn status() -> SyncStatus {
    let state = SyncState::load_from(&SyncState::path());
    SyncStatus {
        file_exists: state
            .folder
            .as_ref()
            .is_some_and(|f| Path::new(f).join(FILE_NAME).exists()),
        has_passphrase: saved_passphrase().is_some(),
        state,
    }
}

/// Record a failed background sync so the UI can show it.
pub fn record_error(msg: &str) {
    let p = SyncState::path();
    let mut st = SyncState::load_from(&p);
    st.last_error = Some(msg.to_string());
    let _ = st.save_to(&p);
}

/// Stop syncing on this device (the file in the folder stays).
pub fn disable() -> Result<()> {
    save_passphrase(None)?;
    let p = SyncState::path();
    let mut st = SyncState::load_from(&p);
    st.folder = None;
    st.last_error = None;
    st.conns.clear();
    st.snippets.clear();
    st.dashboards.clear();
    st.save_to(&p)
}

pub struct Syncer<'a> {
    pub store: &'a ConnectionStore,
    pub history: &'a History,
    pub state_path: PathBuf,
    /// read / write passwords in the OS keychain (off in tests)
    pub keychain: bool,
    /// dashboards file to sync (None: leave the file's dashboards untouched)
    pub dashboards: Option<PathBuf>,
}

impl Syncer<'_> {
    fn local_connections(&self, secrets: bool) -> Result<Vec<SyncedConnection>> {
        Ok(self
            .store
            .list()?
            .into_iter()
            .map(|config| {
                let (password, ssh_secret) = if secrets && self.keychain {
                    (
                        config.password().ok().flatten(),
                        config.ssh_secret().ok().flatten(),
                    )
                } else {
                    (None, None)
                };
                SyncedConnection {
                    config,
                    password,
                    ssh_secret,
                }
            })
            .collect())
    }

    fn read_remote(&self, file: &Path, pass: &str) -> Result<Option<(Payload, Vec<u8>)>> {
        let text = match std::fs::read_to_string(file) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let env: Envelope =
            serde_json::from_str(&text).map_err(|_| Error::Other("sync file is damaged".into()))?;
        let plain = open(pass, &env)?;
        let payload: Payload = serde_json::from_slice(&plain)?;
        Ok(Some((payload, b64d(&env.salt)?)))
    }

    /// Check the passphrase against an existing sync file (or accept it for a new one).
    pub fn check(&self, folder: &Path, pass: &str) -> Result<Option<Payload>> {
        if !folder.is_dir() {
            return Err(Error::Other(format!(
                "folder not found: {}",
                folder.display()
            )));
        }
        if pass.chars().count() < 8 {
            return Err(Error::Other(
                "use a passphrase of at least 8 characters".into(),
            ));
        }
        Ok(self
            .read_remote(&folder.join(FILE_NAME), pass)?
            .map(|p| p.0))
    }

    /// Pull what changed elsewhere, then push this device's view.
    pub async fn sync(
        &self,
        folder: &Path,
        pass: &str,
        include_secrets: bool,
    ) -> Result<SyncReport> {
        let mut state = SyncState::load_from(&self.state_path);
        let file = folder.join(FILE_NAME);
        if !folder.is_dir() {
            return Err(Error::Other(format!(
                "sync folder not found: {}",
                folder.display()
            )));
        }
        let remote = self.read_remote(&file, pass)?;
        let mut report = SyncReport::default();

        // ---- connections
        let mut local: Vec<SyncedConnection> = self.local_connections(include_secrets)?;
        let salt = remote.as_ref().map(|r| r.1.clone());
        if let Some((payload, _)) = &remote {
            report.remote_device = Some(payload.device.clone());
            state.remote_device = Some(payload.device.clone());
            state.remote_at = Some(payload.at);
            for rc in &payload.connections {
                let rfp = conn_fp(rc);
                let base = state.conns.get(&rc.config.id).cloned();
                let li = local.iter().position(|l| l.config.id == rc.config.id);
                let take = match li {
                    None => base.is_none(), // new elsewhere (if we had it before, it was deleted here)
                    Some(i) => {
                        let lfp = conn_fp(&local[i]);
                        lfp != rfp && base.as_deref() == Some(lfp.as_str())
                    }
                };
                if take {
                    self.store.save(rc.config.clone())?;
                    if self.keychain {
                        if let Some(p) = &rc.password {
                            let _ = rc.config.set_password(Some(p));
                        }
                        if let Some(p) = &rc.ssh_secret {
                            let _ = rc.config.set_ssh_secret(Some(p));
                        }
                    }
                    match li {
                        Some(i) => local[i] = rc.clone(),
                        None => local.push(rc.clone()),
                    }
                    report.pulled_connections += 1;
                }
            }
        }

        // ---- snippets
        let mut snippets = self.history.list_snippets(None, None).await?;
        if let Some((payload, _)) = &remote {
            for rs in &payload.snippets {
                let key = snippet_key(rs);
                let base = state.snippets.get(&key).cloned();
                let rfp = snippet_fp(rs);
                match snippets.iter().position(|s| snippet_key(s) == key) {
                    None if base.is_none() => {
                        let saved = self
                            .history
                            .save_snippet(&Snippet {
                                id: 0,
                                ..rs.clone()
                            })
                            .await?;
                        snippets.push(saved);
                        report.pulled_snippets += 1;
                    }
                    Some(i) => {
                        let lfp = snippet_fp(&snippets[i]);
                        if lfp != rfp && base.as_deref() == Some(lfp.as_str()) {
                            let saved = self
                                .history
                                .save_snippet(&Snippet {
                                    id: snippets[i].id,
                                    ..rs.clone()
                                })
                                .await?;
                            snippets[i] = saved;
                            report.pulled_snippets += 1;
                        }
                    }
                    None => {}
                }
            }
        }

        // ---- dashboards (same three-way rule, keyed by id)
        let dstore = self
            .dashboards
            .clone()
            .map(crate::dashboards::DashboardStore::at);
        let mut dashes = match &dstore {
            Some(ds) => ds.list()?,
            None => remote
                .as_ref()
                .map(|r| r.0.dashboards.clone())
                .unwrap_or_default(),
        };
        if let (Some(ds), Some((payload, _))) = (&dstore, &remote) {
            for rd in &payload.dashboards {
                let base = state.dashboards.get(&rd.id).cloned();
                let rfp = dash_fp(rd);
                match dashes.iter().position(|d| d.id == rd.id) {
                    None if base.is_none() => {
                        let saved = ds.save(rd.clone())?;
                        dashes.push(saved);
                        report.pulled_dashboards += 1;
                    }
                    Some(i) => {
                        let lfp = dash_fp(&dashes[i]);
                        if lfp != rfp && base.as_deref() == Some(lfp.as_str()) {
                            dashes[i] = ds.save(rd.clone())?;
                            report.pulled_dashboards += 1;
                        }
                    }
                    None => {}
                }
            }
        }
        dashes.sort_by(|a, b| a.id.cmp(&b.id));
        let dash_fps: Vec<String> = dashes.iter().map(dash_fp).collect();

        // ---- push when our view differs from the file
        if include_secrets {
            if let Some((p, _)) = &remote {
                for l in local.iter_mut() {
                    if let Some(r) = p
                        .connections
                        .iter()
                        .find(|r| r.config.id == l.config.id && conn_fp(r) == conn_fp(l))
                    {
                        l.password = l.password.take().or_else(|| r.password.clone());
                        l.ssh_secret = l.ssh_secret.take().or_else(|| r.ssh_secret.clone());
                    }
                }
            }
        } else {
            for l in local.iter_mut() {
                l.password = None;
                l.ssh_secret = None;
            }
        }
        let mut sorted = local.clone();
        sorted.sort_by(|a, b| a.config.id.cmp(&b.config.id));
        let mut snips: Vec<Snippet> = snippets
            .iter()
            .map(|s| Snippet { id: 0, ..s.clone() })
            .collect();
        snips.sort_by_key(snippet_key);
        let changed = match &remote {
            None => true,
            Some((p, _)) => {
                let mut rc = p.connections.clone();
                rc.sort_by(|a, b| a.config.id.cmp(&b.config.id));
                let mut rs: Vec<Snippet> = p
                    .snippets
                    .iter()
                    .map(|s| Snippet { id: 0, ..s.clone() })
                    .collect();
                rs.sort_by_key(snippet_key);
                let mut rd = p.dashboards.clone();
                rd.sort_by(|a, b| a.id.cmp(&b.id));
                rd.iter().map(dash_fp).collect::<Vec<_>>() != dash_fps
                    || serde_json::to_vec(&rc)? != serde_json::to_vec(&sorted)?
                    || fp(&rs.iter().map(snippet_fp).collect::<Vec<_>>())
                        != fp(&snips.iter().map(snippet_fp).collect::<Vec<_>>())
            }
        };
        if changed {
            let payload = Payload {
                device: device_name(),
                at: now(),
                connections: sorted.clone(),
                snippets: snips.clone(),
                dashboards: dashes.clone(),
            };
            let env = seal(pass, &serde_json::to_vec(&payload)?, salt)?;
            let tmp = folder.join(format!(".{FILE_NAME}.tmp"));
            std::fs::write(&tmp, serde_json::to_vec_pretty(&env)?)?;
            std::fs::rename(&tmp, &file)?;
            report.pushed = true;
        }

        state.conns = sorted
            .iter()
            .map(|c| (c.config.id.clone(), conn_fp(c)))
            .collect();
        state.snippets = snips
            .iter()
            .map(|s| (snippet_key(s), snippet_fp(s)))
            .collect();
        state.dashboards = dashes.iter().map(|d| (d.id.clone(), dash_fp(d))).collect();
        state.last_sync = Some(now());
        state.last_error = None;
        state.folder = Some(folder.to_string_lossy().into());
        state.include_secrets = include_secrets;
        state.save_to(&self.state_path)?;
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_wrong_pass() {
        let env = seal("correct horse", b"hello", None).unwrap();
        assert_eq!(open("correct horse", &env).unwrap(), b"hello");
        assert!(open("wrong horse!", &env)
            .unwrap_err()
            .to_string()
            .contains("wrong passphrase"));
        assert!(seal("short", b"x", None).is_err());
        assert_eq!(base64_encode(b"ab"), "YWI=");
        assert_eq!(base64_encode(b"abc"), "YWJj");
    }
}
