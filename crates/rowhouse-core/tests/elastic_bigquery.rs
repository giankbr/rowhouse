//! Elasticsearch / OpenSearch SQL and BigQuery REST, each against a fake server.
use std::sync::{Arc, Mutex};

use rowhouse_core::{
    browse::PageRequest, ChangeSet, Connection, ConnectionConfig, DbKind, RowUpdate,
};
use serde_json::{json, Value};
use wiremock::{matchers, Mock, MockServer, Request, Respond, ResponseTemplate};

fn upd() -> RowUpdate {
    RowUpdate {
        key: json!({ "id": 1 }).as_object().unwrap().clone(),
        set: json!({ "paid": false }).as_object().unwrap().clone(),
    }
}

// ---------------------------------------------------------------- Elasticsearch

struct EsSql {
    opensearch: bool,
}

impl Respond for EsSql {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let auth = req
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok());
        if auth != Some("Basic ZWxhc3RpYzpzZWNyZXQ=") {
            return ResponseTemplate::new(401).set_body_json(
                json!({ "error": { "reason": "missing authentication credentials" } }),
            );
        }
        let body: Value = serde_json::from_slice(&req.body).unwrap();
        let (cols_key, rows_key) = if self.opensearch {
            ("schema", "datarows")
        } else {
            ("columns", "rows")
        };
        if let Some(c) = body["cursor"].as_str() {
            assert_eq!(c, "page2");
            return ResponseTemplate::new(200)
                .set_body_json(json!({ rows_key: [[3, "carol", true]] }));
        }
        let q = body["query"].as_str().unwrap();
        assert!(!q.ends_with(';'), "trailing ; is stripped");
        if q.starts_with("SHOW TABLES") {
            return ResponseTemplate::new(200).set_body_json(if self.opensearch {
                json!({ "schema": [{"name":"TABLE_CAT"},{"name":"TABLE_SCHEM"},{"name":"TABLE_NAME"},{"name":"TABLE_TYPE"}],
                        "datarows": [["os", null, "orders", "BASE TABLE"], ["os", null, ".kibana", "BASE TABLE"]] })
            } else {
                json!({ "columns": [{"name":"catalog","type":"keyword"},{"name":"name","type":"keyword"},{"name":"type","type":"keyword"},{"name":"kind","type":"keyword"}],
                        "rows": [["es", "orders", "TABLE", "INDEX"], ["es", "orders-current", "VIEW", "ALIAS"], ["es", ".security", "TABLE", "INDEX"]] })
            });
        }
        if q.contains("COUNT(*)") {
            return ResponseTemplate::new(200).set_body_json(
                json!({ cols_key: [{"name":"COUNT(*)","type":"long"}], rows_key: [[3]] }),
            );
        }
        let fetch = body["fetch_size"].as_u64().unwrap();
        ResponseTemplate::new(200).set_body_json(json!({
            cols_key: [{"name":"id","type":"long"},{"name":"customer","type":"keyword"},{"name":"paid","type":"boolean"}],
            rows_key: [[1, "alice", true], [2, "bob", false]],
            "cursor": if fetch > 2 { "page2" } else { "" },
        }))
    }
}

async fn es_server(opensearch: bool) -> MockServer {
    let s = MockServer::start().await;
    let root = if opensearch {
        json!({ "version": { "distribution": "opensearch", "number": "2.17.0" } })
    } else {
        json!({ "version": { "number": "8.15.0" }, "tagline": "You Know, for Search" })
    };
    Mock::given(matchers::method("GET"))
        .and(matchers::path("/"))
        .and(matchers::header(
            "authorization",
            "Basic ZWxhc3RpYzpzZWNyZXQ=",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(root))
        .with_priority(1)
        .mount(&s)
        .await;
    Mock::given(matchers::method("GET"))
        .and(matchers::path("/"))
        .respond_with(ResponseTemplate::new(401).set_body_json(
            json!({ "error": { "reason": "unable to authenticate user [elastic]" } }),
        ))
        .mount(&s)
        .await;
    let sql_path = if opensearch {
        "/_plugins/_sql"
    } else {
        "/_sql"
    };
    Mock::given(matchers::method("POST"))
        .and(matchers::path(sql_path))
        .respond_with(EsSql { opensearch })
        .mount(&s)
        .await;
    Mock::given(matchers::method("POST"))
        .and(matchers::path(format!("{sql_path}/close")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "succeeded": true })))
        .mount(&s)
        .await;
    Mock::given(matchers::method("GET"))
        .and(matchers::path("/orders/_mapping"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "orders": { "mappings": { "properties": {
                "customer": { "type": "text", "fields": { "keyword": { "type": "keyword" } } },
                "total": { "type": "double" },
                "address": { "properties": { "city": { "type": "keyword" } } }
            } } }
        })))
        .mount(&s)
        .await;
    s
}

