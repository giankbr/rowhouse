//! Sidebar table actions: DDL, rename, duplicate, truncate, drop, create.
use rowhouse_core::{table_ops::TableAction, ColumnInfo, Connection, ConnectionConfig, DbKind};

fn col(name: &str, ty: &str, nullable: bool, pk: bool) -> ColumnInfo {
    ColumnInfo {
        name: name.into(),
        data_type: ty.into(),
        nullable,
        default: None,
        primary_key: pk,
    }
}

async fn count(c: &Connection, sql: &str) -> i64 {
    let r = c.query(sql, 10).await.unwrap();
    let v = &r.results[0].rows[0][0];
    v.as_i64()
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        .unwrap()
}

#[tokio::test]
async fn sqlite_table_ops() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("a.db");
    std::fs::File::create(&f).unwrap();
    let mut cfg = ConnectionConfig::new("s", DbKind::Sqlite);
    cfg.file = Some(f.to_string_lossy().into());
    let c = Connection::connect(&cfg).await.unwrap();
    run(&c, "main", "INTEGER", "TEXT").await;
}

async fn run(c: &Connection, schema: &str, int: &str, text: &str) {
    let cols = vec![col("id", int, false, true), col("name", text, true, false)];
    c.create_table(schema, "rh_ops", &cols).await.unwrap();
    c.query(
        &format!(
            "CREATE INDEX rh_ops_name ON {} (name)",
            c.select_all_sql(schema, "rh_ops")
                .trim_start_matches("SELECT * FROM ")
        ),
        10,
    )
    .await
    .unwrap();
    c.query(
        &format!(
            "INSERT INTO {} VALUES (1, 'a'), (2, 'b')",
            tgt(c, schema, "rh_ops")
        ),
        10,
    )
    .await
    .unwrap();

    let ddl = c.table_ddl(schema, "rh_ops").await.unwrap();
    assert!(ddl.to_uppercase().contains("CREATE TABLE"), "{ddl}");
    assert!(ddl.contains("rh_ops_name"), "index missing: {ddl}");

    c.table_action(
        schema,
        "rh_ops",
        false,
        &TableAction::Duplicate {
            new_name: "rh_ops2".into(),
            with_data: true,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        count(
            c,
            &format!("SELECT COUNT(*) FROM {}", tgt(c, schema, "rh_ops2"))
        )
        .await,
        2
    );
    c.table_action(schema, "rh_ops2", false, &TableAction::Truncate)
        .await
        .unwrap();
    assert_eq!(
        count(
            c,
            &format!("SELECT COUNT(*) FROM {}", tgt(c, schema, "rh_ops2"))
        )
        .await,
        0
    );
    c.table_action(
        schema,
        "rh_ops2",
        false,
        &TableAction::Rename {
            new_name: "rh_ops3".into(),
        },
    )
    .await
    .unwrap();
    let names: Vec<String> = c
        .list_tables()
        .await
        .unwrap()
        .into_iter()
        .filter(|t| t.schema == schema)
        .map(|t| t.name)
        .collect();
    assert!(
        names.contains(&"rh_ops3".into()) && !names.contains(&"rh_ops2".into()),
        "{names:?}"
    );
    c.table_action(schema, "rh_ops3", false, &TableAction::Drop)
        .await
        .unwrap();
    c.table_action(schema, "rh_ops", false, &TableAction::Drop)
        .await
        .unwrap();
    let names: Vec<String> = c
        .list_tables()
        .await
        .unwrap()
        .into_iter()
        .filter(|t| t.schema == schema)
        .map(|t| t.name)
        .collect();
    assert!(!names.iter().any(|n| n.starts_with("rh_ops")), "{names:?}");
}

fn tgt(c: &Connection, schema: &str, t: &str) -> String {
    c.select_all_sql(schema, t)
        .trim_start_matches("SELECT * FROM ")
        .to_string()
}

#[tokio::test]
async fn postgres_table_ops() {
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
    c.query(
        "DROP SCHEMA IF EXISTS rh_ops CASCADE; CREATE SCHEMA rh_ops;",
        10,
    )
    .await
    .unwrap();
    run(&c, "rh_ops", "integer", "text").await;
    c.query("CREATE VIEW rh_ops.v AS SELECT 1 AS one", 10)
        .await
        .unwrap();
    let ddl = c.table_ddl("rh_ops", "v").await.unwrap();
    assert!(ddl.starts_with("CREATE OR REPLACE VIEW"), "{ddl}");
    // leave the database as we found it (other tests dump/restore all schemas)
    c.query("DROP SCHEMA rh_ops CASCADE", 10).await.unwrap();
}

#[tokio::test]
async fn mysql_table_ops() {
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
    for t in ["rh_ops", "rh_ops2", "rh_ops3"] {
        c.query(&format!("DROP TABLE IF EXISTS {t}"), 10)
            .await
            .unwrap();
    }
    run(&c, p[4], "int", "varchar(40)").await;
}

#[tokio::test]
async fn duckdb_table_ops() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = ConnectionConfig::new("duck", DbKind::Duckdb);
    cfg.file = Some(dir.path().join("w.duckdb").to_string_lossy().into());
    let c = Connection::connect(&cfg).await.unwrap();
    run(&c, "main", "INTEGER", "VARCHAR").await;
}
