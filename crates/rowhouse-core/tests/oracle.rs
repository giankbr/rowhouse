//! Oracle: needs Oracle Instant Client and a server. CI sets ROWHOUSE_TEST_ORACLE=host:port:user:password:service.
use rowhouse_core::{table_ops::TableAction, Connection, ConnectionConfig, DbKind};

#[tokio::test]
async fn missing_client_is_explained() {
    if std::env::var("ROWHOUSE_TEST_ORACLE").is_ok() {
        return; // the client is installed there
    }
    let mut cfg = ConnectionConfig::new("ora", DbKind::Oracle);
    cfg.host = Some("127.0.0.1".into());
    cfg.port = Some(1);
    cfg.client_dir = Some("/definitely/not/here".into());
    let e = Connection::connect_with(&cfg, Some("x"))
        .await
        .err()
        .expect("no server here")
        .to_string();
    eprintln!("ORACLE_ERR: {e}");
    assert!(
        e.contains("Instant Client") || e.contains("ORA-") || e.contains("DPI-"),
        "{e}"
    );
}

#[tokio::test]
async fn oracle_roundtrip() {
    let Ok(spec) = std::env::var("ROWHOUSE_TEST_ORACLE") else {
        return;
    };
    let p: Vec<&str> = spec.split(':').collect();
    let mut cfg = ConnectionConfig::new("ora", DbKind::Oracle);
    cfg.host = Some(p[0].into());
    cfg.port = p[1].parse().ok();
    cfg.user = Some(p[2].into());
    cfg.database = Some(p[4].into());
    let c = Connection::connect_with(&cfg, Some(p[3])).await.unwrap();
    let owner = p[2].to_uppercase();
    let _ = c.query("DROP TABLE rh_people PURGE", 1).await;
    c.query(
        "CREATE TABLE rh_people (id NUMBER(10) PRIMARY KEY, name VARCHAR2(40) NOT NULL, born DATE, score BINARY_DOUBLE);\n\
         INSERT INTO rh_people VALUES (1, 'Ayu', DATE '1990-05-01', 1.5);\n\
         INSERT INTO rh_people VALUES (2, 'Budi', NULL, NULL);\n\
         BEGIN\n  UPDATE rh_people SET score = 2 WHERE id = 2;\nEND;\n/\n",
        10,
    )
    .await
    .unwrap();
    let out = c
        .query(
            "SELECT id, name, born, score FROM rh_people ORDER BY id",
            10,
        )
        .await
        .unwrap();
    let r = &out.results[0];
    assert_eq!(r.rows.len(), 2);
    assert_eq!(r.rows[0][0], serde_json::json!(1));
    assert_eq!(r.rows[0][2], serde_json::json!("1990-05-01 00:00:00"));
    assert_eq!(r.rows[1][3], serde_json::json!(2.0));

    let tables = c.list_tables().await.unwrap();
    assert!(
        tables
            .iter()
            .any(|t| t.schema == owner && t.name == "RH_PEOPLE"),
        "{tables:?}"
    );
    let cols = c.describe_table(&owner, "RH_PEOPLE").await.unwrap();
    assert!(cols.iter().any(|c| c.name == "ID" && c.primary_key));
    assert!(c
        .table_ddl(&owner, "RH_PEOPLE")
        .await
        .unwrap()
        .contains("CREATE TABLE"));
    let page = c
        .table_page(&rowhouse_core::browse::PageRequest {
            schema: owner.clone(),
            table: "RH_PEOPLE".into(),
            filters: vec![],
            sort: None,
            limit: 1,
            offset: 1,
            count: true,
        })
        .await
        .unwrap();
    assert_eq!((page.result.rows.len(), page.total), (1, Some(2)));
    // read-only guard
    assert!(c.query_read_only("DELETE FROM rh_people", 1).await.is_err());
    c.table_action(&owner, "RH_PEOPLE", false, &TableAction::Drop)
        .await
        .unwrap();
}