fn es_config(s: &MockServer, kind: DbKind) -> ConnectionConfig {
    let mut c = ConnectionConfig::new("es", kind);
    c.host = Some(s.uri());
    c.user = Some("elastic".into());
    c
}

#[tokio::test]
async fn elasticsearch_sql() {
    let s = es_server(false).await;
    assert!(
        Connection::connect_with(&es_config(&s, DbKind::Elasticsearch), Some("wrong"))
            .await
            .err()
            .unwrap()
            .to_string()
            .contains("401"),
    );
    let c = Connection::connect_with(&es_config(&s, DbKind::Elasticsearch), Some("secret"))
        .await
        .unwrap();

    let tables = c.list_tables().await.unwrap();
    let names: Vec<_> = tables
        .iter()
        .map(|t| (t.name.as_str(), t.kind.as_str()))
        .collect();
    assert_eq!(names, [("orders", "table"), ("orders-current", "view")]);

    // cursor paging: 2 rows, then 1 more
    let out = c.query("SELECT * FROM orders;", 1000).await.unwrap();
    let r = &out.results[0];
    assert_eq!(r.columns[1].name, "customer");
    assert_eq!(r.rows.len(), 3);
    assert!(!r.truncated);
    // stops early and reports truncation
    let r = &c.query("SELECT * FROM orders", 1).await.unwrap().results[0];
    assert_eq!((r.rows.len(), r.truncated), (1, true));

    let cols = c.describe_table("", "orders").await.unwrap();
    let names: Vec<_> = cols.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        names,
        ["customer", "customer.keyword", "total", "address.city"]
    );

    // writes never reach the server
    let e = c
        .query("DELETE FROM orders", 10)
        .await
        .unwrap_err()
        .to_string();
    assert!(e.contains("read-only"), "{e}");
    let e = c
        .apply_changes(&ChangeSet {
            schema: String::new(),
            table: "orders".into(),
            updates: vec![upd()],
            inserts: vec![],
            deletes: vec![],
        })
        .await;
    assert!(e.is_err());

    // grid paging without OFFSET: page 2 skips ahead client-side
    let page = c
        .table_page(&PageRequest {
            schema: String::new(),
            table: "orders".into(),
            offset: 2,
            limit: 2,
            sort: None,
            filters: vec![],
            count: true,
        })
        .await
        .unwrap();
    assert!(
        page.sql.contains("LIMIT 4") && !page.sql.contains("OFFSET"),
        "{}",
        page.sql
    );
    assert_eq!(
        page.result.rows,
        vec![vec![json!(3), json!("carol"), json!(true)]]
    );
    assert_eq!(page.total, Some(3));
}

#[tokio::test]
async fn opensearch_sql() {
    let s = es_server(true).await;
    let c = Connection::connect_with(&es_config(&s, DbKind::Opensearch), Some("secret"))
        .await
        .unwrap();
    let tables = c.list_tables().await.unwrap();
    assert_eq!(tables.len(), 1);
    assert_eq!(tables[0].name, "orders");
    let r = &c.query("SELECT * FROM orders", 1000).await.unwrap().results[0];
    assert_eq!(r.rows.len(), 3);
    assert_eq!(r.columns[2].data_type, "boolean");
}

