//! Pro licenses: signed offline, verified offline (no phoning home).
//!
//! A key looks like `RH1-<payload>.<signature>` (both base64url): the payload is
//! JSON, the signature Ed25519 over the payload bytes. The public key is baked in
//! at build time from `ROWHOUSE_LICENSE_PUBKEY` (base64url, 32 bytes).
//!
//! Builds without a public key (anyone building from source) are *community
//! builds*: every feature is unlocked. Official releases set the key in CI, and
//! there the Pro features ask for a license.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{config::ConnectionStore, Error, Result};

const PREFIX: &str = "RH1-";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct License {
    pub email: String,
    /// "pro" | "team"
    pub plan: String,
    #[serde(default = "one")]
    pub seats: u32,
    /// unix seconds
    pub issued: i64,
    /// updates included until (unix seconds); the license keeps working after
    pub updates_until: i64,
    #[serde(default)]
    pub id: String,
}

fn one() -> u32 {
    1
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LicenseStatus {
    /// built without a license key: everything is unlocked
    pub community_build: bool,
    pub pro: bool,
    pub license: Option<License>,
    /// release date of this build is past `updates_until`
    pub updates_expired: bool,
    pub error: Option<String>,
}

fn b64url_decode(s: &str) -> Option<Vec<u8>> {
    let std: String = s
        .trim()
        .chars()
        .map(|c| match c {
            '-' => '+',
            '_' => '/',
            c => c,
        })
        .collect();
    crate::backend::libsql::base64_decode(&std)
}

/// Public key compiled into this build, if any.
pub fn public_key() -> Option<Vec<u8>> {
    option_env!("ROWHOUSE_LICENSE_PUBKEY")
        .filter(|k| !k.trim().is_empty())
        .and_then(b64url_decode)
        .filter(|k| k.len() == 32)
}

/// Check a key against `pubkey` (32-byte Ed25519).
pub fn verify_with(key: &str, pubkey: &[u8]) -> Result<License> {
    let body = key
        .trim()
        .strip_prefix(PREFIX)
        .ok_or_else(|| Error::Other("not a Rowhouse license key".into()))?;
    let (payload, sig) = body
        .split_once('.')
        .ok_or_else(|| Error::Other("license key is incomplete".into()))?;
    let payload_bytes =
        b64url_decode(payload).ok_or_else(|| Error::Other("license key is damaged".into()))?;
    let sig = b64url_decode(sig).ok_or_else(|| Error::Other("license key is damaged".into()))?;
    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, pubkey)
        .verify(&payload_bytes, &sig)
        .map_err(|_| {
            Error::Other("license key signature doesn't match — check for typos".into())
        })?;
    let lic: License = serde_json::from_slice(&payload_bytes)?;
    if lic.plan != "pro" && lic.plan != "team" {
        return Err(Error::Other(format!("unknown plan `{}`", lic.plan)));
    }
    Ok(lic)
}

fn path() -> PathBuf {
    ConnectionStore::default_dir().join("license.key")
}

/// Unix seconds of this build (set by CI as `ROWHOUSE_BUILD_TIME`), else now.
fn build_time() -> i64 {
    option_env!("ROWHOUSE_BUILD_TIME")
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(crate::schedules::now)
}

pub fn status() -> LicenseStatus {
    let Some(pk) = public_key() else {
        return LicenseStatus {
            community_build: true,
            pro: true,
            license: None,
            updates_expired: false,
            error: None,
        };
    };
    let key = std::fs::read_to_string(path()).unwrap_or_default();
    if key.trim().is_empty() {
        return LicenseStatus {
            community_build: false,
            pro: false,
            license: None,
            updates_expired: false,
            error: None,
        };
    }
    match verify_with(&key, &pk) {
        Ok(lic) => LicenseStatus {
            community_build: false,
            pro: true,
            updates_expired: build_time() > lic.updates_until,
            license: Some(lic),
            error: None,
        },
        Err(e) => LicenseStatus {
            community_build: false,
            pro: false,
            license: None,
            updates_expired: false,
            error: Some(e.to_string()),
        },
    }
}

/// Verify and store a key (in `license.key` next to the connections).
pub fn activate(key: &str) -> Result<LicenseStatus> {
    let pk = public_key().ok_or_else(|| {
        Error::Other("this is a community build — everything is already unlocked".into())
    })?;
    verify_with(key, &pk)?;
    let p = path();
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(p, key.trim())?;
    Ok(status())
}

pub fn deactivate() -> Result<()> {
    match std::fs::remove_file(path()) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

/// `Ok` when Pro features may be used (licensed, or a community build).
pub fn require_pro(feature: &str) -> Result<()> {
    if status().pro {
        Ok(())
    } else {
        Err(Error::Other(format!(
            "{feature} is a Rowhouse Pro feature — add your license in Settings → License"
        )))
    }
}
