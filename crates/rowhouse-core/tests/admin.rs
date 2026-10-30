//! Indexes / foreign keys DDL, server activity, backup & restore.
use rowhouse_core::{
    dump::{backup, restore, DumpContent, DumpFormat, DumpOptions},
    structure::ColumnOp,
    Connection, ConnectionConfig, DbKind,
};

async fn sqlite() -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("a.db");
    std::fs::File::create(&f).unwrap();
    let mut cfg = ConnectionConfig::new("s", DbKind::Sqlite);
    cfg.file = Some(f.to_string_lossy().into());
    (dir, Connection::connect(&cfg).await.unwrap())
}

#[tokio::test]
async fn sqlite_indexes_and_backup() {
    let (dir, c) = sqlite().await;
    c.query("CREATE TABLE t (id INTEGER PRIMARY KEY, a TEXT, b INT); INSERT INTO t (a, b) VALUES ('x', 1), ('y', 2);", 10)
        .await
        .unwrap();
    let ops = vec![ColumnOp::AddIndex {
        name: "t_ab".into(),
        columns: vec!["a".into(), "b".into()],
        unique: true,
    }];
    c.apply_structure("main", "t", &ops).await.unwrap();
    let ix = c.list_indexes("main", "t").await.unwrap();
    let t_ab = ix.iter().find(|i| i.name == "t_ab").unwrap();
    assert!(t_ab.unique && t_ab.columns == ["a", "b"]);
    c.apply_structure(
        "main",
        "t",
        &[ColumnOp::DropIndex {
            name: "t_ab".into(),
        }],
    )
    .await
    .unwrap();
    assert!(c
        .list_indexes("main", "t")
        .await
        .unwrap()
        .iter()
        .all(|i| i.name != "t_ab"));
    // SQLite can't add FKs in place → refused with an explanation
    let fk = ColumnOp::AddForeignKey {
        name: "f".into(),
        columns: vec!["b".into()],
        ref_schema: "main".into(),
        ref_table: "t".into(),
        ref_columns: vec!["id".into()],
        on_delete: None,
    };
    assert!(c
        .apply_structure("main", "t", &[fk])
        .await
        .unwrap_err()
        .to_string()
        .contains("recreate"));

    let out = dir.path().join("backup/copy.db");
    let rep = backup(&c, None, &DumpOptions::default(), &out)
        .await
        .unwrap();
    assert!(rep.bytes > 0);
    let mut cfg = ConnectionConfig::new("copy", DbKind::Sqlite);
    cfg.file = Some(out.to_string_lossy().into());
    let copy = Connection::connect(&cfg).await.unwrap();
    assert_eq!(
        copy.query("SELECT count(*) FROM t", 1)
            .await
            .unwrap()
            .results[0]
            .rows[0][0],
        serde_json::json!(2)
    );
}

