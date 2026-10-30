//! Folder sync between two "devices" sharing one folder.
use rowhouse_core::{
    history::{History, Snippet},
    sync::{Syncer, FILE_NAME},
    ConnectionConfig, ConnectionStore, DbKind,
};

struct Device {
    _dir: tempfile::TempDir,
    store: ConnectionStore,
    history: History,
    state: std::path::PathBuf,
}

async fn device() -> Device {
    let dir = tempfile::tempdir().unwrap();
    let store = ConnectionStore::at(dir.path().join("connections.json"));
    let history = History::open(&dir.path().join("history.db")).await.unwrap();
    let state = dir.path().join("sync.json");
    Device {
        _dir: dir,
        store,
        history,
        state,
    }
}

impl Device {
    fn syncer(&self) -> Syncer<'_> {
        Syncer {
            store: &self.store,
            history: &self.history,
            state_path: self.state.clone(),
            keychain: false,
        }
    }
}

const PASS: &str = "correct horse battery";

#[tokio::test]
async fn two_devices_converge() {
    let shared = tempfile::tempdir().unwrap();
    let folder = shared.path();
    let a = device().await;
    let b = device().await;

    let mut pg = ConnectionConfig::new("Prod", DbKind::Postgres);
    pg.host = Some("db.example.com".into());
    a.store.save(pg.clone()).unwrap();
    a.history
        .save_snippet(&Snippet {
            id: 0,
            name: "Revenue".into(),
            sql: "SELECT 1".into(),
            connection_id: None,
            updated_at: 0,
        })
        .await
        .unwrap();

    let r = a.syncer().sync(folder, PASS, false).await.unwrap();
    assert!(r.pushed);
    let raw = std::fs::read_to_string(folder.join(FILE_NAME)).unwrap();
    assert!(
        !raw.contains("db.example.com") && !raw.contains("Revenue"),
        "file must be encrypted"
    );

    // wrong passphrase is refused
    assert!(b
        .syncer()
        .sync(folder, "wrong passphrase", false)
        .await
        .is_err());

    // B pulls everything
    let r = b.syncer().sync(folder, PASS, false).await.unwrap();
    assert_eq!((r.pulled_connections, r.pulled_snippets), (1, 1));
    assert!(!r.pushed, "nothing new on B");
    assert_eq!(
        b.store.get(&pg.id).unwrap().host.as_deref(),
        Some("db.example.com")
    );

    // B edits the connection, A edits the snippet — both survive
    let mut edited = b.store.get(&pg.id).unwrap();
    edited.host = Some("db2.example.com".into());
    b.store.save(edited).unwrap();
    assert!(b.syncer().sync(folder, PASS, false).await.unwrap().pushed);

    let mut sn = a.history.list_snippets(None, None).await.unwrap().remove(0);
    sn.sql = "SELECT 2".into();
    a.history.save_snippet(&sn).await.unwrap();
    let r = a.syncer().sync(folder, PASS, false).await.unwrap();
    assert_eq!(r.pulled_connections, 1);
    assert!(r.pushed);
    assert_eq!(
        a.store.get(&pg.id).unwrap().host.as_deref(),
        Some("db2.example.com")
    );

    let r = b.syncer().sync(folder, PASS, false).await.unwrap();
    assert_eq!(r.pulled_snippets, 1);
    assert_eq!(
        b.history.list_snippets(None, None).await.unwrap()[0].sql,
        "SELECT 2"
    );

    // steady state: nothing moves
    let r = a.syncer().sync(folder, PASS, false).await.unwrap();
    assert!(!r.pushed && r.pulled_connections == 0 && r.pulled_snippets == 0);

    // deleting locally doesn't resurrect from the file
    a.store.delete(&pg.id).unwrap();
    a.syncer().sync(folder, PASS, false).await.unwrap();
    assert!(a.store.get(&pg.id).is_err());
}
