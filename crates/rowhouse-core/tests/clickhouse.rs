//! Runs only when ROWHOUSE_TEST_CH=host:port[:user:password] is set.
use rowhouse_core::{Connection, ConnectionConfig, DbKind};

#[tokio::test]
async fn clickhouse_http() {
    let Ok(spec) = std::env::var("ROWHOUSE_TEST_CH") else {
        return;
    };
    let p: Vec<&str> = spec.splitn(4, ':').collect();
    let mut cfg = ConnectionConfig::new("ch", DbKind::Clickhouse);
    cfg.host = Some(p[0].into());
    cfg.port = p[1].parse().ok();
    cfg.user = Some(p.get(2).copied().unwrap_or("default").into());
    cfg.database = Some("default".into());
    let c = Connection::connect_with(&cfg, p.get(3).copied())
        .await
        .unwrap();

    let out = c
        .query(
            "DROP TABLE IF EXISTS rh_t; CREATE TABLE rh_t (id UInt64, name String, v Nullable(Float64)) ENGINE = MergeTree ORDER BY id;
             INSERT INTO rh_t VALUES (1, 'a', 1.5), (2, 'b', NULL);
             SELECT id, name, v FROM rh_t ORDER BY id",
            1,
        )
        .await
        .unwrap();
    let r = out.results.last().unwrap();
    assert_eq!(r.columns[0].data_type, "UInt64");
    assert_eq!(
        r.rows,
        vec![vec![
            serde_json::json!(1),
            "a".into(),
            serde_json::json!(1.5)
        ]]
    );
    assert!(r.truncated);

    let tables = c.list_tables().await.unwrap();
    assert!(tables.iter().any(|t| t.name == "rh_t"));
    let cols = c.describe_table("default", "rh_t").await.unwrap();
    assert_eq!(cols.len(), 3);
    assert!(cols[0].primary_key);
    assert!(cols[2].nullable && !cols[1].nullable);

    // server-side readonly=1
    assert!(c
        .query_read_only("SELECT count() FROM rh_t", 10)
        .await
        .is_ok());
    assert!(c.query_read_only("DROP TABLE rh_t", 10).await.is_err());

    // quoting: backslashes must be escaped for ClickHouse
    let lit = c.literal("a\\'b");
    let out = c.query(&format!("SELECT {lit} AS s"), 10).await.unwrap();
    assert_eq!(out.results[0].rows[0][0], "a\\'b");
    assert_eq!(
        c.select_table_sql("default", "rh_t", 3),
        "SELECT * FROM `default`.`rh_t` LIMIT 3"
    );
    c.query("DROP TABLE rh_t", 10).await.unwrap();
}
