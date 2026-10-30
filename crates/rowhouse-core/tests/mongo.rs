//! Parser / conversion tests always run; the live test runs only when
//! ROWHOUSE_TEST_MONGO=host:port is set.
use mongodb::bson::{doc, oid::ObjectId, Bson};
use rowhouse_core::{
    backend::{
        mongo::{check_read_only, coerce_like, docs_result, filter_doc, from_json, to_json},
        mongo_shell::{parse, parse_value, shell, Stmt},
    },
    browse::{Filter, FilterOp, PageRequest, Sort},
    guard, ChangeSet, Connection, ConnectionConfig, DbKind, RowUpdate,
};
use serde_json::json;

#[test]
fn parses_find_with_chain_and_shell_literals() {
    let src = r#"
        // recent paid orders
        db.orders.find(
          { status: 'paid', total: { $gte: 100.5 }, user: ObjectId("65f0c0ffee00000000000001"),
            at: { $lt: ISODate("2024-02-01") }, tag: /^vip/i, "a.b": NumberLong(5), },
          { _id: 0 }
        ).sort({ total: -1 }).skip(5).limit(10);
        show collections
        db.getCollection("weird name").countDocuments({})
        db.system.profile.find()
        db.runCommand({ ping: 1 })
    "#;
    let s = parse(src).unwrap();
    assert_eq!(s.len(), 5);
    let Stmt::Call {
        coll,
        method,
        args,
        chain,
    } = &s[0]
    else {
        panic!()
    };
    assert_eq!((coll.as_str(), method.as_str()), ("orders", "find"));
    let f = args[0].as_document().unwrap();
    assert_eq!(f.get_str("status").unwrap(), "paid");
    assert_eq!(
        f.get_document("total").unwrap().get_f64("$gte").unwrap(),
        100.5
    );
    assert!(matches!(f.get("user"), Some(Bson::ObjectId(_))));
    assert!(matches!(
        f.get_document("at").unwrap().get("$lt"),
        Some(Bson::DateTime(_))
    ));
    assert!(
        matches!(f.get("tag"), Some(Bson::RegularExpression(r)) if r.pattern == "^vip" && r.options == "i")
    );
    assert_eq!(f.get("a.b"), Some(&Bson::Int64(5)));
    assert_eq!(
        chain.iter().map(|c| c.0.as_str()).collect::<Vec<_>>(),
        ["sort", "skip", "limit"]
    );
    assert_eq!(s[1], Stmt::ShowCollections);
    assert!(
        matches!(&s[2], Stmt::Call { coll, method, .. } if coll == "weird name" && method == "countDocuments")
    );
    assert!(matches!(&s[3], Stmt::Call { coll, .. } if coll == "system.profile"));
    assert!(matches!(&s[4], Stmt::Command { admin: false, doc } if doc.contains_key("ping")));
}

#[test]
fn parse_errors_point_at_the_problem() {
    let e = parse("db.users.find({ a: })").unwrap_err();
    assert!(e.contains("line 1"), "{e}");
    assert!(parse("select * from users").is_err());
    assert!(parse("db.users.find({ a: ObjectId('nope') })")
        .unwrap_err()
        .contains("ObjectId"));
}

#[test]
fn values_and_round_trips() {
    assert_eq!(
        parse_value("[1, 2.5, 'x', null, true]")
            .unwrap()
            .to_string(),
        r#"[1, 2.5, "x", null, true]"#
    );
    assert_eq!(
        parse_value("3000000000").unwrap(),
        Bson::Int64(3_000_000_000)
    );
    let oid = ObjectId::parse_str("65f0c0ffee00000000000001").unwrap();
    let d = doc! { "_id": oid, "n": 1, "nested": { "x": [1, 2] }, "when": mongodb::bson::DateTime::from_millis(0) };
    assert_eq!(
        to_json(&Bson::Document(d.clone())),
        json!({ "_id": "65f0c0ffee00000000000001", "n": 1, "nested": { "x": [1, 2] }, "when": "1970-01-01T00:00:00Z" })
    );
    // the shell rendering parses back to the same document
    assert_eq!(
        parse_value(&shell(&Bson::Document(d.clone()))).unwrap(),
        Bson::Document(d)
    );
    // JSON text from a grid cell becomes a document again
    assert_eq!(
        from_json(&json!("{\"a\": 1}")),
        Bson::Document(doc! { "a": 1 })
    );
    assert_eq!(from_json(&json!("plain")), Bson::String("plain".into()));
}

