//! Turso (Hrana v2 over HTTP) and Cloudflare D1 (REST `/raw`), each against a
//! small fake server backed by a real in-memory SQLite database.
use std::sync::{Arc, Mutex};

use rowhouse_core::{
    browse::{Filter, FilterOp, PageRequest},
    guard, ChangeSet, Connection, ConnectionConfig, DbKind, RowUpdate,
};
use rusqlite::types::ValueRef;
use serde_json::{json, Value};
use wiremock::{matchers, Mock, MockServer, Request, Respond, ResponseTemplate};

type Db = Arc<Mutex<rusqlite::Connection>>;

struct Exec {
    cols: Vec<(String, Option<String>)>,
    rows: Vec<Vec<ValueOwned>>,
    changes: u64,
}

enum ValueOwned {
    Null,
    Int(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

fn exec(db: &rusqlite::Connection, sql: &str) -> Result<Exec, String> {
    let mut st = db.prepare(sql).map_err(|e| e.to_string())?;
    let cols: Vec<(String, Option<String>)> = st
        .columns()
        .iter()
        .map(|c| (c.name().to_string(), c.decl_type().map(str::to_string)))
        .collect();
    let n = cols.len();
    let mut rows = Vec::new();
    {
        let mut q = st.raw_query();
        while let Some(r) = q.next().map_err(|e| e.to_string())? {
            let mut row = Vec::new();
            for i in 0..n {
                row.push(match r.get_ref(i).map_err(|e| e.to_string())? {
                    ValueRef::Null => ValueOwned::Null,
                    ValueRef::Integer(v) => ValueOwned::Int(v),
                    ValueRef::Real(v) => ValueOwned::Real(v),
                    ValueRef::Text(t) => ValueOwned::Text(String::from_utf8_lossy(t).into()),
                    ValueRef::Blob(b) => ValueOwned::Blob(b.to_vec()),
                });
            }
            rows.push(row);
        }
    }
    let changes = if n == 0 { db.changes() } else { 0 };
    Ok(Exec {
        cols,
        rows,
        changes,
    })
}

fn b64(bytes: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

// ---------------------------------------------------------------- Hrana

struct Hrana(Db, Option<&'static str>);

fn cond_ok(c: &Value, results: &[Option<Value>], errors: &[Option<Value>]) -> bool {
    match c["type"].as_str().unwrap() {
        "ok" => results[c["step"].as_u64().unwrap() as usize].is_some(),
        "error" => errors[c["step"].as_u64().unwrap() as usize].is_some(),
        "not" => !cond_ok(&c["cond"], results, errors),
        "and" => c["conds"]
            .as_array()
            .unwrap()
            .iter()
            .all(|x| cond_ok(x, results, errors)),
        "or" => c["conds"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| cond_ok(x, results, errors)),
        other => panic!("condition {other}"),
    }
}

impl Respond for Hrana {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        if let Some(t) = self.1 {
            let auth = req
                .headers
                .get("authorization")
                .and_then(|v| v.to_str().ok());
            if auth != Some(&format!("Bearer {t}")) {
                return ResponseTemplate::new(401)
                    .set_body_json(json!({ "error": "Unauthorized" }));
            }
        }
        let body: Value = serde_json::from_slice(&req.body).unwrap();
        assert!(body["baton"].is_null());
        let db = self.0.lock().unwrap();
        let mut out = Vec::new();
        for r in body["requests"].as_array().unwrap() {
            match r["type"].as_str().unwrap() {
                "close" => out.push(json!({ "type": "ok", "response": { "type": "close" } })),
                "batch" => {
                    let steps = r["batch"]["steps"].as_array().unwrap();
                    let mut results: Vec<Option<Value>> = vec![None; steps.len()];
                    let mut errors: Vec<Option<Value>> = vec![None; steps.len()];
                    for (i, s) in steps.iter().enumerate() {
                        if let Some(c) = s.get("condition") {
                            if !cond_ok(c, &results, &errors) {
                                continue;
                            }
                        }
                        match exec(&db, s["stmt"]["sql"].as_str().unwrap()) {
                            Ok(e) => {
                                results[i] = Some(json!({
                                    "cols": e.cols.iter().map(|(n, d)| json!({ "name": n, "decltype": d })).collect::<Vec<_>>(),
                                    "rows": e.rows.iter().map(|row| row.iter().map(|v| match v {
                                        ValueOwned::Null => json!({ "type": "null" }),
                                        ValueOwned::Int(n) => json!({ "type": "integer", "value": n.to_string() }),
                                        ValueOwned::Real(f) => json!({ "type": "float", "value": f }),
                                        ValueOwned::Text(t) => json!({ "type": "text", "value": t }),
                                        ValueOwned::Blob(b) => json!({ "type": "blob", "base64": b64(b) }),
                                    }).collect::<Vec<_>>()).collect::<Vec<_>>(),
                                    "affected_row_count": e.changes,
                                    "last_insert_rowid": null,
                                }))
                            }
                            Err(m) => {
                                errors[i] = Some(json!({ "message": m, "code": "SQLITE_ERROR" }))
                            }
                        }
                    }
                    out.push(
                        json!({ "type": "ok", "response": { "type": "batch", "result": {
                        "step_results": results, "step_errors": errors,
                    } } }),
                    );
                }
                other => panic!("request type {other}"),
            }
        }
        ResponseTemplate::new(200)
            .set_body_json(json!({ "baton": null, "base_url": null, "results": out }))
    }
}

// ---------------------------------------------------------------- D1

struct D1(Db);

impl Respond for D1 {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let auth = req
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok());
        if auth != Some("Bearer cf-token") {
            return ResponseTemplate::new(403).set_body_json(json!({
                "success": false, "errors": [{ "code": 10000, "message": "Authentication error" }], "messages": [], "result": []
            }));
        }
        let body: Value = serde_json::from_slice(&req.body).unwrap();
        let db = self.0.lock().unwrap();
        // a batch is one transaction
        db.execute_batch("BEGIN").unwrap();
        let mut out = Vec::new();
        for s in guard::split_statements(body["sql"].as_str().unwrap()) {
            match exec(&db, &s) {
                Ok(e) => out.push(json!({
                    "results": {
                        "columns": e.cols.iter().map(|c| c.0.clone()).collect::<Vec<_>>(),
                        "rows": e.rows.iter().map(|row| row.iter().map(|v| match v {
                            ValueOwned::Null => Value::Null,
                            ValueOwned::Int(n) => json!(n),
                            ValueOwned::Real(f) => json!(f),
                            ValueOwned::Text(t) => json!(t),
                            ValueOwned::Blob(b) => json!(b),
                        }).collect::<Vec<_>>()).collect::<Vec<_>>(),
                    },
                    "meta": { "changes": e.changes, "duration": 0.1, "rows_read": 0, "rows_written": e.changes },
                    "success": true,
                })),
                Err(m) => {
                    db.execute_batch("ROLLBACK").unwrap();
                    return ResponseTemplate::new(400).set_body_json(json!({
                        "success": false, "errors": [{ "code": 7500, "message": format!("{m}: SQLITE_ERROR") }], "messages": [], "result": []
                    }));
                }
            }
        }
        db.execute_batch("COMMIT").unwrap();
        ResponseTemplate::new(200)
            .set_body_json(json!({ "success": true, "errors": [], "messages": [], "result": out }))
    }
}

// ---------------------------------------------------------------- shared checks

async fn exercise(c: &Connection, db: &Db) {
    c.query(
        "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL, score REAL, avatar BLOB);
         INSERT INTO users (name, score, avatar) VALUES ('Ana', 9.5, x'cafe'), ('Budi', 7, NULL), ('Citra', NULL, NULL);",
        100,
    )
    .await
    .unwrap();
    let out = c
        .query("SELECT id, name, score, avatar FROM users ORDER BY id", 100)
        .await
        .unwrap();
    let r = &out.results[0];
    assert_eq!(
        r.columns
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>(),
        ["id", "name", "score", "avatar"]
    );
    assert_eq!(r.rows[0][0], json!(1));
    assert_eq!(r.rows[0][1], json!("Ana"));
    assert_eq!(r.rows[0][2], json!(9.5));
    assert_eq!(r.rows.len(), 3);

    let tables = c.list_tables().await.unwrap();
    assert_eq!(
        tables.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
        ["users"]
    );
    let cols = c.describe_table("main", "users").await.unwrap();
    assert!(cols[0].primary_key && cols[1].name == "name" && !cols[1].nullable);

    let page = c
        .table_page(&PageRequest {
            schema: "main".into(),
            table: "users".into(),
            filters: vec![Filter {
                column: "name".into(),
                op: FilterOp::Contains,
                value: "i".into(),
                numeric: false,
            }],
            sort: None,
            limit: 10,
            offset: 0,
            count: true,
        })
        .await
        .unwrap();
    assert_eq!(page.total, Some(2));

    // grid edits, all-or-nothing
    let cs = ChangeSet {
        schema: "main".into(),
        table: "users".into(),
        updates: vec![RowUpdate {
            key: serde_json::from_value(json!({ "id": 2 })).unwrap(),
            set: serde_json::from_value(json!({ "score": 8 })).unwrap(),
        }],
        inserts: vec![serde_json::from_value(json!({ "name": "Dewi" })).unwrap()],
        deletes: vec![serde_json::from_value(json!({ "id": 3 })).unwrap()],
    };
    assert_eq!(c.apply_changes(&cs).await.unwrap(), 3);
    let count = |sql: &str| {
        db.lock()
            .unwrap()
            .query_row(sql, [], |r| r.get::<_, i64>(0))
            .unwrap()
    };
    assert_eq!(count("SELECT count(*) FROM users"), 3);
    assert_eq!(
        count("SELECT CAST(score AS INTEGER) FROM users WHERE id = 2"),
        8
    );

    // a stale key (row 99 doesn't exist) rolls back the update that came before it
    let bad = ChangeSet {
        updates: vec![
            RowUpdate {
                key: serde_json::from_value(json!({ "id": 1 })).unwrap(),
                set: serde_json::from_value(json!({ "name": "CHANGED" })).unwrap(),
            },
            RowUpdate {
                key: serde_json::from_value(json!({ "id": 99 })).unwrap(),
                set: serde_json::from_value(json!({ "name": "x" })).unwrap(),
            },
        ],
        inserts: vec![],
        deletes: vec![],
        ..cs.clone()
    };
    let e = c.apply_changes(&bad).await.unwrap_err().to_string();
    assert!(e.contains("rolled back"), "{e}");
    assert_eq!(
        count("SELECT count(*) FROM users WHERE name = 'CHANGED'"),
        0
    );

    // read-only path refuses writes
    assert!(c.query_read_only("DELETE FROM users", 10).await.is_err());
    assert_eq!(
        c.query_read_only("SELECT count(*) FROM users", 10)
            .await
            .unwrap()
            .results[0]
            .rows[0][0],
        json!(3)
    );
    // errors come back as errors
    assert!(c.query("SELECT nope FROM users", 10).await.is_err());
}

#[tokio::test]
async fn turso_over_hrana() {
    let db: Db = Arc::new(Mutex::new(rusqlite::Connection::open_in_memory().unwrap()));
    let server = MockServer::start().await;
    Mock::given(matchers::method("POST"))
        .and(matchers::path("/v2/pipeline"))
        .respond_with(Hrana(db.clone(), Some("tok")))
        .mount(&server)
        .await;
    let mut cfg = ConnectionConfig::new("t", DbKind::Turso);
    cfg.host = Some(server.uri());
    // wrong token → 401 surfaces as an error
    assert!(Connection::connect_with(&cfg, Some("bad")).await.is_err());
    let c = Connection::connect_with(&cfg, Some("tok")).await.unwrap();
    exercise(&c, &db).await;
    // blobs come back hex-encoded
    let out = c
        .query("SELECT avatar FROM users WHERE id = 1", 10)
        .await
        .unwrap();
    assert_eq!(out.results[0].rows[0][0], json!("0xcafe"));
    // read-only scripts run inside BEGIN … ROLLBACK
    let ro = c.query_read_only("SELECT 1; SELECT 2", 10).await.unwrap();
    assert_eq!(ro.results.len(), 2);
}

#[tokio::test]
async fn cloudflare_d1() {
    let db: Db = Arc::new(Mutex::new(rusqlite::Connection::open_in_memory().unwrap()));
    // D1 keeps an internal table that must not show up
    db.lock()
        .unwrap()
        .execute_batch("CREATE TABLE _cf_KV (k TEXT)")
        .unwrap();
    let server = MockServer::start().await;
    Mock::given(matchers::method("POST"))
        .and(matchers::path(
            "/client/v4/accounts/acc123/d1/database/db-uuid/raw",
        ))
        .respond_with(D1(db.clone()))
        .mount(&server)
        .await;
    let mut cfg = ConnectionConfig::new("d", DbKind::D1);
    cfg.host = Some(format!("{}/client/v4", server.uri()));
    cfg.user = Some("acc123".into());
    cfg.database = Some("db-uuid".into());
    let e = Connection::connect_with(&cfg, Some("nope"))
        .await
        .err()
        .unwrap()
        .to_string();
    assert!(e.contains("Authentication error"), "{e}");
    let c = Connection::connect_with(&cfg, Some("cf-token"))
        .await
        .unwrap();
    exercise(&c, &db).await;
}
