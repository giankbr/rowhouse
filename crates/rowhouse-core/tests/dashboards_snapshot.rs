use rowhouse_core::dashboards::{Dashboard, DashboardStore, Snapshot};
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
    s.enabled = false;
    assert!(
        store.save(dash("a", Some(s))).is_ok(),
        "disabled snapshots aren't checked"
    );
}