#[test]
fn edits_keep_the_stored_type() {
    let date = Bson::DateTime(mongodb::bson::DateTime::from_millis(0));
    assert!(matches!(
        coerce_like(Some(&date), Bson::String("2024-05-01".into())),
        Bson::DateTime(_)
    ));
    assert_eq!(
        coerce_like(Some(&Bson::Int64(1)), Bson::String("42".into())),
        Bson::Int64(42)
    );
    assert_eq!(
        coerce_like(Some(&Bson::Boolean(true)), Bson::String("false".into())),
        Bson::Boolean(false)
    );
    assert_eq!(
        coerce_like(Some(&Bson::Int32(1)), Bson::String("abc".into())),
        Bson::String("abc".into())
    );
    assert_eq!(
        coerce_like(None, Bson::String("x".into())),
        Bson::String("x".into())
    );
}

#[test]
fn grid_filters_become_a_query() {
    let f = |column: &str, op, value: &str, numeric| Filter {
        column: column.into(),
        op,
        value: value.into(),
        numeric,
    };
    assert_eq!(filter_doc(&[]), doc! {});
    assert_eq!(
        filter_doc(&[f("age", FilterOp::Gt, "30", true)]),
        doc! { "age": { "$gt": 30 } }
    );
    assert_eq!(
        filter_doc(&[
            f("name", FilterOp::Contains, "a.b", false),
            f("_id", FilterOp::Eq, "65f0c0ffee00000000000001", false)
        ]),
        doc! { "$and": [
            { "name": { "$regex": "a\\.b", "$options": "i" } },
            { "_id": ObjectId::parse_str("65f0c0ffee00000000000001").unwrap() },
        ] }
    );
    assert_eq!(
        filter_doc(&[f("x", FilterOp::In, "1, 2", true)]),
        doc! { "x": { "$in": [1, 2] } }
    );
}

#[test]
fn read_only_allowlist() {
    let ok = |s: &str| parse(s).unwrap().iter().all(|x| check_read_only(x).is_ok());
    assert!(ok("db.a.find({}).limit(5)"));
    assert!(ok(
        "db.a.aggregate([{ $match: {} }, { $group: { _id: '$k', n: { $sum: 1 } } }])"
    ));
    assert!(ok(
        "db.a.countDocuments() \n show dbs \n db.runCommand({ dbStats: 1 })"
    ));
    assert!(!ok("db.a.insertOne({ x: 1 })"));
    assert!(!ok("db.a.aggregate([{ $out: 'b' }])"));
    assert!(!ok("db.a.drop()"));
    assert!(!ok("db.runCommand({ dropDatabase: 1 })"));
}

#[test]
fn destructive_mongo_statements() {
    let d = guard::destructive("db.users.drop()\ndb.logs.deleteMany({})\ndb.logs.deleteMany({ old: true })\ndb.a.updateMany({}, { $set: { x: 1 } })");
    let kinds: Vec<_> = d
        .iter()
        .map(|x| (x.kind.as_str(), x.object.as_str()))
        .collect();
    assert_eq!(
        kinds,
        [
            ("drop", "users"),
            ("delete-all", "logs"),
            ("update-all", "a")
        ]
    );
}

#[test]
fn documents_flatten_to_columns() {
    let r = docs_result(
        vec![doc! { "a": 1, "_id": 1 }, doc! { "_id": 2, "b": "x" }],
        10,
    );
    let names: Vec<_> = r.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["_id", "a", "b"]);
    assert_eq!(r.rows[1], vec![json!(2), json!(null), json!("x")]);
}

