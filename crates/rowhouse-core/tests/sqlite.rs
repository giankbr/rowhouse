use rowhouse_core::{Connection, ConnectionConfig, ConnectionStore, DbKind};

async fn setup() -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("t.db");
    std::fs::File::create(&file).unwrap();
    let mut cfg = ConnectionConfig::new("test", DbKind::Sqlite);
    cfg.file = Some(file.to_string_lossy().into());
    let c = Connection::connect_with(&cfg, None).await.unwrap();
    c.query(
        "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL, score REAL, avatar BLOB);
         INSERT INTO users (name, score, avatar) VALUES ('ana', 9.5, x'cafe'), ('budi', NULL, NULL);
         CREATE VIEW v_users AS SELECT name FROM users;",
        100,
    )
    .await
    .unwrap();
    (dir, c)
}

#[tokio::test]
async fn query_and_types() {
    let (_d, c) = setup().await;
    let out = c
        .query("SELECT * FROM users ORDER BY id; SELECT 1 AS one", 100)
        .await
        .unwrap();
    assert_eq!(out.results.len(), 2);
    let r = &out.results[0];
    assert_eq!(
        r.columns
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>(),
        ["id", "name", "score", "avatar"]
    );
    assert_eq!(
        r.rows[0],
        serde_json::json!([1, "ana", 9.5, "0xcafe"])
            .as_array()
            .unwrap()
            .clone()
    );
    assert!(r.rows[1][2].is_null());
    assert_eq!(out.results[1].rows[0][0], 1);
}

#[tokio::test]
async fn row_limit_truncates() {
    let (_d, c) = setup().await;
    let out = c.query("SELECT * FROM users", 1).await.unwrap();
    assert_eq!(out.results[0].rows.len(), 1);
    assert!(out.results[0].truncated);
}

#[tokio::test]
async fn metadata() {
    let (_d, c) = setup().await;
    let tables = c.list_tables().await.unwrap();
    assert_eq!(tables.len(), 2);
    assert!(tables
        .iter()
        .any(|t| t.name == "v_users" && t.kind == "view"));
    let cols = c.describe_table("main", "users").await.unwrap();
    assert_eq!(cols.len(), 4);
    assert!(cols[0].primary_key);
    assert!(!cols[1].nullable);
}

#[tokio::test]
async fn read_only_blocks_writes() {
    let (_d, c) = setup().await;
    assert!(c.query_read_only("DELETE FROM users", 10).await.is_err());
    assert!(c
        .query_read_only("SELECT count(*) FROM users", 10)
        .await
        .is_ok());
    // still two rows; and the pool connection is writable again afterwards
    c.query("INSERT INTO users (name) VALUES ('cici')", 10)
        .await
        .unwrap();
    let out = c.query("SELECT count(*) FROM users", 10).await.unwrap();
    assert_eq!(out.results[0].rows[0][0], 3);
}

#[test]
fn store_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConnectionStore::at(dir.path().join("c.json"));
    assert!(store.list().unwrap().is_empty());
    let cfg = store
        .save(ConnectionConfig::new("Local PG", DbKind::Postgres))
        .unwrap();
    assert_eq!(store.get("local pg").unwrap().id, cfg.id);
    store.delete(&cfg.id).unwrap();
    assert!(store.list().unwrap().is_empty());
}
