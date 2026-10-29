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

#[tokio::test]
async fn snippets_crud() {
    use rowhouse_core::Snippet;
    let dir = tempfile::tempdir().unwrap();
    let h = History::open(&dir.path().join("h.db")).await.unwrap();
    let a = h
        .save_snippet(&Snippet {
            id: 0,
            name: "Active users".into(),
            sql: "select * from users where active".into(),
            connection_id: Some("c1".into()),
            updated_at: 0,
        })
        .await
        .unwrap();
    assert!(a.id > 0);
    h.save_snippet(&Snippet {
        id: 0,
        name: "Global".into(),
        sql: "select 1".into(),
        connection_id: None,
        updated_at: 0,
    })
    .await
    .unwrap();
    h.save_snippet(&Snippet {
        id: 0,
        name: "Other".into(),
        sql: "select 2".into(),
        connection_id: Some("c2".into()),
        updated_at: 0,
    })
    .await
    .unwrap();

    let c1 = h.list_snippets(Some("c1"), None).await.unwrap();
    assert_eq!(c1.len(), 2); // own + global
    assert_eq!(
        h.list_snippets(Some("c1"), Some("active"))
            .await
            .unwrap()
            .len(),
        1
    );

    h.save_snippet(&Snippet {
        name: "Renamed".into(),
        ..a.clone()
    })
    .await
    .unwrap();
    assert!(h
        .list_snippets(Some("c1"), Some("renamed"))
        .await
        .unwrap()
        .iter()
        .any(|s| s.id == a.id));
    h.delete_snippet(a.id).await.unwrap();
    assert_eq!(h.list_snippets(Some("c1"), None).await.unwrap().len(), 1);
}
