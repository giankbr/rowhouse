//! Unit checks always run; the live test needs ROWHOUSE_TEST_CASSANDRA=host:port.
use rowhouse_core::{
    backend::cassandra::{check_read_only, from_json_literal, statements, to_json},
    browse::{Filter, FilterOp, PageRequest, Sort},
    ChangeSet, Connection, ConnectionConfig, DbKind, RowUpdate,
};
use scylla::value::{CqlTimestamp, CqlValue};
use serde_json::json;

#[test]
fn batches_stay_together() {
    let s = statements(
        "SELECT * FROM a; BEGIN BATCH INSERT INTO t (k) VALUES (1); UPDATE t SET v = 'x;y' WHERE k = 2; APPLY BATCH; SELECT 1 FROM b",
    );
    assert_eq!(s.len(), 3, "{s:#?}");
    assert!(
        s[1].starts_with("BEGIN BATCH") && s[1].ends_with("APPLY BATCH") && s[1].contains("'x;y'")
    );
}

#[test]
fn literals_and_values() {
    assert_eq!(from_json_literal(&json!("it's")), r#"fromJson('"it''s"')"#);
    assert_eq!(from_json_literal(&json!([1, 2])), "fromJson('[1,2]')");
    assert_eq!(from_json_literal(&json!(null)), "NULL");
    assert_eq!(
        to_json(&CqlValue::Timestamp(CqlTimestamp(0))),
        json!("1970-01-01T00:00:00.000Z")
    );
    assert_eq!(
        to_json(&CqlValue::Map(vec![(
            CqlValue::Text("a".into()),
            CqlValue::Int(1)
        )])),
        json!({ "a": 1 })
    );
    assert_eq!(to_json(&CqlValue::Blob(vec![0xca, 0xfe])), json!("0xcafe"));
    assert_eq!(
        to_json(&CqlValue::List(vec![CqlValue::BigInt(5)])),
        json!([5])
    );
}

#[test]
fn read_only_allowlist() {
    assert!(check_read_only("SELECT * FROM t").is_ok());
    assert!(check_read_only("DESCRIBE TABLES").is_ok());
    assert!(check_read_only("INSERT INTO t (k) VALUES (1)").is_err());
    assert!(check_read_only("BEGIN BATCH APPLY BATCH").is_err());
    assert!(check_read_only("TRUNCATE t").is_err());
}

#[tokio::test]
async fn live_cassandra() {
    let Ok(spec) = std::env::var("ROWHOUSE_TEST_CASSANDRA") else {
        return;
    };
    let (host, port) = spec.split_once(':').unwrap();
    let mut cfg = ConnectionConfig::new("c", DbKind::Cassandra);
    cfg.host = Some(host.into());
    cfg.port = port.parse().ok();
    let c = Connection::connect_with(&cfg, None).await.unwrap();
    c.query(
        "CREATE KEYSPACE IF NOT EXISTS rh_test WITH replication = {'class': 'SimpleStrategy', 'replication_factor': 1};
         DROP TABLE IF EXISTS rh_test.users;
         CREATE TABLE rh_test.users (team text, id uuid, name text, score int, tags list<text>, joined timestamp, PRIMARY KEY (team, id));
         BEGIN BATCH
           INSERT INTO rh_test.users (team, id, name, score, tags, joined) VALUES ('a', 11111111-1111-1111-1111-111111111111, 'Ana', 9, ['x'], '2024-01-02');
           INSERT INTO rh_test.users (team, id, name, score) VALUES ('a', 22222222-2222-2222-2222-222222222222, 'Budi', 7);
           INSERT INTO rh_test.users (team, id, name, score) VALUES ('b', 33333333-3333-3333-3333-333333333333, 'Citra', 8);
         APPLY BATCH;",
        100,
    )
    .await
    .unwrap();

    let tables = c.list_tables().await.unwrap();
    assert!(tables
        .iter()
        .any(|t| t.schema == "rh_test" && t.name == "users"));
    let cols = c.describe_table("rh_test", "users").await.unwrap();
    let names: Vec<_> = cols
        .iter()
        .map(|c| (c.name.as_str(), c.primary_key))
        .collect();
    assert_eq!(&names[..2], &[("team", true), ("id", true)]);

    let out = c
        .query(
            "SELECT name, tags, joined FROM rh_test.users WHERE team = 'a'",
            10,
        )
        .await
        .unwrap();
    let r = &out.results[0];
    assert_eq!(r.rows.len(), 2);
    assert_eq!(r.rows[0][1], json!(["x"]));

    let page = c
        .table_page(&PageRequest {
            schema: "rh_test".into(),
            table: "users".into(),
            filters: vec![Filter {
                column: "score".into(),
                op: FilterOp::Ge,
                value: "8".into(),
                numeric: true,
            }],
            sort: Some(Sort {
                column: "score".into(),
                desc: true,
            }),
            limit: 10,
            offset: 0,
            count: true,
        })
        .await
        .unwrap();
    assert_eq!(page.total, Some(2));
    assert!(page.sql.contains("ALLOW FILTERING"));

    // grid edits through fromJson (uuid key, timestamp, list) and IF EXISTS
    let key = json!({ "team": "a", "id": "22222222-2222-2222-2222-222222222222" });
    let cs = ChangeSet {
        schema: "rh_test".into(),
        table: "users".into(),
        updates: vec![RowUpdate {
            key: serde_json::from_value(key.clone()).unwrap(),
            set: serde_json::from_value(
                json!({ "score": "10", "tags": "[\"y\",\"z\"]", "joined": "2024-05-06T00:00:00Z" }),
            )
            .unwrap(),
        }],
        inserts: vec![],
        deletes: vec![],
    };
    c.apply_changes(&cs).await.unwrap();
    let after = c
        .query("SELECT score, tags FROM rh_test.users WHERE team = 'a' AND id = 22222222-2222-2222-2222-222222222222", 10)
        .await
        .unwrap();
    assert_eq!(after.results[0].rows[0], vec![json!(10), json!(["y", "z"])]);
    // a missing row is reported, not upserted
    let ghost = ChangeSet {
        updates: vec![RowUpdate {
            key: serde_json::from_value(
                json!({ "team": "zz", "id": "44444444-4444-4444-4444-444444444444" }),
            )
            .unwrap(),
            set: serde_json::from_value(json!({ "name": "ghost" })).unwrap(),
        }],
        ..cs.clone()
    };
    assert!(c
        .apply_changes(&ghost)
        .await
        .unwrap_err()
        .to_string()
        .contains("row not found"));
    assert!(c
        .query_read_only("DELETE FROM rh_test.users WHERE team = 'a'", 1)
        .await
        .is_err());
    c.query("DROP KEYSPACE rh_test", 1).await.unwrap();
}
