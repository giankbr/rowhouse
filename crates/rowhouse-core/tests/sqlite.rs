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

fn map(v: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    v.as_object().unwrap().clone()
}

#[tokio::test]
async fn apply_changes_roundtrip() {
    use rowhouse_core::{ChangeSet, RowUpdate};
    let (_d, c) = setup().await;
    let cs = ChangeSet {
        schema: "main".into(),
        table: "users".into(),
        updates: vec![RowUpdate {
            key: map(serde_json::json!({"id": 1})),
            set: map(serde_json::json!({"name": "ana o'neil", "score": null})),
        }],
        inserts: vec![map(serde_json::json!({"name": "dodi", "score": "7.25"}))],
        deletes: vec![map(serde_json::json!({"id": 2}))],
    };
    let sql = c.preview_changes(&cs).unwrap();
    assert_eq!(
        sql[0],
        r#"UPDATE "users" SET "name" = 'ana o''neil', "score" = NULL WHERE "id" = 1"#
    );
    assert_eq!(c.apply_changes(&cs).await.unwrap(), 3);
    let out = c
        .query("SELECT id, name, score FROM users ORDER BY id", 10)
        .await
        .unwrap();
    let rows = &out.results[0].rows;
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows[0],
        serde_json::json!([1, "ana o'neil", null])
            .as_array()
            .unwrap()
            .clone()
    );
    assert_eq!(rows[1][1], "dodi");
    assert_eq!(rows[1][2], 7.25);
}

#[tokio::test]
async fn apply_changes_rolls_back_on_missing_row() {
    use rowhouse_core::{ChangeSet, RowUpdate};
    let (_d, c) = setup().await;
    let cs = ChangeSet {
        schema: "main".into(),
        table: "users".into(),
        updates: vec![RowUpdate {
            key: map(serde_json::json!({"id": 1})),
            set: map(serde_json::json!({"name": "changed"})),
        }],
        inserts: vec![],
        deletes: vec![map(serde_json::json!({"id": 999}))],
    };
    let err = c.apply_changes(&cs).await.unwrap_err().to_string();
    assert!(err.contains("rolled back"), "{err}");
    let out = c
        .query("SELECT name FROM users WHERE id = 1", 10)
        .await
        .unwrap();
    assert_eq!(out.results[0].rows[0][0], "ana");
}

#[tokio::test]
async fn export_csv_json_sql_and_reimport() {
    use rowhouse_core::{ExportFormat, ImportOptions};
    let (d, c) = setup().await;
    let csv = d.path().join("u.csv");
    let n = c
        .export(
            "SELECT id, name, score FROM users ORDER BY id",
            ExportFormat::Csv,
            &csv,
            None,
        )
        .await
        .unwrap();
    assert_eq!(n, 2);
    assert_eq!(
        std::fs::read_to_string(&csv).unwrap(),
        "id,name,score\n1,ana,9.5\n2,budi,\n"
    );

    let json = d.path().join("u.json");
    c.export(
        "SELECT id, name FROM users ORDER BY id",
        ExportFormat::Json,
        &json,
        None,
    )
    .await
    .unwrap();
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&json).unwrap()).unwrap();
    assert_eq!(v[1]["name"], "budi");

    let sql = d.path().join("u.sql");
    c.export(
        "SELECT name FROM users WHERE id = 1",
        ExportFormat::Sql,
        &sql,
        Some(("main", "users")),
    )
    .await
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(&sql).unwrap(),
        "INSERT INTO \"users\" (\"name\") VALUES ('ana');\n"
    );

    // import: names with quotes, empty → NULL, 1200 rows to cross batch boundaries
    let imp = d.path().join("imp.csv");
    let mut body = String::from("name,score\n");
    body.push_str("\"o'hara, jr\",\n");
    for i in 0..1199 {
        body.push_str(&format!("user{i},{i}.5\n"));
    }
    std::fs::write(&imp, body).unwrap();
    assert_eq!(
        c.import_csv(
            "main",
            "users",
            &imp,
            &ImportOptions {
                empty_as_null: true
            }
        )
        .await
        .unwrap(),
        1200
    );
    let out = c
        .query(
            "SELECT count(*), sum(score IS NULL) FROM users; SELECT name FROM users WHERE id = 3",
            10,
        )
        .await
        .unwrap();
    assert_eq!(out.results[0].rows[0][0], 1202);
    assert_eq!(out.results[0].rows[0][1], 2);
    assert_eq!(out.results[1].rows[0][0], "o'hara, jr");

    // unknown column → error, nothing inserted
    std::fs::write(&imp, "nope\nx\n").unwrap();
    assert!(c
        .import_csv("main", "users", &imp, &ImportOptions::default())
        .await
        .is_err());
}

