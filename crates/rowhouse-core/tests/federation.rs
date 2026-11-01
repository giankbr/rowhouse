//! Workspaces: DuckDB with other databases and files attached.
use rowhouse_core::{
    federation::{attach_connection_sql, attach_file_sql, check_alias, explain_error, Attachment},
    Connection, ConnectionConfig, ConnectionStore, DbKind,
};
use serde_json::json;

fn att(alias: &str, path: Option<&str>, conn: Option<&str>) -> Attachment {
    Attachment {
        alias: alias.into(),
        path: path.map(Into::into),
        connection_id: conn.map(Into::into),
    }
}

#[tokio::test]
async fn joins_a_duckdb_file_a_saved_connection_and_a_csv() {
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("ROWHOUSE_CONFIG_DIR", dir.path());
    // a DuckDB file with customers
    let crm = dir.path().join("crm.duckdb");
    {
        let c = duckdb::Connection::open(&crm).unwrap();
        c.execute_batch(
            "CREATE TABLE customers (id INTEGER PRIMARY KEY, name TEXT, city TEXT);
             INSERT INTO customers VALUES (1, 'Alice', 'Bandung'), (2, 'Bob', 'Jakarta'), (3, 'Carol', 'Bandung');",
        )
        .unwrap();
    }
    // a second DuckDB file, as a saved connection
    let wh = dir.path().join("warehouse.duckdb");
    {
        let c = duckdb::Connection::open(&wh).unwrap();
        c.execute_batch("CREATE TABLE targets (city TEXT, target INTEGER); INSERT INTO targets VALUES ('Bandung', 50), ('Jakarta', 10);")
            .unwrap();
    }
    let store = ConnectionStore::open_default();
    let mut saved = ConnectionConfig::new("Warehouse", DbKind::Duckdb);
    saved.file = Some(wh.to_string_lossy().into_owned());
    let saved = store.save(saved).unwrap();
    // orders as CSV
    let csv = dir.path().join("orders.csv");
    std::fs::write(&csv, "customer_id,total\n1,10.5\n1,20\n2,7.25\n3,30\n").unwrap();

    let mut ws = ConnectionConfig::new("Workspace", DbKind::Duckdb);
    ws.attach = vec![
        att("crm", Some(crm.to_str().unwrap()), None),
        att("wh", None, Some(&saved.id)),
        att("orders", Some(csv.to_str().unwrap()), None),
    ];
    let c = Connection::connect_with(&ws, None).await.unwrap();

    let r = &c
        .query(
            "SELECT c.city, sum(o.total) AS revenue, max(t.target) AS target
             FROM orders o JOIN crm.customers c ON c.id = o.customer_id
             JOIN wh.targets t ON t.city = c.city
             GROUP BY 1 ORDER BY 1",
            100,
        )
        .await
        .unwrap()
        .results[0];
    assert_eq!(r.rows.len(), 2);
    assert_eq!(r.rows[0][0], json!("Bandung"));
    assert_eq!(r.rows[0][2], json!(50));

    // the sidebar sees attached tables as catalog.schema, files as views
    let tables = c.list_tables().await.unwrap();
    let names: Vec<_> = tables
        .iter()
        .map(|t| format!("{}.{}:{}", t.schema, t.name, t.kind))
        .collect();
    assert!(
        names.contains(&"crm.main.customers:table".to_string()),
        "{names:?}"
    );
    assert!(
        names.contains(&"wh.main.targets:table".to_string()),
        "{names:?}"
    );
    assert!(names.contains(&"main.orders:view".to_string()), "{names:?}");

    let cols = c.describe_table("crm.main", "customers").await.unwrap();
    assert_eq!(
        cols.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
        ["id", "name", "city"]
    );
    let sql = c.select_table_sql("crm.main", "customers", 5);
    assert_eq!(sql, "SELECT * FROM \"crm\".\"main\".\"customers\" LIMIT 5");
    assert_eq!(c.query(&sql, 10).await.unwrap().results[0].rows.len(), 3);

    // attachments are read-only
    assert!(c
        .query("INSERT INTO crm.customers VALUES (9, 'x', 'y')", 1)
        .await
        .is_err());
    // but the workspace itself can hold scratch tables
    c.query(
        "CREATE TABLE scratch AS SELECT * FROM orders WHERE total > 10",
        1,
    )
    .await
    .unwrap();
    assert_eq!(
        c.query("SELECT count(*) FROM scratch", 1)
            .await
            .unwrap()
            .results[0]
            .rows[0][0],
        json!(3)
    );
}

