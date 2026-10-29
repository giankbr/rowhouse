//! MySQL and CockroachDB — run only when configured (CI provides both):
//! `ROWHOUSE_TEST_MYSQL=host:port:user:password:db`, `ROWHOUSE_TEST_COCKROACH=host:port:user`
use rowhouse_core::{ChangeSet, Connection, ConnectionConfig, DbKind, RowUpdate};

fn obj(v: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    v.as_object().unwrap().clone()
}

async fn crud(c: &Connection, schema: &str, create: &str) {
    c.query(&format!("DROP TABLE IF EXISTS rh_t; {create}; INSERT INTO rh_t (id, name) VALUES (1, 'a'), (2, 'b')"), 10)
        .await
        .unwrap();
    let cols = c.describe_table(schema, "rh_t").await.unwrap();
    assert!(cols[0].primary_key, "{cols:?}");
    assert!(c
        .list_tables()
        .await
        .unwrap()
        .iter()
        .any(|t| t.name == "rh_t"));
    let cs = ChangeSet {
        schema: schema.into(),
        table: "rh_t".into(),
        // second update sets an identical value (MySQL reports 0 rows)
        updates: vec![
            RowUpdate {
                key: obj(serde_json::json!({"id": 1})),
                set: obj(serde_json::json!({"name": "a'1"})),
            },
            RowUpdate {
                key: obj(serde_json::json!({"id": 2})),
                set: obj(serde_json::json!({"name": "b"})),
            },
        ],
        inserts: vec![obj(serde_json::json!({"id": 3, "name": "c\\d"}))],
        deletes: vec![],
    };
    c.apply_changes(&cs).await.unwrap();
    let out = c
        .query("SELECT id, name FROM rh_t ORDER BY id", 10)
        .await
        .unwrap();
    assert_eq!(out.results[0].rows[0][1], "a'1");
    assert_eq!(out.results[0].rows[2][1], "c\\d");
    assert!(c.query_read_only("DELETE FROM rh_t", 10).await.is_err());
    c.query("DROP TABLE rh_t", 10).await.unwrap();
}

#[tokio::test]
async fn mysql() {
    let Ok(spec) = std::env::var("ROWHOUSE_TEST_MYSQL") else {
        return;
    };
    let p: Vec<&str> = spec.split(':').collect();
    let mut cfg = ConnectionConfig::new("my", DbKind::Mysql);
    cfg.host = Some(p[0].into());
    cfg.port = p[1].parse().ok();
    cfg.user = Some(p[2].into());
    cfg.database = Some(p[4].into());
    let c = Connection::connect_with(&cfg, Some(p[3])).await.unwrap();
    crud(
        &c,
        p[4],
        "CREATE TABLE rh_t (id INT PRIMARY KEY, name VARCHAR(20))",
    )
    .await;
}

#[tokio::test]
async fn cockroach() {
    let Ok(spec) = std::env::var("ROWHOUSE_TEST_COCKROACH") else {
        return;
    };
    let p: Vec<&str> = spec.split(':').collect();
    let mut cfg = ConnectionConfig::new("crdb", DbKind::Cockroach);
    cfg.host = Some(p[0].into());
    cfg.port = p[1].parse().ok();
    cfg.user = Some(p[2].into());
    cfg.database = Some("defaultdb".into());
    let c = Connection::connect_with(&cfg, None).await.unwrap();
    crud(
        &c,
        "public",
        "CREATE TABLE rh_t (id INT PRIMARY KEY, name STRING)",
    )
    .await;
}