#[tokio::test]
async fn erd_finds_foreign_keys() {
    let (_d, c) = setup().await;
    c.query(
        "CREATE TABLE teams (id INTEGER PRIMARY KEY, name TEXT);
         CREATE TABLE members (id INTEGER PRIMARY KEY, team_id INTEGER REFERENCES teams, user_id INTEGER REFERENCES users(id));
         CREATE TABLE pairs (a INTEGER, b INTEGER, PRIMARY KEY (a, b));
         CREATE TABLE pair_notes (pa INTEGER, pb INTEGER, note TEXT, FOREIGN KEY (pa, pb) REFERENCES pairs(a, b));",
        10,
    )
    .await
    .unwrap();
    let erd = c.erd(None).await.unwrap();
    assert!(erd
        .tables
        .iter()
        .any(|t| t.name == "members" && t.columns.len() == 3));
    let fk = |t: &str, rt: &str| {
        erd.foreign_keys
            .iter()
            .find(|f| f.table == t && f.ref_table == rt)
            .cloned()
    };
    let team = fk("members", "teams").unwrap();
    assert_eq!(
        (team.columns, team.ref_columns),
        (vec!["team_id".to_string()], vec!["id".to_string()])
    );
    assert_eq!(fk("members", "users").unwrap().ref_columns, vec!["id"]);
    let pair = fk("pair_notes", "pairs").unwrap();
    assert_eq!(pair.columns, vec!["pa", "pb"]);
    assert_eq!(pair.ref_columns, vec!["a", "b"]);
}

#[tokio::test]
async fn sqlite_query_plan() {
    let (_d, c) = setup().await;
    let p = c
        .explain_plan("SELECT * FROM users WHERE name = 'x' ORDER BY score", false)
        .await
        .unwrap();
    assert!(
        p.nodes.iter().any(|n| n.label.starts_with("SCAN users")
            && n.warnings.iter().any(|w| w == "full table scan")),
        "{:#?}",
        p.nodes
    );
    assert!(p.raw.contains("TEMP B-TREE"));
    let p = c
        .explain_plan("SELECT * FROM users WHERE id = 1", false)
        .await
        .unwrap();
    assert!(p.nodes[0].label.contains("INTEGER PRIMARY KEY") && p.nodes[0].warnings.is_empty());
}

#[tokio::test]
async fn sqlite_vs_sqlite_compare() {
    let (_d1, a) = setup().await;
    let (_d2, b) = setup().await;
    a.query("ALTER TABLE users ADD COLUMN nick TEXT DEFAULT 'x'", 10)
        .await
        .unwrap();
    b.query("CREATE TABLE extra (id INTEGER)", 10)
        .await
        .unwrap();
    let d = b.compare_schema("main", &a, "main").await.unwrap();
    assert!(!d.cross_dialect);
    assert_eq!(d.changes.len(), 2, "{:#?}", d.changes);
    assert!(
        d.sql
            .contains("ALTER TABLE \"users\" ADD COLUMN \"nick\" TEXT DEFAULT 'x';"),
        "{}",
        d.sql
    );
    assert!(d.sql.contains("-- DROP TABLE \"extra\";"));
    b.query(&d.sql, 10).await.unwrap();
    assert_eq!(
        b.compare_schema("main", &a, "main")
            .await
            .unwrap()
            .changes
            .len(),
        1
    ); // only the commented drop
}