// ---------------------------------------------------------------- BigQuery

const KEY: &str = include_str!("fixtures/test-sa-key.pem");

struct Token;
impl Respond for Token {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let body = String::from_utf8_lossy(&req.body).to_string();
        assert!(
            body.starts_with("grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Ajwt-bearer")
        );
        let jwt = body.split("assertion=").nth(1).unwrap();
        let parts: Vec<&str> = jwt.split('.').collect();
        assert_eq!(parts.len(), 3);
        // verify the RS256 signature with the key's public half
        let der = rowhouse_core::backend::libsql::base64_decode(
            &KEY.lines()
                .filter(|l| !l.starts_with("-----"))
                .collect::<String>(),
        )
        .unwrap();
        let pair = ring::signature::RsaKeyPair::from_pkcs8(&der).unwrap();
        let sig = rowhouse_core::backend::libsql::base64_decode(parts[2]).unwrap();
        ring::signature::UnparsedPublicKey::new(
            &ring::signature::RSA_PKCS1_2048_8192_SHA256,
            ring::signature::KeyPair::public_key(&pair).as_ref(),
        )
        .verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &sig)
        .expect("valid signature");
        let claims: Value = serde_json::from_slice(
            &rowhouse_core::backend::libsql::base64_decode(parts[1]).unwrap(),
        )
        .unwrap();
        assert_eq!(claims["iss"], "rowhouse@test-proj.iam.gserviceaccount.com");
        assert_eq!(claims["scope"], "https://www.googleapis.com/auth/bigquery");
        ResponseTemplate::new(200).set_body_json(
            json!({ "access_token": "tok-123", "expires_in": 3600, "token_type": "Bearer" }),
        )
    }
}

struct Queries(Arc<Mutex<Vec<String>>>);
impl Respond for Queries {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&req.body).unwrap();
        assert_eq!(body["useLegacySql"], false);
        assert_eq!(body["location"], "asia-southeast2");
        let q = body["query"].as_str().unwrap().to_string();
        self.0.lock().unwrap().push(q.clone());
        if q.starts_with("BEGIN TRANSACTION") {
            return ResponseTemplate::new(200).set_body_json(json!({
                "jobReference": { "jobId": "job-tx", "location": "asia-southeast2" },
                "jobComplete": true,
            }));
        }
        if q.contains("broken") {
            return ResponseTemplate::new(400).set_body_json(json!({
                "error": { "code": 400, "message": "Unrecognized name: broken at [1:8]" }
            }));
        }
        // not finished yet: the client polls getQueryResults
        ResponseTemplate::new(200).set_body_json(json!({
            "jobReference": { "jobId": "job1", "location": "asia-southeast2" },
            "jobComplete": false,
        }))
    }
}

fn schema() -> Value {
    json!({ "fields": [
        { "name": "id", "type": "INTEGER", "mode": "REQUIRED" },
        { "name": "created", "type": "TIMESTAMP" },
        { "name": "paid", "type": "BOOLEAN" },
        { "name": "tags", "type": "STRING", "mode": "REPEATED" },
        { "name": "addr", "type": "RECORD", "fields": [{ "name": "city", "type": "STRING" }] },
        { "name": "total", "type": "FLOAT" }
    ] })
}

fn row(id: i64) -> Value {
    json!({ "f": [
        { "v": id.to_string() },
        { "v": "1.7303616E9" },
        { "v": "true" },
        { "v": [{ "v": "vip" }, { "v": "new" }] },
        { "v": { "f": [{ "v": "Jakarta" }] } },
        { "v": "12.5" }
    ] })
}

struct Results;
impl Respond for Results {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let q = req.url.query().unwrap_or("");
        assert!(q.contains("location=asia-southeast2"), "{q}");
        if q.contains("pageToken=p2") {
            return ResponseTemplate::new(200).set_body_json(json!({
                "jobComplete": true, "schema": schema(), "rows": [row(3)], "totalRows": "3",
            }));
        }
        ResponseTemplate::new(200).set_body_json(json!({
            "jobComplete": true, "schema": schema(), "rows": [row(1), row(2)],
            "totalRows": "3", "pageToken": "p2",
        }))
    }
}

