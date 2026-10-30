//! Runs only when ROWHOUSE_TEST_PG is set, e.g. `ROWHOUSE_TEST_PG=localhost:5432:postgres`.
use rowhouse_core::{Connection, ConnectionConfig, DbKind};

async fn conn() -> Option<Connection> {
    let spec = std::env::var("ROWHOUSE_TEST_PG").ok()?;
    let mut parts = spec.split(':');
    let mut cfg = ConnectionConfig::new("pg", DbKind::Postgres);
    cfg.host = parts.next().map(Into::into);
    cfg.port = parts.next().and_then(|p| p.parse().ok());
    cfg.user = parts.next().map(Into::into);
    cfg.database = Some("postgres".into());
    Some(
        Connection::connect_with(
            &cfg,
            std::env::var("ROWHOUSE_TEST_PG_PASSWORD").ok().as_deref(),
        )
        .await
        .unwrap(),
    )
}

#[tokio::test]
async fn pg_types_and_meta() {
    let Some(c) = conn().await else { return };
    c.query(
        "DROP TABLE IF EXISTS rowhouse_t; CREATE TABLE rowhouse_t (id serial PRIMARY KEY, ok bool, \
         amount numeric(12,2), meta jsonb, at timestamptz DEFAULT now(), tags text[], raw bytea);
         INSERT INTO rowhouse_t (ok, amount, meta, tags, raw) VALUES (true, 12.50, '{\"a\":1}', '{x,y}', '\\xcafe');",
        10,
    )
    .await
    .unwrap();
    let out = c
        .query("SELECT id, ok, amount, meta, tags, raw FROM rowhouse_t", 10)
        .await
        .unwrap();
    let row = &out.results[0].rows[0];
    assert_eq!(row[0], 1);
    assert_eq!(row[1], true);
    assert_eq!(row[2], "12.50");
    assert_eq!(row[3]["a"], 1);
    assert_eq!(row[4], "{x,y}");
    assert_eq!(row[5], "\\xcafe");

    let t = c.list_tables().await.unwrap();
    assert!(t
        .iter()
        .any(|t| t.schema == "public" && t.name == "rowhouse_t"));
    let cols = c.describe_table("public", "rowhouse_t").await.unwrap();
    assert!(cols[0].primary_key && cols[0].default.is_some());
    assert_eq!(cols[5].data_type, "text[]");
    assert_eq!(cols[2].data_type, "numeric(12,2)");

    // grid edits: string literals coerce to numeric / bool / jsonb
    use rowhouse_core::{ChangeSet, RowUpdate};
    let obj = |v: serde_json::Value| v.as_object().unwrap().clone();
    let cs = ChangeSet {
        schema: "public".into(),
        table: "rowhouse_t".into(),
        updates: vec![RowUpdate {
            key: obj(serde_json::json!({"id": 1})),
            set: obj(serde_json::json!({"ok": "false", "amount": "99.10", "meta": "{\"b\":2}"})),
        }],
        inserts: vec![obj(serde_json::json!({"ok": true}))],
        deletes: vec![],
    };
    assert_eq!(c.apply_changes(&cs).await.unwrap(), 2);
    let out = c
        .query("SELECT ok, amount, meta FROM rowhouse_t WHERE id = 1", 10)
        .await
        .unwrap();
    assert_eq!(out.results[0].rows[0][0], false);
    assert_eq!(out.results[0].rows[0][1], "99.10");
    assert_eq!(out.results[0].rows[0][2]["b"], 2);

    // read-only transaction catches what the keyword guard can't see
    assert!(c
        .query_read_only("SELECT nextval('rowhouse_t_id_seq')", 10)
        .await
        .is_err());
    assert!(c
        .query_read_only("DELETE FROM rowhouse_t", 10)
        .await
        .is_err());
    c.query(
        "DROP TABLE IF EXISTS rh_child; CREATE TABLE rh_child (id int PRIMARY KEY, parent_id int REFERENCES rowhouse_t(id))",
        10,
    )
    .await
    .unwrap();
    let erd = c.erd(Some("public")).await.unwrap();
    let fk = erd
        .foreign_keys
        .iter()
        .find(|f| f.table == "rh_child")
        .unwrap();
    assert_eq!(
        (
            fk.ref_table.as_str(),
            fk.columns[0].as_str(),
            fk.ref_columns[0].as_str()
        ),
        ("rowhouse_t", "parent_id", "id")
    );
    // query plans
    c.query(
        "INSERT INTO rh_child SELECT g, 1 FROM generate_series(1, 20000) g; ANALYZE rh_child",
        10,
    )
    .await
    .unwrap();
    let p = c
        .explain_plan("SELECT * FROM rh_child WHERE parent_id = 1", false)
        .await
        .unwrap();
    assert_eq!(p.nodes[0].label, "Seq Scan");
    assert!(p.nodes[0]
        .detail
        .as_deref()
        .unwrap()
        .starts_with("rh_child"));
    assert!(p.nodes[0].cost.unwrap() > 0.0);
    let a = c
        .explain_plan(
            "SELECT count(*) FROM rh_child c JOIN rowhouse_t t ON t.id = c.parent_id",
            true,
        )
        .await
        .unwrap();
    assert!(a.analyzed && a.execution_ms.is_some());
    assert!(a.nodes[0].actual_ms.is_some());
    fn any_warn(n: &rowhouse_core::plan::PlanNode) -> bool {
        n.warnings.iter().any(|w| w.contains("full table scan")) || n.children.iter().any(any_warn)
    }
    assert!(any_warn(&a.nodes[0]), "{:#?}", a.nodes);
    // server-side paging & filters
    {
        use rowhouse_core::browse::{Filter, FilterOp, PageRequest, Sort};
        let p = c
            .table_page(&PageRequest {
                schema: "public".into(),
                table: "rh_child".into(),
                filters: vec![
                    Filter {
                        column: "id".into(),
                        op: FilterOp::Gt,
                        value: "19990".into(),
                        numeric: true,
                    },
                    Filter {
                        column: "id".into(),
                        op: FilterOp::Contains,
                        value: "999".into(),
                        numeric: false,
                    },
                ],
                sort: Some(Sort {
                    column: "id".into(),
                    desc: false,
                }),
                limit: 3,
                offset: 1,
                count: true,
            })
            .await
            .unwrap();
        assert_eq!(p.total, Some(9)); // 19991..19999 contain "999"
        assert_eq!(
            p.result
                .rows
                .iter()
                .map(|r| r[0].clone())
                .collect::<Vec<_>>(),
            vec![19992, 19993, 19994]
        );
    }
    // ANALYZE executes → writes are refused
    assert!(c.explain_plan("DELETE FROM rh_child", true).await.is_err());
    assert_eq!(
        c.query("SELECT count(*) FROM rh_child", 10)
            .await
            .unwrap()
            .results[0]
            .rows[0][0],
        20000
    );
    c.query("DROP TABLE rh_child", 10).await.unwrap();
    c.query("DROP TABLE rowhouse_t", 10).await.unwrap();
}

