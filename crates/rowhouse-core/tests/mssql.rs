//! Runs only when ROWHOUSE_TEST_MSSQL=host:port:user:password is set (CI runs it
//! against mcr.microsoft.com/mssql/server).
use rowhouse_core::{ChangeSet, Connection, ConnectionConfig, DbKind, RowUpdate};

fn obj(v: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    v.as_object().unwrap().clone()
}

#[tokio::test]
async fn sqlserver_end_to_end() {
    let Ok(spec) = std::env::var("ROWHOUSE_TEST_MSSQL") else {
        return;
    };
    let p: Vec<&str> = spec.splitn(4, ':').collect();
    let mut cfg = ConnectionConfig::new("mssql", DbKind::Mssql);
    cfg.host = Some(p[0].into());
    cfg.port = p[1].parse().ok();
    cfg.user = Some(p[2].into());
    cfg.database = Some("master".into());
    let c = Connection::connect_with(&cfg, Some(p[3])).await.unwrap();

    let out = c
        .query(
            "IF OBJECT_ID('dbo.rh_t') IS NOT NULL DROP TABLE dbo.rh_t;
             CREATE TABLE dbo.rh_t (id INT PRIMARY KEY, name NVARCHAR(50), amount DECIMAL(10,2), ok BIT, at DATETIME2);
             INSERT INTO dbo.rh_t VALUES (1, N'ünï', 12.50, 1, '2026-01-02T03:04:05'), (2, NULL, NULL, 0, NULL);
             SELECT * FROM dbo.rh_t ORDER BY id;",
            100,
        )
        .await
        .unwrap();
    let r = out.results.last().unwrap();
    assert_eq!(r.rows[0][0], 1);
    assert_eq!(r.rows[0][1], "ünï");
    assert_eq!(r.rows[0][2], "12.50");
    assert_eq!(r.rows[0][3], true);
    assert_eq!(r.rows[0][4], "2026-01-02 03:04:05");
    assert!(r.rows[1][1].is_null());

    let cols = c.describe_table("dbo", "rh_t").await.unwrap();
    assert!(cols[0].primary_key && !cols[1].primary_key);
    assert!(c
        .list_tables()
        .await
        .unwrap()
        .iter()
        .any(|t| t.name == "rh_t"));
    assert_eq!(
        c.select_table_sql("dbo", "rh_t", 5),
        "SELECT TOP 5 * FROM [dbo].[rh_t]"
    );

    // read-only: writes are rolled back even if the guard were bypassed
    assert!(c.query_read_only("DELETE FROM dbo.rh_t", 10).await.is_err());

    let cs = ChangeSet {
        schema: "dbo".into(),
        table: "rh_t".into(),
        updates: vec![RowUpdate {
            key: obj(serde_json::json!({"id": 2})),
            set: obj(serde_json::json!({"name": "two", "ok": true})),
        }],
        inserts: vec![obj(serde_json::json!({"id": 3, "name": "three"}))],
        deletes: vec![obj(serde_json::json!({"id": 1}))],
    };
    assert_eq!(c.apply_changes(&cs).await.unwrap(), 3);
    // a stale delete rolls everything back
    let bad = ChangeSet {
        deletes: vec![obj(serde_json::json!({"id": 99}))],
        ..cs.clone()
    };
    assert!(c.apply_changes(&bad).await.is_err());
    let out = c.query("SELECT COUNT(*) FROM dbo.rh_t", 10).await.unwrap();
    assert_eq!(out.results[0].rows[0][0], 2);
    c.query(
        "CREATE TABLE dbo.rh_c (id INT PRIMARY KEY, t_id INT REFERENCES dbo.rh_t(id))",
        10,
    )
    .await
    .unwrap();
    let erd = c.erd(Some("dbo")).await.unwrap();
    assert!(erd
        .foreign_keys
        .iter()
        .any(|f| f.table == "rh_c" && f.ref_table == "rh_t" && f.columns == ["t_id"]));
    c.query("DROP TABLE dbo.rh_c", 10).await.unwrap();
    c.query("DROP TABLE dbo.rh_t", 10).await.unwrap();
}