#[tokio::test]
async fn bigquery_rest() {
    let s = MockServer::start().await;
    let api = "/bigquery/v2/projects/test-proj";
    let auth = || matchers::header("authorization", "Bearer tok-123");
    Mock::given(matchers::method("POST"))
        .and(matchers::path("/token"))
        .respond_with(Token)
        .expect(1) // cached afterwards
        .mount(&s)
        .await;
    Mock::given(matchers::method("GET"))
        .and(matchers::path(format!("{api}/datasets")))
        .and(auth())
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "datasets": [{ "datasetReference": { "datasetId": "sales" } }, { "datasetReference": { "datasetId": "analytics" } }]
        })))
        .mount(&s)
        .await;
    Mock::given(matchers::method("GET"))
        .and(matchers::path(format!("{api}/datasets/sales/tables")))
        .and(auth())
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "tables": [
                { "tableReference": { "tableId": "orders" }, "type": "TABLE" },
                { "tableReference": { "tableId": "daily" }, "type": "VIEW" }
            ]
        })))
        .mount(&s)
        .await;
    Mock::given(matchers::method("GET"))
        .and(matchers::path(format!("{api}/datasets/analytics/tables")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&s)
        .await;
    Mock::given(matchers::method("GET"))
        .and(matchers::path(format!(
            "{api}/datasets/sales/tables/orders"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "schema": schema() })))
        .mount(&s)
        .await;
    let seen = Arc::new(Mutex::new(Vec::new()));
    Mock::given(matchers::method("POST"))
        .and(matchers::path(format!("{api}/queries")))
        .and(auth())
        .respond_with(Queries(seen.clone()))
        .mount(&s)
        .await;
    Mock::given(matchers::method("GET"))
        .and(matchers::path(format!("{api}/queries/job1")))
        .respond_with(Results)
        .mount(&s)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let key = dir.path().join("sa.json");
    std::fs::write(
        &key,
        json!({
            "type": "service_account",
            "project_id": "test-proj",
            "client_email": "rowhouse@test-proj.iam.gserviceaccount.com",
            "private_key": KEY,
            "token_uri": format!("{}/token", s.uri()),
        })
        .to_string(),
    )
    .unwrap();
    let mut cfg = ConnectionConfig::new("bq", DbKind::Bigquery);
    cfg.host = Some(format!("{}/bigquery/v2", s.uri()));
    cfg.user = Some("asia-southeast2".into());
    cfg.file = Some(key.to_string_lossy().into_owned());
    let c = Connection::connect_with(&cfg, None).await.unwrap();

    let tables = c.list_tables().await.unwrap();
    let t: Vec<_> = tables
        .iter()
        .map(|t| (t.schema.as_str(), t.name.as_str(), t.kind.as_str()))
        .collect();
    assert_eq!(
        t,
        [("sales", "orders", "table"), ("sales", "daily", "view")]
    );

    let cols = c.describe_table("sales", "orders").await.unwrap();
    assert_eq!(cols[0].data_type, "INT64");
    assert!(!cols[0].nullable);
    assert_eq!(cols[3].data_type, "ARRAY<STRING>");
    assert_eq!(cols[4].data_type, "STRUCT<city STRING>");

    // polls until complete, follows pageToken, converts types
    let r = &c
        .query("SELECT * FROM sales.orders", 1000)
        .await
        .unwrap()
        .results[0];
    assert_eq!(r.rows.len(), 3);
    assert_eq!(
        r.rows[0],
        vec![
            json!(1),
            json!("2024-10-31 08:00:00 UTC"),
            json!(true),
            json!(["vip", "new"]),
            json!({ "city": "Jakarta" }),
            json!(12.5)
        ]
    );
    let r = &c
        .query("SELECT * FROM sales.orders", 2)
        .await
        .unwrap()
        .results[0];
    assert_eq!((r.rows.len(), r.truncated), (2, true));

    let e = c.query("SELECT broken", 10).await.unwrap_err().to_string();
    assert!(e.contains("Unrecognized name"), "{e}");

    assert_eq!(c.ident("my`t"), "`my\\`t`");
    assert_eq!(
        c.select_table_sql("sales", "orders", 5),
        "SELECT * FROM `sales`.`orders` LIMIT 5"
    );

    // edits run as one transaction with row-count assertions
    c.apply_changes(&ChangeSet {
        schema: "sales".into(),
        table: "orders".into(),
        updates: vec![upd()],
        inserts: vec![],
        deletes: vec![],
    })
    .await
    .unwrap();
    let script = seen.lock().unwrap().last().unwrap().clone();
    assert!(script.starts_with("BEGIN TRANSACTION;"), "{script}");
    assert!(
        script.contains("UPDATE `sales`.`orders` SET `paid` = FALSE WHERE `id` = 1;"),
        "{script}"
    );
    assert!(script.contains("ASSERT @@row_count = 1"), "{script}");
    assert!(script.ends_with("COMMIT TRANSACTION;"), "{script}");
}