#[tokio::test]
async fn pg_schema_compare_roundtrip() {
    let Some(c) = conn().await else { return };
    c.query(
        "DROP SCHEMA IF EXISTS rh_a CASCADE; DROP SCHEMA IF EXISTS rh_b CASCADE;
         CREATE SCHEMA rh_a; CREATE SCHEMA rh_b;
         CREATE TABLE rh_a.users (id int PRIMARY KEY, email varchar(120) NOT NULL, active bool DEFAULT true, bio text);
         CREATE TABLE rh_a.teams (id int PRIMARY KEY, name text NOT NULL);
         CREATE TABLE rh_b.users (id int PRIMARY KEY, email varchar(64), legacy int);
         CREATE TABLE rh_b.old_stuff (id int);",
        10,
    )
    .await
    .unwrap();
    let d = c.compare_schema("rh_b", &c, "rh_a").await.unwrap();
    let kinds: Vec<String> = d
        .changes
        .iter()
        .map(|ch| {
            serde_json::to_value(ch).unwrap()["kind"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
    for k in [
        "tableAdded",
        "tableRemoved",
        "columnAdded",
        "columnRemoved",
        "columnChanged",
    ] {
        assert!(kinds.iter().any(|x| x == k), "missing {k} in {kinds:?}");
    }
    assert!(
        d.sql.contains("-- DROP TABLE \"rh_b\".\"old_stuff\";"),
        "{}",
        d.sql
    );
    assert!(
        d.sql.contains(
            "ALTER TABLE \"rh_b\".\"users\" ALTER COLUMN \"email\" TYPE character varying(120)"
        ),
        "{}",
        d.sql
    );

    // apply it: afterwards only the (commented) drops remain as differences
    c.query(&d.sql, 10).await.unwrap();
    let again = c.compare_schema("rh_b", &c, "rh_a").await.unwrap();
    let left: Vec<String> = again
        .changes
        .iter()
        .map(|ch| {
            serde_json::to_value(ch).unwrap()["kind"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
    assert_eq!(
        left,
        vec!["columnRemoved", "tableRemoved"],
        "{:#?}",
        again.changes
    );
    assert!(again.identical.contains(&"teams".to_string()));
    c.query("DROP SCHEMA rh_a CASCADE; DROP SCHEMA rh_b CASCADE", 10)
        .await
        .unwrap();
}

#[tokio::test]
async fn pg_structure_edit() {
    use rowhouse_core::{structure::ColumnOp, ColumnInfo};
    let Some(c) = conn().await else { return };
    c.query("DROP TABLE IF EXISTS rh_s; CREATE TABLE rh_s (id int PRIMARY KEY, code varchar(10), legacy int)", 10).await.unwrap();
    let cols = c.describe_table("public", "rh_s").await.unwrap();
    let code = cols.iter().find(|x| x.name == "code").unwrap().clone();
    let ops = vec![
        ColumnOp::Add {
            column: ColumnInfo {
                name: "status".into(),
                data_type: "text".into(),
                nullable: false,
                default: Some("'new'".into()),
                primary_key: false,
            },
        },
        ColumnOp::Alter {
            from: code.clone(),
            to: ColumnInfo {
                data_type: "varchar(40)".into(),
                nullable: false,
                default: Some("'n/a'".into()),
                ..code
            },
        },
        ColumnOp::Rename {
            from: "legacy".into(),
            to: "old_value".into(),
        },
    ];
    c.apply_structure("public", "rh_s", &ops).await.unwrap();
    let cols = c.describe_table("public", "rh_s").await.unwrap();
    let get = |n: &str| cols.iter().find(|x| x.name == n).unwrap().clone();
    assert_eq!(get("code").data_type, "character varying(40)");
    assert!(!get("code").nullable);
    assert!(get("code").default.unwrap().contains("n/a"));
    assert!(!get("status").nullable);
    assert!(cols.iter().any(|x| x.name == "old_value"));
    c.apply_structure(
        "public",
        "rh_s",
        &[ColumnOp::Drop {
            name: "old_value".into(),
        }],
    )
    .await
    .unwrap();
    assert_eq!(c.describe_table("public", "rh_s").await.unwrap().len(), 3);
    // a failing statement rolls back the whole batch (Postgres DDL is transactional)
    let bad = vec![
        ColumnOp::Add {
            column: ColumnInfo {
                name: "a".into(),
                data_type: "int".into(),
                nullable: true,
                default: None,
                primary_key: false,
            },
        },
        ColumnOp::Add {
            column: ColumnInfo {
                name: "b".into(),
                data_type: "no_such_type".into(),
                nullable: true,
                default: None,
                primary_key: false,
            },
        },
    ];
    assert!(c.apply_structure("public", "rh_s", &bad).await.is_err());
    assert!(!c
        .describe_table("public", "rh_s")
        .await
        .unwrap()
        .iter()
        .any(|x| x.name == "a"));
    c.query("DROP TABLE rh_s", 10).await.unwrap();
}
