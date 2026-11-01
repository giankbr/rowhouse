//! Masked exports and the per-connection default.
use rowhouse_core::{config::Environment, io::ExportFormat, Connection, ConnectionConfig, DbKind};

#[tokio::test]
async fn export_can_mask_personal_data() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("m.db");
    std::fs::File::create(&db).unwrap();
    let mut cfg = ConnectionConfig::new("lite", DbKind::Sqlite);
    cfg.file = Some(db.to_string_lossy().into());
    let c = Connection::connect(&cfg).await.unwrap();
    c.query("CREATE TABLE u (id INTEGER, email TEXT, phone TEXT, note TEXT); INSERT INTO u VALUES (1, 'alice@example.com', '+62 812 3456 7890', 'card 4242424242424242');", 1)
        .await
        .unwrap();
    let out = dir.path().join("u.csv");
    c.export_with("SELECT * FROM u", ExportFormat::Csv, &out, None, true)
        .await
        .unwrap();
    let text = std::fs::read_to_string(&out).unwrap();
    assert!(
        !text.contains("alice@") && text.contains("a•••@•••.com"),
        "{text}"
    );
    assert!(
        !text.contains("4242424242424242") && text.contains("7890"),
        "{text}"
    );
    c.export("SELECT * FROM u", ExportFormat::Csv, &out, None)
        .await
        .unwrap();
    assert!(std::fs::read_to_string(&out)
        .unwrap()
        .contains("alice@example.com"));

    // masking defaults to on for production
    assert!(!cfg.masks());
    cfg.env = Environment::Production;
    assert!(cfg.masks());
    cfg.mask_pii = Some(false);
    assert!(!cfg.masks());
}
