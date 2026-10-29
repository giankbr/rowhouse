//! Postgres through an SSH tunnel. Runs only when both are configured:
//! `ROWHOUSE_TEST_PG=localhost:5432:postgres ROWHOUSE_TEST_SSH=127.0.0.1:22:user:/path/key [ROWHOUSE_TEST_SSH_PASSPHRASE=..]`
use rowhouse_core::{config::SshAuth, config::SshConfig, Connection, ConnectionConfig, DbKind};

fn setup() -> Option<(ConnectionConfig, SshConfig)> {
    let pg = std::env::var("ROWHOUSE_TEST_PG").ok()?;
    let ssh = std::env::var("ROWHOUSE_TEST_SSH").ok()?;
    let p: Vec<&str> = pg.split(':').collect();
    let s: Vec<&str> = ssh.split(':').collect();
    let mut cfg = ConnectionConfig::new("tunnel", DbKind::Postgres);
    cfg.host = Some(p[0].into());
    cfg.port = p[1].parse().ok();
    cfg.user = Some(p[2].into());
    cfg.database = Some("postgres".into());
    let ssh = SshConfig {
        host: s[0].into(),
        port: s[1].parse().ok(),
        user: s[2].into(),
        auth: SshAuth::Key,
        key_path: Some(s[3].into()),
        host_key: None,
    };
    Some((cfg, ssh))
}

#[tokio::test]
async fn query_through_tunnel_and_pin_host_key() {
    let Some((mut cfg, ssh)) = setup() else {
        return;
    };
    let pass = std::env::var("ROWHOUSE_TEST_SSH_PASSPHRASE").ok();
    cfg.ssh = Some(ssh);

    // 1. trust on first use → fingerprint exposed
    let c = Connection::connect_with_secrets(&cfg, None, pass.as_deref())
        .await
        .unwrap();
    let out = c
        .query("SELECT inet_server_port(), 40 + 2", 10)
        .await
        .unwrap();
    assert_eq!(out.results[0].rows[0][1], 42);
    let fp = c.ssh_host_key().unwrap().to_string();
    assert!(fp.starts_with("SHA256:"), "{fp}");
    c.close().await;

    // 2. pinned & matching → ok
    cfg.ssh.as_mut().unwrap().host_key = Some(fp);
    let c = Connection::connect_with_secrets(&cfg, None, pass.as_deref())
        .await
        .unwrap();
    c.ping().await.unwrap();

    // 3. pinned & different → refused before auth
    cfg.ssh.as_mut().unwrap().host_key = Some("SHA256:not-the-right-key".into());
    let err = Connection::connect_with_secrets(&cfg, None, pass.as_deref())
        .await
        .err()
        .unwrap()
        .to_string();
    assert!(err.contains("host key changed"), "{err}");
}

#[tokio::test]
async fn bad_key_path_is_a_clear_error() {
    let Some((mut cfg, mut ssh)) = setup() else {
        return;
    };
    ssh.key_path = Some("/nope/id_ed25519".into());
    cfg.ssh = Some(ssh);
    let err = Connection::connect_with_secrets(&cfg, None, None)
        .await
        .err()
        .unwrap()
        .to_string();
    assert!(err.starts_with("ssh: load key"), "{err}");
}