#[tokio::test]
async fn workspace_errors_are_clear() {
    let dir = tempfile::tempdir().unwrap();
    let mut ws = ConnectionConfig::new("W", DbKind::Duckdb);
    ws.attach = vec![att("bad alias", Some("/x.csv"), None)];
    let e = Connection::connect_with(&ws, None)
        .await
        .err()
        .unwrap()
        .to_string();
    assert!(e.contains("valid alias"), "{e}");
    ws.attach = vec![att(
        "x",
        Some(dir.path().join("a.xlsx").to_str().unwrap()),
        None,
    )];
    let e = Connection::connect_with(&ws, None)
        .await
        .err()
        .unwrap()
        .to_string();
    assert!(e.contains("don't know how to read"), "{e}");
    ws.attach = vec![
        att("a", Some("/x.csv"), None),
        att("A", Some("/y.csv"), None),
    ];
    let e = Connection::connect_with(&ws, None)
        .await
        .err()
        .unwrap()
        .to_string();
    assert!(e.contains("used twice"), "{e}");
}

#[test]
fn attach_statements() {
    assert!(check_alias("main").is_err());
    assert!(check_alias("prod_pg").is_ok());
    let mut pg = ConnectionConfig::new("pg", DbKind::Postgres);
    pg.user = Some("app".into());
    pg.database = Some("shop".into());
    let s = attach_connection_sql("pg", &pg, Some("p w'd"), "127.0.0.1", 6543).unwrap();
    assert_eq!(s[0], "INSTALL postgres");
    assert_eq!(
        s[2],
        "ATTACH 'host=127.0.0.1 port=6543 dbname=shop user=app password=''p w\\''d'' sslmode=prefer' AS pg (TYPE postgres, READ_ONLY)"
    );
    let mut my = ConnectionConfig::new("my", DbKind::Mariadb);
    my.user = Some("root".into());
    let s = attach_connection_sql("my", &my, None, "db", 3306).unwrap();
    assert!(
        s[2].contains("(TYPE mysql, READ_ONLY)") && s[2].contains("host=db port=3306 user=root"),
        "{}",
        s[2]
    );
    assert!(attach_connection_sql(
        "r",
        &ConnectionConfig::new("r", DbKind::Redis),
        None,
        "h",
        1
    )
    .is_err());
    assert!(attach_file_sql("s", "~/data/*.parquet").unwrap()[0].contains("read_parquet("));
    assert!(attach_file_sql("s", "/x/app.sqlite").unwrap()[2].contains("TYPE sqlite"));
    let msg = explain_error("IO Error: Failed to download extension \"postgres_scanner\" at URL …");
    assert!(
        msg.contains("postgres_scanner") && msg.contains("internet"),
        "{msg}"
    );
}

/// CI: Postgres attached through DuckDB's postgres extension (downloads it).
#[tokio::test]
async fn live_postgres_attach() {
    let Ok(spec) = std::env::var("ROWHOUSE_TEST_PG") else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("ROWHOUSE_CONFIG_DIR", dir.path());
    let mut p = spec.split(':');
    let mut pg = ConnectionConfig::new("pg", DbKind::Postgres);
    pg.host = p.next().map(Into::into);
    pg.port = p.next().and_then(|x| x.parse().ok());
    pg.user = p.next().map(Into::into);
    pg.database = Some("postgres".into());
    let pw = std::env::var("ROWHOUSE_TEST_PG_PASSWORD").ok();
    // libpq reads PGPASSWORD when the keychain has nothing
    if let Some(pw) = &pw {
        std::env::set_var("PGPASSWORD", pw);
    }
    let direct = Connection::connect_with(&pg, pw.as_deref()).await.unwrap();
    direct
        .query("DROP TABLE IF EXISTS rh_fed; CREATE TABLE rh_fed (id int, name text); INSERT INTO rh_fed VALUES (1, 'a'), (2, 'b');", 10)
        .await
        .unwrap();
    let pg = ConnectionStore::open_default().save(pg).unwrap();
    let csv = dir.path().join("extra.csv");
    std::fs::write(&csv, "id,score\n1,5\n2,9\n").unwrap();
    let mut ws = ConnectionConfig::new("W", DbKind::Duckdb);
    ws.attach = vec![
        att("pg", None, Some(&pg.id)),
        att("extra", Some(csv.to_str().unwrap()), None),
    ];
    let rows = async {
        let c = Connection::connect_with(&ws, None).await?;
        let out = c
            .query(
                "SELECT f.name, e.score FROM pg.public.rh_fed f JOIN extra e USING (id) ORDER BY 1",
                10,
            )
            .await?;
        Ok::<_, rowhouse_core::Error>(out.results[0].rows.clone())
    }
    .await;
    // clean up before asserting: other tests back this database up and restore it
    direct.query("DROP TABLE rh_fed", 1).await.unwrap();
    assert_eq!(
        rows.unwrap(),
        vec![vec![json!("a"), json!(5)], vec![json!("b"), json!(9)]]
    );
}
