use chrono::{Datelike, Local, TimeZone, Timelike};
use rowhouse_core::{
    io::ExportFormat,
    schedules::{run_on, Every, ExportTarget, Notify, Run, Schedule, ScheduleStore},
    Connection, ConnectionConfig, DbKind,
};

fn sched(every: Every) -> Schedule {
    Schedule {
        id: String::new(),
        name: "Daily revenue".into(),
        connection_id: "x".into(),
        sql: "SELECT 1".into(),
        every,
        enabled: true,
        export: None,
        notify: Notify::OnChange,
        created_at: 0,
        runs: vec![],
    }
}

fn ts(y: i32, m: u32, d: u32, h: u32, min: u32) -> i64 {
    Local
        .with_ymd_and_hms(y, m, d, h, min, 0)
        .single()
        .unwrap()
        .timestamp()
}

#[test]
fn next_run_timing() {
    // interval counts from the last run (or creation)
    let mut s = sched(Every::Minutes { n: 15 });
    s.created_at = 1_000;
    assert_eq!(s.next_run(), Some(1_000 + 900));
    s.runs.push(Run {
        at: 5_000,
        ok: true,
        rows: 1,
        elapsed_ms: 1,
        error: None,
        changed: false,
        file: None,
        hash: None,
    });
    assert_eq!(s.next_run(), Some(5_900));
    assert!(s.is_due(5_900) && !s.is_due(5_899));

    // daily at 08:30: same day if created before, next day if after
    let mut s = sched(Every::Daily { at: "08:30".into() });
    s.created_at = ts(2026, 3, 10, 7, 0);
    assert_eq!(s.next_run(), Some(ts(2026, 3, 10, 8, 30)));
    s.created_at = ts(2026, 3, 10, 9, 0);
    assert_eq!(s.next_run(), Some(ts(2026, 3, 11, 8, 30)));

    // weekly on Monday (0) 09:00; 2026-03-11 is a Wednesday
    let mut s = sched(Every::Weekly {
        weekday: 0,
        at: "09:00".into(),
    });
    s.created_at = ts(2026, 3, 11, 12, 0);
    let next = Local.timestamp_opt(s.next_run().unwrap(), 0).unwrap();
    assert_eq!(
        (
            next.weekday().num_days_from_monday(),
            next.day(),
            next.hour()
        ),
        (0, 16, 9)
    );

    s.enabled = false;
    assert_eq!(s.next_run(), None);
}

#[test]
fn validation_blocks_destructive_and_bad_times() {
    assert!(sched(Every::Daily { at: "25:00".into() })
        .validate()
        .is_err());
    assert!(sched(Every::Minutes { n: 0 }).validate().is_err());
    let mut s = sched(Every::Minutes { n: 5 });
    s.sql = "DELETE FROM users".into();
    assert!(s
        .validate()
        .unwrap_err()
        .to_string()
        .contains("destructive"));
    s.sql = "DELETE FROM sessions WHERE expires_at < now()".into();
    assert!(s.validate().is_ok());
}

#[test]
fn store_keeps_runs_and_resets_timing() {
    let dir = tempfile::tempdir().unwrap();
    let store = ScheduleStore::at(dir.path().join("schedules.json"));
    let s = store.save(sched(Every::Minutes { n: 5 })).unwrap();
    assert!(!s.id.is_empty() && s.created_at > 0);
    let run = Run {
        at: s.created_at + 1,
        ok: true,
        rows: 3,
        elapsed_ms: 2,
        error: None,
        changed: false,
        file: None,
        hash: Some("a".into()),
    };
    for _ in 0..25 {
        store.record(&s.id, run.clone()).unwrap();
    }
    let mut edited = store.list().unwrap().remove(0);
    assert_eq!(edited.runs.len(), 20);
    edited.name = "Renamed".into();
    edited.runs.clear(); // the UI doesn't send runs back
    let saved = store.save(edited).unwrap();
    assert_eq!(saved.runs.len(), 20, "editing keeps the run log");
    store.delete(&s.id).unwrap();
    assert!(store.list().unwrap().is_empty());
}

#[tokio::test]
async fn runs_detect_changes_and_export() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("app.db");
    std::fs::File::create(&db).unwrap();
    let mut cfg = ConnectionConfig::new("lite", DbKind::Sqlite);
    cfg.file = Some(db.to_string_lossy().into());
    let c = Connection::connect(&cfg).await.unwrap();
    c.query(
        "CREATE TABLE t (n INTEGER); INSERT INTO t VALUES (1), (2);",
        10,
    )
    .await
    .unwrap();

    let mut s = sched(Every::Minutes { n: 1 });
    s.sql = "SELECT n FROM t ORDER BY n".into();
    let r1 = run_on(&c, &s).await;
    assert!(r1.ok && r1.rows == 2 && !r1.changed, "{r1:?}");
    s.runs.insert(0, r1.clone());
    let r2 = run_on(&c, &s).await;
    assert!(!r2.changed && r2.hash == r1.hash);
    c.query("INSERT INTO t VALUES (3)", 10).await.unwrap();
    let r3 = run_on(&c, &s).await;
    assert!(r3.changed && r3.rows == 3);
    assert!(s.should_notify(&r3));
    assert!(!s.should_notify(&r2));

    // export: one timestamped file per run
    s.export = Some(ExportTarget {
        dir: dir.path().join("out").to_string_lossy().into(),
        format: ExportFormat::Csv,
    });
    let r = run_on(&c, &s).await;
    let file = r.file.clone().unwrap();
    assert!(
        file.ends_with(".csv") && file.contains("Daily-revenue-"),
        "{file}"
    );
    assert_eq!(std::fs::read_to_string(&file).unwrap().lines().count(), 4);

    // failures are recorded, not raised
    s.export = None;
    s.sql = "SELECT nope FROM t".into();
    let bad = run_on(&c, &s).await;
    assert!(!bad.ok && bad.error.is_some());
    assert!(s.should_notify(&bad));
}