#[tokio::test]
async fn bigquery_needs_credentials() {
    let mut cfg = ConnectionConfig::new("bq", DbKind::Bigquery);
    cfg.database = Some("p".into());
    let e = Connection::connect_with(&cfg, None)
        .await
        .err()
        .unwrap()
        .to_string();
    assert!(e.contains("service-account key"), "{e}");
}

// ---------------------------------------------------------------- live servers (CI)
//
// ROWHOUSE_TEST_ES=http://localhost:9200 (+ ROWHOUSE_TEST_ES_USER / _PASSWORD)
// ROWHOUSE_TEST_OPENSEARCH=http://localhost:9201
// ROWHOUSE_TEST_BIGQUERY=http://localhost:9050 (goccy/bigquery-emulator, project "test")

async fn seed_index(base: &str, user: Option<&str>, pw: Option<&str>, index: &str) {
    let http = reqwest::Client::new();
    let req = |m: reqwest::Method, path: &str| {
        let r = http.request(m, format!("{base}{path}"));
        match user {
            Some(u) => r.basic_auth(u, pw),
            None => r,
        }
    };
    let _ = req(reqwest::Method::DELETE, &format!("/{index}"))
        .send()
        .await;
    let r = req(reqwest::Method::PUT, &format!("/{index}"))
        .json(&json!({ "mappings": { "properties": {
            "customer": { "type": "keyword" },
            "total": { "type": "double" },
            "paid": { "type": "boolean" },
            "address": { "properties": { "city": { "type": "keyword" } } }
        } } }))
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success(), "{}", r.text().await.unwrap());
    let mut bulk = String::new();
    for (i, (c, t)) in [
        ("alice", 10.0),
        ("bob", 25.5),
        ("carol", 7.0),
        ("dan", 40.0),
        ("erin", 3.0),
    ]
    .iter()
    .enumerate()
    {
        bulk.push_str(&format!("{{\"index\":{{\"_id\":\"{i}\"}}}}\n"));
        bulk.push_str(&format!(
            "{{\"customer\":\"{c}\",\"total\":{t},\"paid\":{},\"address\":{{\"city\":\"Bandung\"}}}}\n",
            i % 2 == 0
        ));
    }
    let r = req(
        reqwest::Method::POST,
        &format!("/{index}/_bulk?refresh=true"),
    )
    .header("Content-Type", "application/x-ndjson")
    .body(bulk)
    .send()
    .await
    .unwrap();
    assert!(r.status().is_success());
}

