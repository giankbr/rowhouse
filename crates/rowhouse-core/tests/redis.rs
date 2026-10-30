//! Runs only when ROWHOUSE_TEST_REDIS=host:port is set.
use rowhouse_core::{browse::PageRequest, Connection, ConnectionConfig, DbKind};

#[tokio::test]
async fn redis_commands_keys_and_pages() {
    let Ok(spec) = std::env::var("ROWHOUSE_TEST_REDIS") else {
        return;
    };
    let (host, port) = spec.split_once(':').unwrap();
    let mut cfg = ConnectionConfig::new("r", DbKind::Redis);
    cfg.host = Some(host.into());
    cfg.port = port.parse().ok();
    cfg.database = Some("3".into());
    let c = Connection::connect_with(&cfg, None).await.unwrap();
    c.ping().await.unwrap();
    let out = c
        .query(
            "FLUSHDB
             HSET user:1 name \"Ana Putri\" city Jakarta
             RPUSH queue:jobs a b c d e
             ZADD board 10 ana 20 budi
             SET greeting 'hello world'
             HGETALL user:1
             ZRANGE board 0 -1 WITHSCORES
             GET greeting",
            100,
        )
        .await
        .unwrap();
    let r = &out.results;
    assert_eq!(r[5].columns[0].name, "field");
    assert!(r[5].rows.contains(&vec!["name".into(), "Ana Putri".into()]));
    assert_eq!(
        r[6].rows,
        vec![
            vec![serde_json::json!("ana"), "10".into()],
            vec!["budi".into(), "20".into()]
        ]
    );
    assert_eq!(r[7].rows[0][0], "hello world");

    let keys = c.list_tables().await.unwrap();
    assert!(keys
        .iter()
        .any(|t| t.schema == "user" && t.name == "user:1"));
    assert!(keys
        .iter()
        .any(|t| t.schema == "keys" && t.name == "greeting"));
    assert_eq!(
        c.describe_table("queue", "queue:jobs").await.unwrap()[0].data_type,
        "list"
    );

    let page = |t: &str, offset| PageRequest {
        schema: String::new(),
        table: t.into(),
        filters: vec![],
        sort: None,
        limit: 2,
        offset,
        count: true,
    };
    let p = c.table_page(&page("queue:jobs", 2)).await.unwrap();
    assert_eq!(p.total, Some(5));
    assert_eq!(
        p.result.rows,
        vec![
            vec![serde_json::json!(2), "c".into()],
            vec![serde_json::json!(3), "d".into()]
        ]
    );
    assert_eq!(p.sql, "LRANGE queue:jobs 2 3");

    // read-only allowlist
    assert!(c.query_read_only("HGETALL user:1", 10).await.is_ok());
    assert!(c.query_read_only("DEL user:1", 10).await.is_err());
    assert!(
        c.query("SELECT 2", 10).await.is_err(),
        "SELECT would switch the shared connection's db"
    );
    c.query("FLUSHDB", 1).await.unwrap();
}
