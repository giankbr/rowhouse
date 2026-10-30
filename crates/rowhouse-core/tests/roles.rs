//! Users & roles: create, grant, inspect, revoke, drop.
use rowhouse_core::{roles::RoleOp, Connection, ConnectionConfig, DbKind};

#[tokio::test]
async fn postgres_roles() {
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
        "DROP SCHEMA IF EXISTS rh_roles CASCADE; DROP ROLE IF EXISTS rh_reader; CREATE SCHEMA rh_roles; CREATE TABLE rh_roles.t (id int);",
        10,
    )
    .await
    .unwrap();

    let create = RoleOp::Create {
        name: "rh_reader".into(),
        host: None,
        password: Some("s3cret pass".into()),
        login: true,
        superuser: false,
        create_db: true,
    };
    let shown = c.role_sql(&create, true).unwrap();
    assert!(shown[0].contains("'••••••'") && !shown[0].contains("s3cret"));
    let applied = c.apply_role(&create).await.unwrap();
    assert!(
        !applied.join(" ").contains("s3cret"),
        "returned SQL must be masked"
    );

    let roles = c.list_roles().await.unwrap();
    let r = roles.iter().find(|r| r.name == "rh_reader").unwrap();
    assert!(r.login && !r.superuser && r.attributes.contains(&"CREATEDB".to_string()));

    c.apply_role(&RoleOp::Grant {
        id: "rh_reader".into(),
        privileges: vec!["select".into()],
        schema: "rh_roles".into(),
        table: None,
    })
    .await
    .unwrap();
    let g = c.role_grants("rh_reader").await.unwrap();
    assert!(
        g.iter()
            .any(|g| g.schema == "rh_roles" && g.object == "t" && g.privileges == ["SELECT"]),
        "{g:?}"
    );
    assert!(g.iter().any(|g| g.schema == "rh_roles"
        && g.object.is_empty()
        && g.privileges.contains(&"USAGE".to_string())));

    c.apply_role(&RoleOp::Revoke {
        id: "rh_reader".into(),
        privileges: vec!["SELECT".into()],
        schema: "rh_roles".into(),
        table: Some("t".into()),
    })
    .await
    .unwrap();
    assert!(!c
        .role_grants("rh_reader")
        .await
        .unwrap()
        .iter()
        .any(|g| g.object == "t"));

    assert!(c
        .role_sql(
            &RoleOp::Grant {
                id: "x".into(),
                privileges: vec!["SELECT; DROP".into()],
                schema: "s".into(),
                table: None
            },
            true
        )
        .is_err());

    c.query(
        "DROP SCHEMA rh_roles CASCADE; REVOKE ALL ON SCHEMA public FROM rh_reader;",
        10,
    )
    .await
    .unwrap();
    c.apply_role(&RoleOp::Drop {
        id: "rh_reader".into(),
    })
    .await
    .unwrap();
    assert!(c
        .list_roles()
        .await
        .unwrap()
        .iter()
        .all(|r| r.name != "rh_reader"));
}

#[tokio::test]
async fn mysql_roles() {
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
    c.list_roles().await.unwrap();
    let _ = c.query("DROP USER IF EXISTS 'rh_app'@'%'", 10).await;
    c.apply_role(&RoleOp::Create {
        name: "rh_app".into(),
        host: None,
        password: Some("pw-123456".into()),
        login: true,
        superuser: false,
        create_db: false,
    })
    .await
    .unwrap();
    let id = "rh_app@%";
    assert!(c.list_roles().await.unwrap().iter().any(|r| r.id == id));
    c.apply_role(&RoleOp::Grant {
        id: id.into(),
        privileges: vec!["SELECT".into(), "INSERT".into()],
        schema: p[4].into(),
        table: None,
    })
    .await
    .unwrap();
    let g = c.role_grants(id).await.unwrap();
    assert!(
        g.iter().any(|g| g.schema == p[4]
            && g.privileges.contains(&"SELECT".to_string())
            && g.privileges.contains(&"INSERT".to_string())),
        "{g:?}"
    );
    c.apply_role(&RoleOp::Drop { id: id.into() }).await.unwrap();
    assert!(!c.list_roles().await.unwrap().iter().any(|r| r.id == id));
}
