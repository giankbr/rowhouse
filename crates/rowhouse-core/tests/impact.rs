//! Preview an UPDATE / DELETE in a rolled-back transaction, then commit with an exact row check.
use rowhouse_core::{Connection, ConnectionConfig, DbKind};
use serde_json::json;

async fn exercise(c: &Connection, t: &str) {
    c.query(&format!("DROP TABLE IF EXISTS {t}"), 1)
        .await
        .unwrap();
    c.query(
        &format!("CREATE TABLE {t} (id INTEGER PRIMARY KEY, status VARCHAR(20), total INTEGER)"),
        1,
    )
    .await
    .unwrap();
    c.query(
        &format!("INSERT INTO {t} VALUES (1, 'new', 10), (2, 'new', 20), (3, 'paid', 30)"),
        1,
    )
    .await
    .unwrap();

    // UPDATE: before → after, nothing actually changed
    let sql = format!("UPDATE {t} SET status = 'shipped', total = total + 1 WHERE status = 'new'");
    let p = c.preview_dml(&sql).await.unwrap();
    assert_eq!(p.kind, "update");
    assert_eq!(p.table, t);
    assert_eq!(p.affected, 2);
    assert_eq!(p.key, ["id"]);
    assert_eq!(p.before.rows.len(), 2);
    let after = p.after.unwrap();
    assert_eq!(after.rows.len(), 2);
    let status = after
        .columns
        .iter()
        .position(|c| c.name == "status")
        .unwrap();
    assert!(after.rows.iter().all(|r| r[status] == json!("shipped")));
    let still = c
        .query(&format!("SELECT count(*) FROM {t} WHERE status = 'new'"), 1)
        .await
        .unwrap();
    assert_eq!(
        still.results[0].rows[0][0].to_string(),
        "2",
        "the preview was rolled back"
    );

    // commit refuses when the data moved on since the preview
    c.query(&format!("INSERT INTO {t} VALUES (4, 'new', 40)"), 1)
        .await
        .unwrap();
    let e = c.commit_dml(&sql, 2).await.unwrap_err().to_string();
    assert!(e.contains("expected 2 rows, 3 affected"), "{e}");
    let still = c
        .query(&format!("SELECT count(*) FROM {t} WHERE status = 'new'"), 1)
        .await
        .unwrap();
    assert_eq!(still.results[0].rows[0][0].to_string(), "3", "rolled back");
    assert_eq!(c.commit_dml(&sql, 3).await.unwrap(), 3);

    // DELETE: the rows that would go
    let p = c
        .preview_dml(&format!("DELETE FROM {t} WHERE total > 25"))
        .await
        .unwrap();
    assert_eq!(
        (p.kind.as_str(), p.affected, p.before.rows.len()),
        ("delete", 2, 2)
    );
    assert!(p.after.is_none());
    assert!(c.preview_dml("SELECT 1").await.is_err());
}

#[tokio::test]
async fn sqlite_preview_and_commit() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("i.db");
    std::fs::File::create(&db).unwrap();
    let mut cfg = ConnectionConfig::new("lite", DbKind::Sqlite);
    cfg.file = Some(db.to_string_lossy().into());
    let c = Connection::connect(&cfg).await.unwrap();
    exercise(&c, "rh_impact").await;

    // read-only profiles can't preview writes
    cfg.read_only = true;
    let ro = Connection::connect(&cfg).await.unwrap();
    assert!(ro.preview_dml("DELETE FROM rh_impact").await.is_err());
    assert!(ro.commit_dml("DELETE FROM rh_impact", 0).await.is_err());
}

#[tokio::test]
async fn postgres_preview_and_commit() {
    let Ok(spec) = std::env::var("ROWHOUSE_TEST_PG") else {
        return;
    };
    let mut p = spec.split(':');
    let mut cfg = ConnectionConfig::new("pg", DbKind::Postgres);
    cfg.host = p.next().map(Into::into);
    cfg.port = p.next().and_then(|x| x.parse().ok());
    cfg.user = p.next().map(Into::into);
    cfg.database = Some("postgres".into());
    let pw = std::env::var("ROWHOUSE_TEST_PG_PASSWORD").ok();
    let c = Connection::connect_with(&cfg, pw.as_deref()).await.unwrap();
    exercise(&c, "rh_impact").await;
}

#[tokio::test]
async fn mysql_preview_and_commit() {
    let Ok(spec) = std::env::var("ROWHOUSE_TEST_MYSQL") else {
        return;
    };
    let p: Vec<&str> = spec.split(':').collect();
    let mut cfg = ConnectionConfig::new("my", DbKind::Mysql);
    cfg.host = Some(p[0].into());
    cfg.port = p.get(1).and_then(|x| x.parse().ok());
    cfg.user = p.get(2).map(|s| s.to_string());
    cfg.database = p.get(4).map(|s| s.to_string());
    let c = Connection::connect_with(&cfg, p.get(3).copied())
        .await
        .unwrap();
    exercise(&c, "rh_impact").await;
}