async fn live_search(kind: DbKind, base: String, user: Option<String>, pw: Option<String>) {
    let index = "rh_orders";
    seed_index(&base, user.as_deref(), pw.as_deref(), index).await;
    let mut cfg = ConnectionConfig::new("live", kind);
    cfg.host = Some(base);
    cfg.user = user;
    let c = Connection::connect_with(&cfg, pw.as_deref()).await.unwrap();

    assert!(c
        .list_tables()
        .await
        .unwrap()
        .iter()
        .any(|t| t.name == index));
    let cols = c.describe_table("indices", index).await.unwrap();
    assert!(cols.iter().any(|c| c.name == "address.city"), "{cols:?}");

    let r = &c
        .query(
            &format!("SELECT customer, total FROM {index} ORDER BY total DESC"),
            1000,
        )
        .await
        .unwrap()
        .results[0];
    assert_eq!(r.rows.len(), 5);
    assert_eq!(r.rows[0][0], json!("dan"));

    let r = &c
        .query(&format!("SELECT customer FROM {index}"), 2)
        .await
        .unwrap()
        .results[0];
    assert_eq!((r.rows.len(), r.truncated), (2, true));

    let page = c
        .table_page(&PageRequest {
            schema: "indices".into(),
            table: index.into(),
            offset: 4,
            limit: 2,
            sort: None,
            filters: vec![],
            count: true,
        })
        .await
        .unwrap();
    assert_eq!(page.result.rows.len(), 1);
    assert_eq!(page.total, Some(5));
    assert!(c.query(&format!("DELETE FROM {index}"), 1).await.is_err());
}

#[tokio::test]
async fn live_elasticsearch() {
    let Ok(base) = std::env::var("ROWHOUSE_TEST_ES") else {
        eprintln!("skipped: ROWHOUSE_TEST_ES not set");
        return;
    };
    live_search(
        DbKind::Elasticsearch,
        base,
        std::env::var("ROWHOUSE_TEST_ES_USER").ok(),
        std::env::var("ROWHOUSE_TEST_ES_PASSWORD").ok(),
    )
    .await;
}

#[tokio::test]
async fn live_opensearch() {
    let Ok(base) = std::env::var("ROWHOUSE_TEST_OPENSEARCH") else {
        eprintln!("skipped: ROWHOUSE_TEST_OPENSEARCH not set");
        return;
    };
    live_search(DbKind::Opensearch, base, None, None).await;
}

#[tokio::test]
async fn live_bigquery_emulator() {
    let Ok(base) = std::env::var("ROWHOUSE_TEST_BIGQUERY") else {
        eprintln!("skipped: ROWHOUSE_TEST_BIGQUERY not set");
        return;
    };
    let mut cfg = ConnectionConfig::new("bq", DbKind::Bigquery);
    cfg.host = Some(format!("{}/bigquery/v2", base.trim_end_matches('/')));
    cfg.database = Some("test".into());
    // the emulator accepts any token
    let c = Connection::connect_with(&cfg, Some("emulator"))
        .await
        .unwrap();
    c.query(
        "CREATE OR REPLACE TABLE sales.rh_orders (id INT64 NOT NULL, customer STRING, total FLOAT64, tags ARRAY<STRING>)",
        10,
    )
    .await
    .unwrap();
    c.query(
        "INSERT INTO sales.rh_orders (id, customer, total, tags) VALUES \
         (1, 'alice', 10.5, ['vip']), (2, 'bob', 25.0, []), (3, 'carol', 7.25, ['new', 'vip'])",
        10,
    )
    .await
    .unwrap();
    let tables = c.list_tables().await.unwrap();
    assert!(
        tables
            .iter()
            .any(|t| t.schema == "sales" && t.name == "rh_orders"),
        "{tables:?}"
    );
    let cols = c.describe_table("sales", "rh_orders").await.unwrap();
    assert_eq!(cols[0].data_type, "INT64");
    assert_eq!(cols[3].data_type, "ARRAY<STRING>");
    let r = &c
        .query(
            "SELECT id, customer, total, tags FROM sales.rh_orders ORDER BY id",
            100,
        )
        .await
        .unwrap()
        .results[0];
    assert_eq!(r.rows.len(), 3);
    assert_eq!(
        r.rows[2],
        vec![json!(3), json!("carol"), json!(7.25), json!(["new", "vip"])]
    );
    let r = &c
        .query("SELECT id FROM sales.rh_orders", 2)
        .await
        .unwrap()
        .results[0];
    assert_eq!((r.rows.len(), r.truncated), (2, true));
}