#[tokio::test]
async fn live_mongo() {
    let Ok(spec) = std::env::var("ROWHOUSE_TEST_MONGO") else {
        return;
    };
    let (host, port) = spec.split_once(':').unwrap();
    let mut cfg = ConnectionConfig::new("m", DbKind::Mongodb);
    cfg.host = Some(host.into());
    cfg.port = port.parse().ok();
    cfg.database = Some("rowhouse_test".into());
    let c = Connection::connect_with(&cfg, None).await.unwrap();
    c.ping().await.unwrap();
    let out = c
        .query(
            "db.people.drop()
             db.people.insertMany([
               { _id: 1, name: 'Ana', age: 31, joined: ISODate('2024-01-02') },
               { _id: 2, name: 'Budi', age: 25, tags: ['x'] },
               { _id: 3, name: 'Citra', age: 40 },
             ])
             db.people.find({ age: { $gte: 30 } }).sort({ age: -1 })
             db.people.aggregate([{ $group: { _id: null, avg: { $avg: '$age' } } }])",
            100,
        )
        .await
        .unwrap();
    assert_eq!(out.results[1].rows_affected, 3);
    let names: Vec<_> = out.results[2].rows.iter().map(|r| r[1].clone()).collect();
    assert_eq!(names, [json!("Citra"), json!("Ana")]);
    assert_eq!(out.results[3].rows[0][1], json!(32.0));

    let tables = c.list_tables().await.unwrap();
    assert!(tables.iter().any(|t| t.name == "people"));
    let cols = c.describe_table("rowhouse_test", "people").await.unwrap();
    assert!(cols[0].primary_key && cols[0].name == "_id");
    assert!(cols
        .iter()
        .any(|c| c.name == "joined" && c.data_type == "date"));

    let page = c
        .table_page(&PageRequest {
            schema: "rowhouse_test".into(),
            table: "people".into(),
            filters: vec![Filter {
                column: "name".into(),
                op: FilterOp::StartsWith,
                value: "b".into(),
                numeric: false,
            }],
            sort: Some(Sort {
                column: "age".into(),
                desc: false,
            }),
            limit: 10,
            offset: 0,
            count: true,
        })
        .await
        .unwrap();
    assert_eq!(page.total, Some(1));
    assert!(page.sql.starts_with("db.people.find("));

    // grid edits keyed by _id; the date column keeps its type
    let cs = ChangeSet {
        schema: "rowhouse_test".into(),
        table: "people".into(),
        updates: vec![RowUpdate {
            key: serde_json::from_value(json!({ "_id": 1 })).unwrap(),
            set: serde_json::from_value(json!({ "age": 32, "joined": "2024-03-04T00:00:00Z" }))
                .unwrap(),
        }],
        inserts: vec![serde_json::from_value(json!({ "name": "Dewi", "age": 22 })).unwrap()],
        deletes: vec![serde_json::from_value(json!({ "_id": 2 })).unwrap()],
    };
    assert_eq!(c.preview_changes(&cs).unwrap().len(), 3);
    assert_eq!(c.apply_changes(&cs).await.unwrap(), 3);
    let after = c.query("db.people.find({ _id: 1 })", 10).await.unwrap();
    let r = &after.results[0];
    let joined = r.columns.iter().position(|c| c.name == "joined").unwrap();
    assert_eq!(r.columns[joined].data_type, "date");
    assert_eq!(r.rows[0][joined], json!("2024-03-04T00:00:00Z"));
    // a missing _id aborts before anything changes
    let bad = ChangeSet {
        deletes: vec![serde_json::from_value(json!({ "_id": 99 })).unwrap()],
        ..cs.clone()
    };
    assert!(c.apply_changes(&bad).await.is_err());

    // read-only connections refuse writes
    assert!(c
        .query_read_only("db.people.deleteMany({})", 10)
        .await
        .is_err());
    assert!(c
        .query_read_only("db.people.countDocuments()", 10)
        .await
        .is_ok());
    c.query("db.people.drop()", 1).await.unwrap();
}