#[tokio::test]
async fn postgres_admin() {
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
        "DROP SCHEMA IF EXISTS rh_admin CASCADE; CREATE SCHEMA rh_admin;
         CREATE TABLE rh_admin.teams (id int PRIMARY KEY, name text);
         CREATE TABLE rh_admin.users (id int PRIMARY KEY, team_id int, email text);
         INSERT INTO rh_admin.teams VALUES (1, 'a'); INSERT INTO rh_admin.users VALUES (1, 1, 'x@y');",
        10,
    )
    .await
    .unwrap();
    let ops = vec![
        ColumnOp::AddIndex {
            name: "users_email".into(),
            columns: vec!["email".into()],
            unique: true,
        },
        ColumnOp::AddForeignKey {
            name: "users_team_fk".into(),
            columns: vec!["team_id".into()],
            ref_schema: "rh_admin".into(),
            ref_table: "teams".into(),
            ref_columns: vec!["id".into()],
            on_delete: Some("cascade".into()),
        },
    ];
    c.apply_structure("rh_admin", "users", &ops).await.unwrap();
    let ix = c.list_indexes("rh_admin", "users").await.unwrap();
    assert!(ix.iter().any(|i| i.primary && i.columns == ["id"]));
    assert!(ix.iter().any(|i| i.name == "users_email" && i.unique));
    let fks = c.foreign_keys().await.unwrap();
    assert!(fks.iter().any(|f| f.name == "users_team_fk"));
    c.query("DELETE FROM rh_admin.teams", 1).await.unwrap();
    assert_eq!(
        c.query("SELECT count(*) FROM rh_admin.users", 1)
            .await
            .unwrap()
            .results[0]
            .rows[0][0],
        serde_json::json!(0),
        "cascade"
    );
    c.apply_structure(
        "rh_admin",
        "users",
        &[
            ColumnOp::DropForeignKey {
                name: "users_team_fk".into(),
            },
            ColumnOp::DropIndex {
                name: "users_email".into(),
            },
        ],
    )
    .await
    .unwrap();

    // activity lists our own session
    let act = c.activity().await.unwrap();
    assert!(act.iter().any(|s| s.own));

    // backup (custom + plain) and restore
    c.query("INSERT INTO rh_admin.teams VALUES (7, 'seven')", 1)
        .await
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let custom = dir.path().join("pg.dump");
    let rep = backup(
        &c,
        pw.as_deref(),
        &DumpOptions {
            format: DumpFormat::Custom,
            ..Default::default()
        },
        &custom,
    )
    .await
    .unwrap();
    assert!(rep.bytes > 0 && rep.tool == "pg_dump");
    let plain = dir.path().join("schema.sql");
    backup(
        &c,
        pw.as_deref(),
        &DumpOptions {
            content: DumpContent::SchemaOnly,
            tables: vec!["rh_admin.teams".into()],
            ..Default::default()
        },
        &plain,
    )
    .await
    .unwrap();
    let text = std::fs::read_to_string(&plain).unwrap();
    assert!(text.contains("CREATE TABLE rh_admin.teams") && !text.contains("seven"));
    c.query("DROP SCHEMA rh_admin CASCADE", 1).await.unwrap();
    restore(&c, pw.as_deref(), &custom).await.unwrap();
    assert_eq!(
        c.query("SELECT name FROM rh_admin.teams", 1)
            .await
            .unwrap()
            .results[0]
            .rows[0][0],
        serde_json::json!("seven")
    );
    c.query("DROP SCHEMA rh_admin CASCADE", 1).await.unwrap();
}

#[tokio::test]
async fn mysql_admin() {
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
    c.query(
        "DROP TABLE IF EXISTS rh_u; DROP TABLE IF EXISTS rh_t;
         CREATE TABLE rh_t (id INT PRIMARY KEY, name VARCHAR(20)) ENGINE=InnoDB;
         CREATE TABLE rh_u (id INT PRIMARY KEY, t_id INT, email VARCHAR(50)) ENGINE=InnoDB;
         INSERT INTO rh_t VALUES (1, 'one');",
        10,
    )
    .await
    .unwrap();
    let ops = vec![
        ColumnOp::AddIndex {
            name: "rh_u_email".into(),
            columns: vec!["email".into()],
            unique: false,
        },
        ColumnOp::AddForeignKey {
            name: "rh_u_fk".into(),
            columns: vec!["t_id".into()],
            ref_schema: p[4].into(),
            ref_table: "rh_t".into(),
            ref_columns: vec!["id".into()],
            on_delete: None,
        },
    ];
    c.apply_structure(p[4], "rh_u", &ops).await.unwrap();
    let ix = c.list_indexes(p[4], "rh_u").await.unwrap();
    assert!(ix
        .iter()
        .any(|i| i.name == "rh_u_email" && !i.unique && i.columns == ["email"]));
    c.apply_structure(
        p[4],
        "rh_u",
        &[ColumnOp::DropForeignKey {
            name: "rh_u_fk".into(),
        }],
    )
    .await
    .unwrap();
    assert!(c.activity().await.unwrap().iter().any(|s| s.own));

    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("my.sql");
    backup(
        &c,
        Some(p[3]),
        &DumpOptions {
            tables: vec!["rh_t".into()],
            ..Default::default()
        },
        &f,
    )
    .await
    .unwrap();
    assert!(std::fs::read_to_string(&f).unwrap().contains("'one'"));
    c.query("DROP TABLE rh_u; DROP TABLE rh_t;", 1)
        .await
        .unwrap();
    restore(&c, Some(p[3]), &f).await.unwrap();
    assert_eq!(
        c.query("SELECT name FROM rh_t", 1).await.unwrap().results[0].rows[0][0],
        serde_json::json!("one")
    );
    c.query("DROP TABLE rh_t", 1).await.unwrap();
}