#[tokio::test]
async fn table_page_filters_sort_paginate() {
    use rowhouse_core::browse::{Filter, FilterOp, PageRequest, Sort};
    let (_d, c) = setup().await;
    let mut sql = String::from("INSERT INTO users (name, score) VALUES ");
    sql.push_str(
        &(0..95)
            .map(|i| format!("('user{i:02}', {})", i % 10))
            .collect::<Vec<_>>()
            .join(", "),
    );
    c.query(&sql, 1).await.unwrap(); // 97 rows total
    let req = |filters: Vec<Filter>, sort: Option<Sort>, offset| PageRequest {
        schema: "main".into(),
        table: "users".into(),
        filters,
        sort,
        limit: 20,
        offset,
        count: true,
    };
    let f = |column: &str, op, value: &str, numeric| Filter {
        column: column.into(),
        op,
        value: value.into(),
        numeric,
    };

    let p = c
        .table_page(&req(
            vec![],
            Some(Sort {
                column: "id".into(),
                desc: true,
            }),
            20,
        ))
        .await
        .unwrap();
    assert_eq!(p.total, Some(97));
    assert_eq!(p.result.rows.len(), 20);
    assert_eq!(p.result.rows[0][0], 77); // ids 97..1 desc, page 2 starts at 77
    assert!(
        p.sql.ends_with("ORDER BY \"id\" DESC LIMIT 20 OFFSET 20"),
        "{}",
        p.sql
    );

    let p = c
        .table_page(&req(
            vec![
                f("score", FilterOp::Ge, "8", true),
                f("name", FilterOp::Contains, "USER1", false),
            ],
            None,
            0,
        ))
        .await
        .unwrap();
    assert_eq!(p.total, Some(2)); // user18, user19 (case-insensitive)
    let p = c
        .table_page(&req(vec![f("score", FilterOp::IsNull, "", false)], None, 0))
        .await
        .unwrap();
    assert_eq!(p.total, Some(1)); // budi
    let p = c
        .table_page(&req(
            vec![f("name", FilterOp::In, "ana, budi,o'x", false)],
            None,
            0,
        ))
        .await
        .unwrap();
    assert_eq!(p.total, Some(2));
    let p = c
        .table_page(&req(
            vec![
                f("name", FilterOp::StartsWith, "user0", false),
                f("score", FilterOp::Ne, "3", true),
            ],
            None,
            0,
        ))
        .await
        .unwrap();
    assert_eq!(p.total, Some(9));
}

#[tokio::test]
async fn data_compare_and_sync() {
    let (_d1, src) = setup().await;
    let (_d2, dst) = setup().await;
    // source: change ana's score, add cici; target: has extra row 9
    src.query(
        "UPDATE users SET score = 7 WHERE id = 1; INSERT INTO users (id, name) VALUES (3, 'cici')",
        10,
    )
    .await
    .unwrap();
    dst.query("INSERT INTO users (id, name) VALUES (9, 'zed')", 10)
        .await
        .unwrap();
    let d = dst
        .compare_data("main", "users", &src, "main", "users")
        .await
        .unwrap();
    assert_eq!(d.key_columns, vec!["id"]);
    assert_eq!(
        (d.only_in_source, d.only_in_target, d.changed, d.identical),
        (1, 1, 1, 1)
    );
    assert_eq!(
        d.samples_changed[0].changes["score"],
        (serde_json::json!(9.5), serde_json::json!(7.0))
    );
    assert!(
        d.sql.contains("-- DELETE FROM \"users\" WHERE \"id\" = 9;"),
        "{}",
        d.sql
    );
    dst.query(&d.sql, 10).await.unwrap();
    let again = dst
        .compare_data("main", "users", &src, "main", "users")
        .await
        .unwrap();
    assert_eq!(
        (again.only_in_source, again.changed, again.only_in_target),
        (0, 0, 1)
    );
}

#[tokio::test]
async fn structure_edit_sqlite() {
    use rowhouse_core::{structure::ColumnOp, ColumnInfo};
    let (_d, c) = setup().await;
    let col = |name: &str, t: &str| ColumnInfo {
        name: name.into(),
        data_type: t.into(),
        nullable: true,
        default: Some("'x'".into()),
        primary_key: false,
    };
    let ops = vec![
        ColumnOp::Add {
            column: col("nick", "TEXT"),
        },
        ColumnOp::Rename {
            from: "score".into(),
            to: "points".into(),
        },
        ColumnOp::Drop {
            name: "avatar".into(),
        },
    ];
    let sql = c.structure_sql("main", "users", &ops).unwrap();
    assert_eq!(
        sql[0],
        "ALTER TABLE \"users\" ADD COLUMN \"nick\" TEXT DEFAULT 'x'"
    );
    c.apply_structure("main", "users", &ops).await.unwrap();
    let names: Vec<String> = c
        .describe_table("main", "users")
        .await
        .unwrap()
        .into_iter()
        .map(|c| c.name)
        .collect();
    assert_eq!(names, ["id", "name", "points", "nick"]);
    // SQLite can't change a column type in place → clear refusal, nothing applied
    let alter = ColumnOp::Alter {
        from: col("nick", "TEXT"),
        to: col("nick", "INTEGER"),
    };
    let err = c
        .apply_structure("main", "users", &[alter])
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("can't apply"), "{err}");
}
