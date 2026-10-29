//! Postgres over TLS. Runs only when ROWHOUSE_TEST_PG_TLS=host:port:/path/to/certdir is set; the dir holds
//! ca.pem, client.pem, client-key.pem (client CN = tlsuser) and the server cert is valid for `localhost` only.
use rowhouse_core::{
    config::{TlsConfig, TlsMode},
    Connection, ConnectionConfig, DbKind,
};

fn cfg(host: &str, port: u16, user: &str, tls: TlsConfig) -> ConnectionConfig {
    let mut c = ConnectionConfig::new("tls", DbKind::Postgres);
    c.host = Some(host.into());
    c.port = Some(port);
    c.user = Some(user.into());
    c.database = Some("postgres".into());
    c.tls = Some(tls);
    c
}

#[tokio::test]
async fn postgres_tls_modes() {
    let Ok(spec) = std::env::var("ROWHOUSE_TEST_PG_TLS") else {
        return;
    };
    let p: Vec<&str> = spec.splitn(3, ':').collect();
    let (host, port, dir) = (p[0], p[1].parse::<u16>().unwrap(), p[2]);
    let ca = Some(format!("{dir}/ca.pem"));

    // verify-full against our CA
    let c = Connection::connect_with(
        &cfg(
            host,
            port,
            "postgres",
            TlsConfig {
                mode: TlsMode::VerifyFull,
                ca_cert: ca.clone(),
                ..Default::default()
            },
        ),
        None,
    )
    .await
    .unwrap();
    let out = c
        .query(
            "SELECT ssl FROM pg_stat_ssl WHERE pid = pg_backend_pid()",
            1,
        )
        .await
        .unwrap();
    assert_eq!(out.results[0].rows[0][0], true);

    // wrong host name → verify-full must fail, verify-ca must pass
    let by_ip = |mode| {
        cfg(
            "127.0.0.1",
            port,
            "postgres",
            TlsConfig {
                mode,
                ca_cert: ca.clone(),
                ..Default::default()
            },
        )
    };
    assert!(Connection::connect_with(&by_ip(TlsMode::VerifyFull), None)
        .await
        .is_err());
    assert!(Connection::connect_with(&by_ip(TlsMode::VerifyCa), None)
        .await
        .is_ok());

    // no CA → verify fails against an unknown issuer; require still encrypts
    let no_ca = |mode| {
        cfg(
            host,
            port,
            "postgres",
            TlsConfig {
                mode,
                ..Default::default()
            },
        )
    };
    assert!(Connection::connect_with(&no_ca(TlsMode::VerifyFull), None)
        .await
        .is_err());
    assert!(Connection::connect_with(&no_ca(TlsMode::Require), None)
        .await
        .is_ok());

    // mutual TLS: tlsuser is only allowed with a client certificate
    let mtls = TlsConfig {
        mode: TlsMode::VerifyFull,
        ca_cert: ca.clone(),
        client_cert: Some(format!("{dir}/client.pem")),
        client_key: Some(format!("{dir}/client-key.pem")),
    };
    let c = Connection::connect_with(&cfg(host, port, "tlsuser", mtls), None)
        .await
        .unwrap();
    assert_eq!(
        c.query("SELECT current_user", 1).await.unwrap().results[0].rows[0][0],
        "tlsuser"
    );
    let no_cert = TlsConfig {
        mode: TlsMode::VerifyFull,
        ca_cert: ca,
        ..Default::default()
    };
    assert!(
        Connection::connect_with(&cfg(host, port, "tlsuser", no_cert), None)
            .await
            .is_err()
    );
}
