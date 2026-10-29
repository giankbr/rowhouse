//! SSH local port-forward: 127.0.0.1:<random> → (ssh host) → db_host:db_port.
//!
//! Host-key policy, in order:
//! 1. profile has a pinned `hostKey` fingerprint → must match exactly
//! 2. host is in `~/.ssh/known_hosts` → must match (a changed key is rejected)
//! 3. otherwise trust on first use; the fingerprint is exposed via
//!    [`Tunnel::host_key`] so the app can pin it in the profile.

use std::sync::{Arc, Mutex};

use russh::{
    client,
    keys::{self, HashAlg, PrivateKeyWithHashAlg},
};
use tokio::{net::TcpListener, task::JoinHandle};

use crate::{
    config::{SshAuth, SshConfig},
    Error, Result,
};

struct Verifier {
    host: String,
    port: u16,
    pinned: Option<String>,
    seen: Arc<Mutex<Option<String>>>,
    rejected: Arc<Mutex<Option<String>>>,
}

impl client::Handler for Verifier {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_key: &keys::PublicKeyOrCertificate,
    ) -> std::result::Result<bool, Self::Error> {
        let key = &server_key.public_key();
        let fp = key.fingerprint(HashAlg::Sha256).to_string();
        *self.seen.lock().unwrap() = Some(fp.clone());
        if let Some(pin) = &self.pinned {
            if pin != &fp {
                *self.rejected.lock().unwrap() =
                    Some(format!("SSH host key changed! expected {pin}, got {fp}"));
                return Ok(false);
            }
            return Ok(true);
        }
        match keys::check_known_hosts(&self.host, self.port, key) {
            Ok(_) => Ok(true), // known & matching, or unknown → TOFU
            Err(keys::Error::KeyChanged { line }) => {
                *self.rejected.lock().unwrap() = Some(format!(
                    "SSH host key for {} does not match ~/.ssh/known_hosts (line {line})",
                    self.host
                ));
                Ok(false)
            }
            Err(_) => Ok(true), // no known_hosts file
        }
    }
}

pub struct Tunnel {
    pub local_port: u16,
    /// SHA256 fingerprint of the server key that was accepted.
    pub host_key: String,
    session: Arc<client::Handle<Verifier>>,
    task: JoinHandle<()>,
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        self.task.abort();
        let s = self.session.clone();
        tokio::spawn(async move {
            let _ = s
                .disconnect(russh::Disconnect::ByApplication, "", "en")
                .await;
        });
    }
}

fn expand_home(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = directories::BaseDirs::new() {
            return home.home_dir().join(rest).to_string_lossy().into_owned();
        }
    }
    path.to_string()
}

impl Tunnel {
    pub async fn open(
        cfg: &SshConfig,
        secret: Option<&str>,
        target_host: &str,
        target_port: u16,
    ) -> Result<Self> {
        let port = cfg.port.unwrap_or(22);
        let seen = Arc::new(Mutex::new(None));
        let rejected = Arc::new(Mutex::new(None));
        let handler = Verifier {
            host: cfg.host.clone(),
            port,
            pinned: cfg.host_key.clone(),
            seen: seen.clone(),
            rejected: rejected.clone(),
        };
        let config = Arc::new(client::Config {
            nodelay: true,
            keepalive_interval: Some(std::time::Duration::from_secs(30)),
            ..Default::default()
        });

        let mut session = match client::connect(config, (cfg.host.as_str(), port), handler).await {
            Ok(s) => s,
            Err(e) => {
                let why = rejected.lock().unwrap().take();
                return Err(Error::Ssh(
                    why.unwrap_or_else(|| format!("connect {}:{port}: {e}", cfg.host)),
                ));
            }
        };

        let ok = match cfg.auth {
            SshAuth::Password => session
                .authenticate_password(&cfg.user, secret.unwrap_or_default())
                .await
                .map_err(|e| Error::Ssh(e.to_string()))?
                .success(),
            SshAuth::Key => {
                let path = expand_home(cfg.key_path.as_deref().unwrap_or("~/.ssh/id_ed25519"));
                let key = keys::load_secret_key(&path, secret.filter(|s| !s.is_empty()))
                    .map_err(|e| Error::Ssh(format!("load key {path}: {e}")))?;
                let hash = session
                    .best_supported_rsa_hash()
                    .await
                    .map_err(|e| Error::Ssh(e.to_string()))?
                    .flatten();
                session
                    .authenticate_publickey(
                        &cfg.user,
                        PrivateKeyWithHashAlg::new(Arc::new(key), hash),
                    )
                    .await
                    .map_err(|e| Error::Ssh(e.to_string()))?
                    .success()
            }
        };
        if !ok {
            return Err(Error::Ssh(format!(
                "authentication failed for {}@{}",
                cfg.user, cfg.host
            )));
        }

        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let local_port = listener.local_addr()?.port();
        let session = Arc::new(session);
        let target_host = target_host.to_string();

        let s = session.clone();
        let task = tokio::spawn(async move {
            while let Ok((mut socket, peer)) = listener.accept().await {
                let s = s.clone();
                let host = target_host.clone();
                tokio::spawn(async move {
                    let Ok(channel) = s
                        .channel_open_direct_tcpip(
                            host,
                            target_port as u32,
                            peer.ip().to_string(),
                            peer.port() as u32,
                        )
                        .await
                    else {
                        return;
                    };
                    let mut stream = channel.into_stream();
                    let _ = tokio::io::copy_bidirectional(&mut socket, &mut stream).await;
                });
            }
        });

        let host_key = seen.lock().unwrap().clone().unwrap_or_default();
        Ok(Self {
            local_port,
            host_key,
            session,
            task,
        })
    }
}
