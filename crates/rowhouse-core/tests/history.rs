use rowhouse_core::{history::NewEntry, History};

#[tokio::test]
async fn record_search_clear() {
    let dir = tempfile::tempdir().unwrap();
    let h = History::open(&dir.path().join("h.db")).await.unwrap();
    for (i, src) in ["app", "mcp", "app"].iter().enumerate() {
        h.record(NewEntry {
            connection_id: if i == 2 { "other" } else { "c1" },
            sql: &format!("  SELECT {i} FROM users  "),
            source: src,
            error: if i == 1 { Some("boom") } else { None },
            rows: i as i64,
            elapsed_ms: 3,
        })
        .await
        .unwrap();
    }
    let all = h.list(Some("c1"), None, 10).await.unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].sql, "SELECT 1 FROM users"); // newest first, trimmed
    assert_eq!(all[0].source, "mcp");
    assert!(!all[0].ok && all[0].error.as_deref() == Some("boom"));
    assert_eq!(h.list(None, Some("select 2"), 10).await.unwrap().len(), 1);
    h.clear(Some("c1")).await.unwrap();
    assert!(h.list(Some("c1"), None, 10).await.unwrap().is_empty());
    assert_eq!(h.list(None, None, 10).await.unwrap().len(), 1);
}
