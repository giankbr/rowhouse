use rowhouse_core::{
    ai::{AiClient, AiSettings, Provider},
    Connection, ConnectionConfig, DbKind,
};
use serde_json::{json, Value};
use wiremock::{
    matchers::{header, method, path},
    Mock, MockServer, Request, ResponseTemplate,
};

async fn db() -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("t.db");
    std::fs::File::create(&file).unwrap();
    let mut cfg = ConnectionConfig::new("t", DbKind::Sqlite);
    cfg.file = Some(file.to_string_lossy().into());
    let c = Connection::connect_with(&cfg, None).await.unwrap();
    c.query(
        "CREATE TABLE users (id INTEGER PRIMARY KEY, email TEXT NOT NULL);
         CREATE TABLE orders (id INTEGER PRIMARY KEY, user_id INTEGER, total REAL);
         INSERT INTO users (email) VALUES ('secret-person@example.com');",
        10,
    )
    .await
    .unwrap();
    (dir, c)
}

fn body(req: &Request) -> Value {
    serde_json::from_slice(&req.body).unwrap()
}

#[tokio::test]
async fn openai_generate_sends_schema_not_rows() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("authorization", "Bearer sk-test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "choices": [{"message": {"content":
              "```sql\nSELECT u.email, SUM(o.total) AS revenue\nFROM users u JOIN orders o ON o.user_id = u.id\nGROUP BY u.email;\n```\nTotal order value per user."}}]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let (_d, c) = db().await;
    let ai = AiClient::new(
        AiSettings {
            enabled: true,
            provider: Provider::Openai,
            base_url: format!("{}/v1/", server.uri()),
            model: "m-1".into(),
        },
        Some("sk-test".into()),
    )
    .unwrap();
    let a = ai.generate(&c, "revenue per user", None).await.unwrap();
    assert!(a.sql.starts_with("SELECT u.email"));
    assert_eq!(a.explanation, "Total order value per user.");

    let req = &server.received_requests().await.unwrap()[0];
    let b = body(req);
    assert_eq!(b["model"], "m-1");
    let system = b["messages"][0]["content"].as_str().unwrap();
    assert!(system.contains("SQLite"));
    assert!(system.contains("users(id INTEGER PK"), "{system}");
    assert!(
        system.contains("orders(id INTEGER PK, user_id INTEGER, total REAL)"),
        "{system}"
    );
    // rows never leave the machine
    assert!(!req.body.windows(6).any(|w| w == b"secret"));
}

#[tokio::test]
async fn anthropic_fix_uses_messages_api() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header("x-api-key", "ak"))
        .and(header("anthropic-version", "2023-06-01"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "content": [{"type": "text", "text": "Column is `email`, not `mail`.\n```sql\nSELECT email FROM users\n```"}]
        })))
        .mount(&server)
        .await;
    let (_d, c) = db().await;
    let ai = AiClient::new(
        AiSettings {
            enabled: true,
            provider: Provider::Anthropic,
            base_url: format!("{}/v1", server.uri()),
            model: "c".into(),
        },
        Some("ak".into()),
    )
    .unwrap();
    let a = ai
        .fix(&c, "SELECT mail FROM users", "no such column: mail")
        .await
        .unwrap();
    assert_eq!(a.sql, "SELECT email FROM users");
    assert_eq!(a.explanation, "Column is `email`, not `mail`.");
    let b = body(&server.received_requests().await.unwrap()[0]);
    assert!(b["system"].as_str().unwrap().contains("users("));
    assert!(b["messages"][0]["content"]
        .as_str()
        .unwrap()
        .contains("no such column: mail"));
}

#[tokio::test]
async fn provider_errors_are_readable() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(401)
                .set_body_json(json!({"error": {"message": "Invalid API key"}})),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"data": [{"id": "b"}, {"id": "a"}]})),
        )
        .mount(&server)
        .await;
    let (_d, c) = db().await;
    let ai = AiClient::new(
        AiSettings {
            enabled: true,
            provider: Provider::Openai,
            base_url: format!("{}/v1", server.uri()),
            model: "m".into(),
        },
        None,
    )
    .unwrap();
    let e = ai.explain(&c, "select 1").await.unwrap_err().to_string();
    assert!(e.contains("401") && e.contains("Invalid API key"), "{e}");
    assert_eq!(ai.list_models().await.unwrap(), vec!["a", "b"]);
    assert!(AiClient::new(AiSettings::default(), None).is_err()); // no model
}
