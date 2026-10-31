use rowhouse_core::dashboards::{Dashboard, DashboardStore, Snapshot};
use rowhouse_core::notify::{Delivery, DeliveryKind};
use rowhouse_core::schedules::Every;

fn dash(id: &str, snap: Option<Snapshot>) -> Dashboard {
    serde_json::from_value(serde_json::json!({
        "id": id, "name": "Ops", "connectionId": "c1", "tiles": [], "refreshSecs": 0, "updatedAt": 0
    }))
    .map(|mut d: Dashboard| {
        d.snapshot = snap;
        d
    })
    .unwrap()
}

fn snap(every: Every) -> Snapshot {
    Snapshot {
        enabled: true,
        every,
        dir: "/tmp/x".into(),
        deliver: vec![],
        notify: true,
        last_run: None,
        last_error: None,
    }
}

#[test]
fn due_and_mark() {
    let dir = tempfile::tempdir().unwrap();
    let store = DashboardStore::at(dir.path().join("d.json"));
    let saved = store
        .save(dash("a", Some(snap(Every::Minutes { n: 1 }))))
        .unwrap();
    // clamped to 5 minutes
    assert_eq!(
        saved.snapshot.as_ref().unwrap().every,
        Every::Minutes { n: 5 }
    );
    let now = saved.updated_at;
    assert!(Dashboard::due(&store.list().unwrap(), now + 60).is_empty());
    assert_eq!(Dashboard::due(&store.list().unwrap(), now + 301).len(), 1);

    store.mark_snapshot("a", now + 301, None).unwrap();
    let l = store.list().unwrap();
    assert_eq!(l[0].updated_at, now, "marking a run is not an edit");
    assert!(Dashboard::due(&l, now + 400).is_empty());

    // a stale editor copy doesn't roll back last_run
    store.save(saved.clone()).unwrap();
    assert_eq!(
        store.list().unwrap()[0].snapshot.as_ref().unwrap().last_run,
        Some(now + 301)
    );
}

#[test]
fn rejects_bad_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let store = DashboardStore::at(dir.path().join("d.json"));
    let mut s = snap(Every::Daily { at: "8am".into() });
    assert!(store.save(dash("a", Some(s.clone()))).is_err());
    s.every = Every::Daily { at: "08:00".into() };
    s.dir = " ".into();
    assert!(store.save(dash("a", Some(s.clone()))).is_err());
    // no folder is fine when the PNG is sent somewhere
    s.deliver = vec![Delivery {
        id: "d1".into(),
        kind: DeliveryKind::Discord,
        channel: None,
        label: String::new(),
    }];
    assert!(store.save(dash("a", Some(s.clone()))).is_ok());
    s.deliver = vec![Delivery {
        id: "d2".into(),
        kind: DeliveryKind::Slack,
        channel: None,
        label: String::new(),
    }];
    assert!(
        store.save(dash("a", Some(s.clone()))).is_err(),
        "slack needs a channel"
    );
    s.deliver.clear();
    s.enabled = false;
    assert!(
        store.save(dash("a", Some(s))).is_ok(),
        "disabled snapshots aren't checked"
    );
}

// ---------------------------------------------------------------- delivery

use wiremock::{matchers, Mock, MockServer, Request, Respond, ResponseTemplate};

const PNG: &[u8] = b"\x89PNG\r\n\x1a\nfake";

struct HasPng(&'static str);
impl Respond for HasPng {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let ct = req.headers.get("content-type").unwrap().to_str().unwrap();
        assert!(ct.starts_with("multipart/form-data"), "{ct}");
        let body = String::from_utf8_lossy(&req.body);
        assert!(body.contains(self.0), "field {} missing", self.0);
        assert!(body.contains("filename=\"Ops-20261101-0800.png\""));
        assert!(body.contains("Content-Type: image/png"));
        assert!(body.contains("Ops · daily snapshot"));
        ResponseTemplate::new(200).set_body_json(serde_json::json!({ "id": "1" }))
    }
}

#[tokio::test]
async fn delivers_to_discord_webhook_and_slack() {
    use rowhouse_core::notify::{check_secret, deliver};
    let s = MockServer::start().await;
    Mock::given(matchers::method("POST"))
        .and(matchers::path("/api/webhooks/1/abcd"))
        .and(matchers::query_param("wait", "true"))
        .respond_with(HasPng("payload_json"))
        .expect(1)
        .mount(&s)
        .await;
    Mock::given(matchers::method("POST"))
        .and(matchers::path("/hook"))
        .respond_with(HasPng("payload"))
        .expect(1)
        .mount(&s)
        .await;
    // Slack: upload URL → bytes → share
    Mock::given(matchers::method("POST"))
        .and(matchers::path("/slack/files.getUploadURLExternal"))
        .and(matchers::header("authorization", "Bearer xoxb-test"))
        .and(matchers::body_string_contains("length=12"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "ok": true, "upload_url": format!("{}/upload/F1", s.uri()), "file_id": "F1"
        })))
        .mount(&s)
        .await;
    Mock::given(matchers::method("POST"))
        .and(matchers::path("/upload/F1"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&s)
        .await;
    Mock::given(matchers::method("POST"))
        .and(matchers::path("/slack/files.completeUploadExternal"))
        .and(matchers::body_partial_json(
            serde_json::json!({ "channel_id": "C123", "files": [{ "id": "F1" }] }),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "ok": true })))
        .expect(1)
        .mount(&s)
        .await;

    let d = |kind, channel: Option<&str>| Delivery {
        id: "x".into(),
        kind,
        channel: channel.map(str::to_string),
        label: String::new(),
    };
    let caption = "Ops · daily snapshot";
    let file = "Ops-20261101-0800.png";
    deliver(
        &d(DeliveryKind::Discord, None),
        &format!("{}/api/webhooks/1/abcd", s.uri()),
        caption,
        file,
        PNG,
    )
    .await
    .unwrap();
    deliver(
        &d(DeliveryKind::Webhook, None),
        &format!("{}/hook", s.uri()),
        caption,
        file,
        PNG,
    )
    .await
    .unwrap();
    std::env::set_var("ROWHOUSE_SLACK_API", format!("{}/slack", s.uri()));
    deliver(
        &d(DeliveryKind::Slack, Some("C123")),
        "xoxb-test",
        caption,
        file,
        PNG,
    )
    .await
    .unwrap();

    // Slack API errors surface (same server; ROWHOUSE_SLACK_API is process-wide)
    Mock::given(matchers::method("POST"))
        .and(matchers::path("/slack2/files.getUploadURLExternal"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({ "ok": false, "error": "missing_scope" })),
        )
        .mount(&s)
        .await;
    std::env::set_var("ROWHOUSE_SLACK_API", format!("{}/slack2", s.uri()));
    let e = deliver(
        &d(DeliveryKind::Slack, Some("C1")),
        "xoxb-t",
        "c",
        "f.png",
        PNG,
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(e.contains("missing_scope"), "{e}");

    // secrets are checked before they're stored
    assert!(check_secret(DeliveryKind::Slack, "https://hooks.slack.com/services/x").is_err());
    assert_eq!(
        check_secret(DeliveryKind::Slack, "xoxb-1-2-abcd").unwrap(),
        "bot token …abcd"
    );
    assert!(check_secret(DeliveryKind::Discord, "https://example.com/x").is_err());
    assert_eq!(
        check_secret(
            DeliveryKind::Discord,
            "https://discord.com/api/webhooks/1/secrettoken"
        )
        .unwrap(),
        "discord.com/…oken"
    );
}
