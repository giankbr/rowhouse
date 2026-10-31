//! Snowflake SQL API v2 against a fake server.
use std::sync::{Arc, Mutex};

use rowhouse_core::{ChangeSet, Connection, ConnectionConfig, DbKind, RowUpdate};
use serde_json::{json, Value};
use wiremock::{matchers, Mock, MockServer, Request, Respond, ResponseTemplate};

const KEY: &str = include_str!("fixtures/test-sa-key.pem");
/// `openssl pkey -pubout -outform DER | openssl dgst -sha256 -binary | base64`
const FINGERPRINT: &str = "UEXTiz5fq1QzNO22pIo5hcCtX6OVr77KxoesR5Bc0R0=";

fn b64(s: &str) -> Vec<u8> {
    rowhouse_core::backend::libsql::base64_decode(s).unwrap()
}

struct Api(Arc<Mutex<Vec<Value>>>);

impl Respond for Api {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        // key-pair JWT: right claims, valid signature
        let auth = req.headers.get("authorization").unwrap().to_str().unwrap();
        assert_eq!(
            req.headers
                .get("x-snowflake-authorization-token-type")
                .unwrap(),
            "KEYPAIR_JWT"
        );
        let jwt = auth.strip_prefix("Bearer ").unwrap();
        let parts: Vec<&str> = jwt.split('.').collect();
        let claims: Value = serde_json::from_slice(&b64(parts[1])).unwrap();
        assert_eq!(claims["sub"], "XY12345.ANALYST");
        assert_eq!(
            claims["iss"],
            format!("XY12345.ANALYST.SHA256:{FINGERPRINT}")
        );
        let der = b64(&KEY
            .lines()
            .filter(|l| !l.starts_with("-----"))
            .collect::<String>());
        let pair = ring::signature::RsaKeyPair::from_pkcs8(&der).unwrap();
        ring::signature::UnparsedPublicKey::new(
            &ring::signature::RSA_PKCS1_2048_8192_SHA256,
            ring::signature::KeyPair::public_key(&pair).as_ref(),
        )
        .verify(
            format!("{}.{}", parts[0], parts[1]).as_bytes(),
            &b64(parts[2]),
        )
        .expect("valid signature");

        let body: Value = serde_json::from_slice(&req.body).unwrap();
        assert_eq!(body["warehouse"], "COMPUTE_WH");
        assert_eq!(body["database"], "SALES");
        assert_eq!(body["schema"], "PUBLIC");
        self.0.lock().unwrap().push(body.clone());
        let sql = body["statement"].as_str().unwrap();
        if sql == "SELECT 1" {
            return ResponseTemplate::new(200).set_body_json(json!({
                "statementHandle": "h0",
                "resultSetMetaData": { "numRows": 1, "rowType": [{ "name": "1", "type": "fixed", "scale": 0, "precision": 1 }], "partitionInfo": [{}] },
                "data": [["1"]]
            }));
        }
        if sql.starts_with("EXECUTE IMMEDIATE") {
            return ResponseTemplate::new(200).set_body_json(json!({
                "statementHandle": "h-tx",
                "resultSetMetaData": { "numRows": 1, "rowType": [{ "name": "anonymous block", "type": "text" }], "partitionInfo": [{}] },
                "data": [[null]]
            }));
        }
        if sql.contains("nope") {
            return ResponseTemplate::new(422).set_body_json(json!({
                "code": "000904", "message": "SQL compilation error: invalid identifier 'NOPE'"
            }));
        }
        // a slow query: 202 first, then poll
        ResponseTemplate::new(202).set_body_json(json!({
            "statementHandle": "h1", "message": "Asynchronous execution in progress."
        }))
    }
}

fn row_type() -> Value {
    json!([
        { "name": "ID", "type": "fixed", "scale": 0, "precision": 38 },
        { "name": "TOTAL", "type": "fixed", "scale": 2, "precision": 12 },
        { "name": "PAID", "type": "boolean" },
        { "name": "CREATED", "type": "timestamp_ntz" },
        { "name": "DAY", "type": "date" },
        { "name": "META", "type": "variant" },
        { "name": "AT_TZ", "type": "timestamp_tz" }
    ])
}

struct Poll;
impl Respond for Poll {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        if req.url.query() == Some("partition=1") {
            return ResponseTemplate::new(200).set_body_json(json!({
                "data": [["3", "7.25", "true", "1730361600.000000000", "20027", "{}", "1730361600.000000000 1860"]]
            }));
        }
        ResponseTemplate::new(200).set_body_json(json!({
            "statementHandle": "h1",
            "resultSetMetaData": { "numRows": 3, "format": "jsonv2", "rowType": row_type(), "partitionInfo": [{ "rowCount": 2 }, { "rowCount": 1 }] },
            "data": [
                ["1", "10.50", "true", "1730361600.123000000", "20027", "{\"tier\":\"gold\"}", "1730361600.000000000 1860"],
                ["2", "25.00", "false", null, null, null, null]
            ]
        }))
    }
}

