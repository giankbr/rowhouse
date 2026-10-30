//! Server-side cancel of a running query (needs ROWHOUSE_TEST_PG / ROWHOUSE_TEST_MYSQL).
use std::{sync::Arc, time::Instant};

use rowhouse_core::{Connection, ConnectionConfig, DbKind};

async fn cancel_after(c: Arc<Connection>, sql: &'static str) -> (std::time::Duration, bool) {
    let started = Instant::now();
    let runner = {
        let c = c.clone();
        tokio::spawn(async move { c.query_tagged(sql, 10, "run-1").await })
    };
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    assert!(
        c.cancel("run-1").await.unwrap(),
        "server-side cancel available"
    );
    let r = runner.await.unwrap();
    (started.elapsed(), r.is_err())
}

#[tokio::test]
async fn postgres_cancel() {
    let Ok(spec) = std::env::var("ROWHOUSE_TEST_PG") else {
        return;
    };
    let mut p = spec.split(':');
    let mut cfg = ConnectionConfig::new("pg", DbKind::Postgres);
    cfg.host = p.next().map(Into::into);
    cfg.port = p.next().and_then(|x| x.parse().ok());
    cfg.user = p.next().map(Into::into);
    cfg.database = Some("postgres".into());
    let c = Arc::new(
        Connection::connect_with(
            &cfg,
            std::env::var("ROWHOUSE_TEST_PG_PASSWORD").ok().as_deref(),
        )
        .await
        .unwrap(),
    );
    let (took, errored) = cancel_after(c.clone(), "SELECT pg_sleep(30)").await;
    assert!(took.as_secs() < 5, "cancelled quickly ({took:?})");
    assert!(errored, "pg reports the cancel as an error");
    // the connection pool still works afterwards
    c.query("SELECT 1", 1).await.unwrap();
    // nothing to cancel → false
    assert!(!c.cancel("nope").await.unwrap());
}

#[tokio::test]
async fn mysql_cancel() {
    let Ok(spec) = std::env::var("ROWHOUSE_TEST_MYSQL") else {
        return;
    };
    let p: Vec<&str> = spec.split(':').collect();
    let mut cfg = ConnectionConfig::new("my", DbKind::Mysql);
    cfg.host = Some(p[0].into());
    cfg.port = p[1].parse().ok();
    cfg.user = Some(p[2].into());
    cfg.database = Some(p[4].into());
    let c = Arc::new(Connection::connect_with(&cfg, Some(p[3])).await.unwrap());
    let (took, _) = cancel_after(c.clone(), "SELECT SLEEP(30)").await;
    assert!(took.as_secs() < 5, "cancelled quickly ({took:?})");
    c.query("SELECT 1", 1).await.unwrap();
}
