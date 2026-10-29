use rowhouse_core::{ChangeSet, Connection, ConnectionConfig, DbKind, ExportFormat, RowUpdate};

fn obj(v: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    v.as_object().unwrap().clone()
}

#[tokio::test]
async fn duckdb_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = ConnectionConfig::new("duck", DbKind::Duckdb);
    cfg.file = Some(dir.path().join("w.duckdb").to_string_lossy().into());
    let c = Connection::connect(&cfg).await.unwrap();

    let out = c
        .query(
            "CREATE TABLE t (id INTEGER PRIMARY KEY, name VARCHAR, amount DECIMAL(10,2), ts TIMESTAMP, tags VARCHAR[]);
             INSERT INTO t VALUES (1, 'a''b', 12.50, TIMESTAMP '2026-01-02 03:04:05', ['x','y']), (2, NULL, NULL, NULL, NULL);
             SELECT * FROM t ORDER BY id;
             SELECT 42 AS answer WHERE false;",
            100,
        )
        .await
        .unwrap();
    assert_eq!(out.results.len(), 4);
    assert_eq!(out.results[1].rows_affected, 2);
    let r = &out.results[2];
    assert_eq!(r.columns.len(), 5);
    assert_eq!(r.rows[0][0], 1);
    assert_eq!(r.rows[0][1], "a'b");
    assert_eq!(r.rows[0][2], "12.50");
    assert_eq!(r.rows[0][3], "2026-01-02 03:04:05");
    assert_eq!(r.rows[0][4], serde_json::json!(["x", "y"]));
    assert!(r.rows[1][1].is_null());
    // empty result keeps its column names
    assert_eq!(out.results[3].columns[0].name, "answer");

    let tables = c.list_tables().await.unwrap();
    assert!(tables.iter().any(|t| t.name == "t" && t.schema == "main"));
    let cols = c.describe_table("main", "t").await.unwrap();
    assert!(cols[0].primary_key, "{cols:?}");
    assert_eq!(cols[1].data_type, "VARCHAR");

    // read-only: guard + rolled back transaction
    assert!(c.query_read_only("DELETE FROM t", 10).await.is_err());

    // grid edits
    let cs = ChangeSet {
        schema: "main".into(),
        table: "t".into(),
        updates: vec![RowUpdate {
            key: obj(serde_json::json!({"id": 2})),
            set: obj(serde_json::json!({"name": "two"})),
        }],
        inserts: vec![obj(serde_json::json!({"id": 3, "name": "three"}))],
        deletes: vec![obj(serde_json::json!({"id": 1}))],
    };
    assert_eq!(c.apply_changes(&cs).await.unwrap(), 3);
    let out = c
        .query("SELECT id, name FROM t ORDER BY id", 10)
        .await
        .unwrap();
    assert_eq!(
        out.results[0].rows,
        vec![
            vec![serde_json::json!(2), "two".into()],
            vec![serde_json::json!(3), "three".into()]
        ]
    );

    // export (non-streaming path)
    let csv = dir.path().join("t.csv");
    assert_eq!(
        c.export(
            "SELECT id, name FROM t ORDER BY id",
            ExportFormat::Csv,
            &csv,
            None
        )
        .await
        .unwrap(),
        2
    );
    assert_eq!(
        std::fs::read_to_string(&csv).unwrap(),
        "id,name\n2,two\n3,three\n"
    );

    // foreign keys
    c.query(
        "CREATE TABLE child (id INTEGER PRIMARY KEY, t_id INTEGER REFERENCES t(id))",
        10,
    )
    .await
    .unwrap();
    let erd = c.erd(None).await.unwrap();
    let fk = erd
        .foreign_keys
        .iter()
        .find(|f| f.table == "child")
        .expect("fk");
    assert_eq!(
        (
            fk.ref_table.as_str(),
            fk.columns.clone(),
            fk.ref_columns.clone()
        ),
        ("t", vec!["t_id".to_string()], vec!["id".to_string()])
    );

    let plan = c
        .explain_plan("SELECT * FROM t WHERE id > 1", false)
        .await
        .unwrap();
    assert!(
        plan.nodes.is_empty() && plan.raw.to_uppercase().contains("SCAN"),
        "{}",
        plan.raw
    );

    // select sql + quoting
    assert_eq!(
        c.select_table_sql("main", "t", 5),
        "SELECT * FROM \"main\".\"t\" LIMIT 5"
    );
}
