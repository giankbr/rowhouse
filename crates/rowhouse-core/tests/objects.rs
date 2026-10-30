//! Functions / triggers / sequences / types in the sidebar, and their definitions.
use rowhouse_core::{objects::DbObject, Connection, ConnectionConfig, DbKind};

fn find<'a>(all: &'a [DbObject], kind: &str, name: &str) -> &'a DbObject {
    all.iter()
        .find(|o| o.kind == kind && o.name == name)
        .unwrap_or_else(|| panic!("{kind} {name} missing in {all:?}"))
}

#[tokio::test]
async fn sqlite_triggers() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("a.db");
    std::fs::File::create(&f).unwrap();
    let mut cfg = ConnectionConfig::new("s", DbKind::Sqlite);
    cfg.file = Some(f.to_string_lossy().into());
    let c = Connection::connect(&cfg).await.unwrap();
    c.query(
        "CREATE TABLE t (id INTEGER PRIMARY KEY, n INT); CREATE TABLE log (id INT);
         CREATE TRIGGER t_ins AFTER INSERT ON t BEGIN INSERT INTO log VALUES (new.id); END;",
        10,
    )
    .await
    .unwrap();
    let all = c.list_objects().await.unwrap();
    let t = find(&all, "trigger", "t_ins");
    assert_eq!(t.detail.as_deref(), Some("t"));
    assert!(c
        .object_definition(t)
        .await
        .unwrap()
        .starts_with("CREATE TRIGGER t_ins"));
}

#[tokio::test]
async fn duckdb_objects() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = ConnectionConfig::new("duck", DbKind::Duckdb);
    cfg.file = Some(dir.path().join("w.duckdb").to_string_lossy().into());
    let c = Connection::connect(&cfg).await.unwrap();
    c.query(
        "CREATE SEQUENCE seq1 START 5; CREATE MACRO add2(a, b) AS a + b; CREATE TYPE mood AS ENUM ('sad', 'ok');",
        10,
    )
    .await
    .unwrap();
    let all = c.list_objects().await.unwrap();
    let m = find(&all, "macro", "add2");
    let def = c.object_definition(m).await.unwrap();
    assert!(def.contains("MACRO") && def.contains("a + b"), "{def}");
    assert!(c
        .object_definition(find(&all, "sequence", "seq1"))
        .await
        .unwrap()
        .contains("SEQUENCE"));
    let ty = c
        .object_definition(find(&all, "type", "mood"))
        .await
        .unwrap();
    assert!(ty.contains("'sad'"), "{ty}");
}

#[tokio::test]
async fn postgres_objects() {
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
        "DROP SCHEMA IF EXISTS rh_obj CASCADE; CREATE SCHEMA rh_obj;
         CREATE TABLE rh_obj.t (id int PRIMARY KEY, n int);
         CREATE FUNCTION rh_obj.twice(x int) RETURNS int LANGUAGE sql AS $$ SELECT x * 2 $$;
         CREATE FUNCTION rh_obj.touch() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN NEW.n := 1; RETURN NEW; END $$;
         CREATE TRIGGER t_touch BEFORE INSERT ON rh_obj.t FOR EACH ROW EXECUTE FUNCTION rh_obj.touch();
         CREATE SEQUENCE rh_obj.s1 START 10;
         CREATE TYPE rh_obj.mood AS ENUM ('sad', 'ok');
         CREATE MATERIALIZED VIEW rh_obj.mv AS SELECT 1 AS one;",
        10,
    )
    .await
    .unwrap();
    let all: Vec<DbObject> = c
        .list_objects()
        .await
        .unwrap()
        .into_iter()
        .filter(|o| o.schema == "rh_obj")
        .collect();
    let f = find(&all, "function", "twice");
    assert_eq!(f.detail.as_deref(), Some("x integer"));
    assert!(c
        .object_definition(f)
        .await
        .unwrap()
        .contains("CREATE OR REPLACE FUNCTION rh_obj.twice"));
    assert!(c
        .object_definition(find(&all, "trigger", "t_touch"))
        .await
        .unwrap()
        .contains("CREATE TRIGGER t_touch"));
    assert!(c
        .object_definition(find(&all, "sequence", "s1"))
        .await
        .unwrap()
        .contains("START WITH 10"));
    assert!(c
        .object_definition(find(&all, "type", "mood"))
        .await
        .unwrap()
        .contains("'sad', 'ok'"));
    assert!(c
        .object_definition(find(&all, "matview", "mv"))
        .await
        .unwrap()
        .contains("MATERIALIZED VIEW"));
    c.query("DROP SCHEMA rh_obj CASCADE", 10).await.unwrap();
}

#[tokio::test]
async fn mysql_objects() {
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
        "DROP FUNCTION IF EXISTS rh_twice; DROP TABLE IF EXISTS rh_obj_t;",
        10,
    )
    .await
    .unwrap();
    c.query("CREATE TABLE rh_obj_t (id INT PRIMARY KEY, n INT)", 10)
        .await
        .unwrap();
    c.query(
        "CREATE FUNCTION rh_twice(x INT) RETURNS INT DETERMINISTIC RETURN x * 2",
        10,
    )
    .await
    .unwrap();
    c.query(
        "CREATE TRIGGER rh_obj_trg BEFORE INSERT ON rh_obj_t FOR EACH ROW SET NEW.n = 1",
        10,
    )
    .await
    .unwrap();
    let all = c.list_objects().await.unwrap();
    assert!(c
        .object_definition(find(&all, "function", "rh_twice"))
        .await
        .unwrap()
        .contains("rh_twice"));
    let t = find(&all, "trigger", "rh_obj_trg");
    assert_eq!(t.detail.as_deref(), Some("rh_obj_t"));
    assert!(c.object_definition(t).await.unwrap().contains("TRIGGER"));
    c.query("DROP FUNCTION rh_twice; DROP TABLE rh_obj_t;", 10)
        .await
        .unwrap();
}
