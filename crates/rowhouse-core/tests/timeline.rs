//! Schema timeline: snapshots on change, entries, diffs with migration SQL.
use rowhouse_core::{compare::Change, timeline, Connection, ConnectionConfig, DbKind};

#[tokio::test]
async fn records_changes_and_diffs() {
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("ROWHOUSE_CONFIG_DIR", dir.path());
    let db = dir.path().join("t.db");
    std::fs::File::create(&db).unwrap();
    let mut cfg = ConnectionConfig::new("lite", DbKind::Sqlite);
    cfg.file = Some(db.to_string_lossy().into());
    let c = Connection::connect(&cfg).await.unwrap();
    c.query(
        "CREATE TABLE users (id INTEGER PRIMARY KEY, email TEXT NOT NULL)",
        1,
    )
    .await
    .unwrap();

    let first = c.record_schema(true).await.unwrap().unwrap();
    assert!(first.initial);
    assert_eq!((first.tables, first.columns), (1, 2));
    assert!(c.record_schema(true).await.unwrap().is_none(), "unchanged");
    assert!(
        c.record_schema(false).await.unwrap().is_none(),
        "checked recently"
    );

    c.query("ALTER TABLE users ADD COLUMN name TEXT; CREATE TABLE orders (id INTEGER PRIMARY KEY, user_id INTEGER, total REAL)", 1)
        .await
        .unwrap();
    let e = c.record_schema(true).await.unwrap().unwrap();
    assert_eq!((e.tables_added, e.columns_added, e.initial), (1, 1, false));

    let list = timeline::entries(&cfg.id);
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].at, e.at, "newest first");

    let d = c.schema_timeline_diff(first.at, None).unwrap();
    assert_eq!(d.schemas.len(), 1);
    assert_eq!(d.schemas[0].schema, "main");
    assert!(d.schemas[0]
        .changes
        .iter()
        .any(|x| matches!(x, Change::TableAdded { table, .. } if table == "orders")));
    assert!(d.schemas[0].changes.iter().any(|x| matches!(x, Change::ColumnAdded { table, column } if table == "users" && column.name == "name")));
    assert!(
        d.sql.contains("CREATE TABLE") && d.sql.contains("ALTER TABLE"),
        "{}",
        d.sql
    );

    // backwards: the migration undoes it (drops stay commented out)
    let back = c.schema_timeline_diff(e.at, Some(first.at)).unwrap();
    assert!(back.sql.contains("-- DROP TABLE"), "{}", back.sql);

    timeline::forget(&cfg.id).unwrap();
    assert!(timeline::entries(&cfg.id).is_empty());
}