#[tokio::test]
async fn snowflake_sql_api() {
    let s = MockServer::start().await;
    let seen = Arc::new(Mutex::new(Vec::new()));
    Mock::given(matchers::method("POST"))
        .and(matchers::path("/api/v2/statements"))
        .respond_with(Api(seen.clone()))
        .mount(&s)
        .await;
    Mock::given(matchers::method("GET"))
        .and(matchers::path("/api/v2/statements/h1"))
        .respond_with(Poll)
        .mount(&s)
        .await;

    let dir = tempfile::tempdir().unwrap();
    let key = dir.path().join("rsa_key.p8");
    std::fs::write(&key, KEY).unwrap();
    let mut cfg = ConnectionConfig::new("sf", DbKind::Snowflake);
    std::env::set_var("ROWHOUSE_SNOWFLAKE_ENDPOINT", s.uri());
    cfg.host = Some("xy12345.ap-southeast-1".into());
    cfg.user = Some("analyst".into());
    cfg.database = Some("sales.public".into());
    cfg.warehouse = Some("compute_wh".into());
    cfg.file = Some(key.to_string_lossy().into_owned());
    assert_eq!(
        rowhouse_core::backend::snowflake::jwt_account("xy12345.ap-southeast-1"),
        "XY12345"
    );
    assert_eq!(
        rowhouse_core::backend::snowflake::jwt_account("myorg-acct.snowflakecomputing.com"),
        "MYORG-ACCT"
    );
    let c = Connection::connect_with(&cfg, None).await.unwrap();

    let r = &c.query("SELECT * FROM orders", 1000).await.unwrap().results[0];
    assert_eq!(r.columns[1].data_type, "NUMBER(12,2)");
    assert_eq!(r.rows.len(), 3, "second partition fetched");
    assert_eq!(
        r.rows[0],
        vec![
            json!(1),
            json!("10.50"),
            json!(true),
            json!("2024-10-31 08:00:00.123"),
            json!("2024-10-31"),
            json!({ "tier": "gold" }),
            json!("2024-10-31 15:00:00 +07:00")
        ]
    );
    assert_eq!(r.rows[1][3], Value::Null);
    let r = &c.query("SELECT * FROM orders", 2).await.unwrap().results[0];
    assert_eq!((r.rows.len(), r.truncated), (2, true));

    let e = c.query("SELECT nope", 1).await.unwrap_err().to_string();
    assert!(e.contains("invalid identifier"), "{e}");

    assert_eq!(c.literal("it's a \\ path"), "'it''s a \\\\ path'");
    c.apply_changes(&ChangeSet {
        schema: "PUBLIC".into(),
        table: "ORDERS".into(),
        updates: vec![RowUpdate {
            key: json!({ "ID": 1 }).as_object().unwrap().clone(),
            set: json!({ "PAID": false }).as_object().unwrap().clone(),
        }],
        inserts: vec![],
        deletes: vec![],
    })
    .await
    .unwrap();
    let script = seen.lock().unwrap().last().unwrap()["statement"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(script.starts_with("EXECUTE IMMEDIATE $$"), "{script}");
    assert!(
        script.contains("UPDATE \"PUBLIC\".\"ORDERS\" SET \"PAID\" = FALSE WHERE \"ID\" = 1;"),
        "{script}"
    );
    assert!(
        script.contains("IF (SQLROWCOUNT <> 1) THEN ROLLBACK; RAISE wrong_count; END IF;"),
        "{script}"
    );
}

#[tokio::test]
async fn snowflake_key_errors() {
    let dir = tempfile::tempdir().unwrap();
    let key = dir.path().join("enc.p8");
    std::fs::write(
        &key,
        "-----BEGIN ENCRYPTED PRIVATE KEY-----\nAAAA\n-----END ENCRYPTED PRIVATE KEY-----\n",
    )
    .unwrap();
    let mut cfg = ConnectionConfig::new("sf", DbKind::Snowflake);
    cfg.host = Some("xy12345".into());
    cfg.user = Some("me".into());
    cfg.file = Some(key.to_string_lossy().into_owned());
    let e = Connection::connect_with(&cfg, None)
        .await
        .err()
        .unwrap()
        .to_string();
    assert!(e.contains("encrypted"), "{e}");
    cfg.file = None;
    let e = Connection::connect_with(&cfg, None)
        .await
        .err()
        .unwrap()
        .to_string();
    assert!(e.contains("programmatic access token"), "{e}");
}
