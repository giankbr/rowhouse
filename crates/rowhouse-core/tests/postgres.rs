//! Runs only when ROWHOUSE_TEST_PG is set, e.g. `ROWHOUSE_TEST_PG=localhost:5432:postgres`.
use rowhouse_core::{Connection, ConnectionConfig, DbKind};

async fn conn() -> Option<Connection> {
    let spec = std::env::var("ROWHOUSE_TEST_PG").ok()?;
    let mut parts = spec.split(':');
    let mut cfg = ConnectionConfig::new("pg", DbKind::Postgres);
    cfg.host = parts.next().map(Into::into);
    cfg.port = parts.next().and_then(|p| p.parse().ok());
    cfg.user = parts.next().map(Into::into);
    cfg.database = Some("postgres".into());
    Some(
        Connection::connect_with(
            &cfg,
            std::env::var("ROWHOUSE_TEST_PG_PASSWORD").ok().as_deref(),
        )
        .await
        .unwrap(),
    )
}

#[tokio::test]
async fn pg_types_and_meta() {
    let Some(c) = conn().await else { return };
    c.query(
        "DROP TABLE IF EXISTS rowhouse_t; CREATE TABLE rowhouse_t (id serial PRIMARY KEY, ok bool, \
         amount numeric(12,2), meta jsonb, at timestamptz DEFAULT now(), tags text[], raw bytea);
         INSERT INTO rowhouse_t (ok, amount, meta, tags, raw) VALUES (true, 12.50, '{\"a\":1}', '{x,y}', '\\xcafe');",
        10,
    )
    .await
    .unwrap();
    let out = c
        .query("SELECT id, ok, amount, meta, tags, raw FROM rowhouse_t", 10)
        .await
        .unwrap();
    let row = &out.results[0].rows[0];
    assert_eq!(row[0], 1);
    assert_eq!(row[1], true);
    assert_eq!(row[2], "12.50");
    assert_eq!(row[3]["a"], 1);
    assert_eq!(row[4], "{x,y}");
    assert_eq!(row[5], "\\xcafe");

    let t = c.list_tables().await.unwrap();
    assert!(t
        .iter()
        .any(|t| t.schema == "public" && t.name == "rowhouse_t"));
    let cols = c.describe_table("public", "rowhouse_t").await.unwrap();
    assert!(cols[0].primary_key && cols[0].default.is_some());
    assert_eq!(cols[5].data_type, "_text");

    // grid edits: string literals coerce to numeric / bool / jsonb
    use rowhouse_core::{ChangeSet, RowUpdate};
    let obj = |v: serde_json::Value| v.as_object().unwrap().clone();
    let cs = ChangeSet {
        schema: "public".into(),
        table: "rowhouse_t".into(),
        updates: vec![RowUpdate {
            key: obj(serde_json::json!({"id": 1})),
            set: obj(serde_json::json!({"ok": "false", "amount": "99.10", "meta": "{\"b\":2}"})),
        }],
        inserts: vec![obj(serde_json::json!({"ok": true}))],
        deletes: vec![],
    };
    assert_eq!(c.apply_changes(&cs).await.unwrap(), 2);
    let out = c
        .query("SELECT ok, amount, meta FROM rowhouse_t WHERE id = 1", 10)
        .await
        .unwrap();
    assert_eq!(out.results[0].rows[0][0], false);
    assert_eq!(out.results[0].rows[0][1], "99.10");
    assert_eq!(out.results[0].rows[0][2]["b"], 2);

    // read-only transaction catches what the keyword guard can't see
    assert!(c
        .query_read_only("SELECT nextval('rowhouse_t_id_seq')", 10)
        .await
        .is_err());
    assert!(c
        .query_read_only("DELETE FROM rowhouse_t", 10)
        .await
        .is_err());
    c.query("DROP TABLE rowhouse_t", 10).await.unwrap();
}
