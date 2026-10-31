//! Tauri shell: thin command layer over `rowhouse-core`.

use std::{collections::HashMap, sync::Arc};

use rowhouse_core::schedules::{self, Run, Schedule, ScheduleStore};
use rowhouse_core::{
    history::NewEntry, ChangeSet, ColumnInfo, Connection, ConnectionConfig, ConnectionStore, Error,
    ExportFormat, History, HistoryEntry, ImportOptions, QueryOutput, TableInfo,
};
use tauri::{Emitter, Manager, State};
use tokio::sync::Mutex;

type CmdResult<T> = Result<T, Error>;

struct AppState {
    store: ConnectionStore,
    history: Option<History>,
    open: Mutex<HashMap<String, Arc<Connection>>>,
    /// running queries by run id, so they can be cancelled
    runs: std::sync::Mutex<HashMap<String, tokio::task::AbortHandle>>,
    /// database picked in the titlebar for this session (profile stays unchanged)
    db_override: std::sync::Mutex<HashMap<String, String>>,
    /// when each open connection last answered; idle ones are pinged before use
    last_ok: std::sync::Mutex<HashMap<String, std::time::Instant>>,
}

impl AppState {
    async fn conn(&self, id: &str) -> CmdResult<Arc<Connection>> {
        // After the laptop slept or the network changed, an idle connection (or
        // its SSH tunnel) may be dead: ping it first and reconnect transparently.
        let cached = self.open.lock().await.get(id).cloned();
        if let Some(c) = cached {
            let idle = self
                .last_ok
                .lock()
                .ok()
                .and_then(|m| m.get(id).map(|t| t.elapsed()))
                .is_none_or(|d| d > std::time::Duration::from_secs(30));
            let alive = !idle
                || matches!(
                    tokio::time::timeout(std::time::Duration::from_secs(5), c.ping()).await,
                    Ok(Ok(()))
                );
            if alive {
                self.mark_ok(id);
                return Ok(c);
            }
            self.drop_conn(id).await;
        }
        let mut open = self.open.lock().await;
        if let Some(c) = open.get(id) {
            return Ok(c.clone());
        }
        let mut cfg = self.store.get(id)?;
        let saved_db = cfg.database.clone();
        if let Some(db) = self
            .db_override
            .lock()
            .ok()
            .and_then(|m| m.get(id).cloned())
        {
            cfg.database = Some(db);
        }
        let c = Connection::connect(&cfg).await?;
        cfg.database = saved_db;
        // Trust-on-first-use: pin the SSH host key the first time we see it.
        if let (Some(ssh), Some(fp)) = (cfg.ssh.as_mut(), c.ssh_host_key()) {
            if ssh.host_key.is_none() && !fp.is_empty() {
                ssh.host_key = Some(fp.to_string());
                self.store.save(cfg)?;
            }
        }
        let c = Arc::new(c);
        open.insert(id.to_string(), c.clone());
        drop(open);
        self.mark_ok(id);
        Ok(c)
    }

    fn mark_ok(&self, id: &str) {
        if let Ok(mut m) = self.last_ok.lock() {
            m.insert(id.to_string(), std::time::Instant::now());
        }
    }

    async fn drop_conn(&self, id: &str) {
        if let Some(c) = self.open.lock().await.remove(id) {
            c.close().await;
        }
    }
}

#[tauri::command]
fn list_connections(state: State<'_, AppState>) -> CmdResult<Vec<ConnectionConfig>> {
    state.store.list()
}

#[tauri::command]
async fn save_connection(
    state: State<'_, AppState>,
    config: ConnectionConfig,
    password: Option<String>,
    ssh_secret: Option<String>,
) -> CmdResult<ConnectionConfig> {
    // `None` = keep existing secret, `Some("")` = clear it.
    if let Some(p) = password.as_deref() {
        config.set_password(Some(p))?;
    }
    if let Some(s) = ssh_secret.as_deref() {
        config.set_ssh_secret(Some(s))?;
    }
    state.drop_conn(&config.id).await;
    state.store.save(config)
}

#[tauri::command]
async fn delete_connection(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    state.drop_conn(&id).await;
    if let Ok(cfg) = state.store.get(&id) {
        let _ = cfg.set_password(None);
    }
    state.store.delete(&id)
}

/// Returns the SSH host-key fingerprint when tunnelled, so the UI can show / pin it.
#[tauri::command]
async fn test_connection(
    config: ConnectionConfig,
    password: Option<String>,
    ssh_secret: Option<String>,
) -> CmdResult<Option<String>> {
    let password = password.or_else(|| config.password().ok().flatten());
    let ssh_secret = ssh_secret.or_else(|| config.ssh_secret().ok().flatten());
    let c = Connection::connect_with_secrets(&config, password.as_deref(), ssh_secret.as_deref())
        .await?;
    c.ping().await?;
    let fp = c.ssh_host_key().map(str::to_string);
    c.close().await;
    Ok(fp)
}

#[tauri::command]
async fn connect(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    state.conn(&id).await?.ping().await
}

#[tauri::command]
async fn disconnect(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    state.drop_conn(&id).await;
    Ok(())
}

#[tauri::command]
async fn backup_database(
    state: State<'_, AppState>,
    id: String,
    path: String,
    options: rowhouse_core::dump::DumpOptions,
) -> CmdResult<rowhouse_core::dump::DumpReport> {
    let c = state.conn(&id).await?;
    let pw = state.store.get(&id)?.password()?;
    rowhouse_core::dump::backup(&c, pw.as_deref(), &options, std::path::Path::new(&path)).await
}

#[tauri::command]
async fn restore_database(
    state: State<'_, AppState>,
    id: String,
    path: String,
) -> CmdResult<rowhouse_core::dump::DumpReport> {
    let c = state.conn(&id).await?;
    let pw = state.store.get(&id)?.password()?;
    rowhouse_core::dump::restore(&c, pw.as_deref(), std::path::Path::new(&path)).await
}

#[tauri::command]
async fn dump_tools(state: State<'_, AppState>, id: String) -> CmdResult<Vec<(String, bool)>> {
    Ok(rowhouse_core::dump::tools_for(&*state.conn(&id).await?))
}

#[tauri::command]
async fn server_activity(
    state: State<'_, AppState>,
    id: String,
) -> CmdResult<Vec<rowhouse_core::activity::Session>> {
    state.conn(&id).await?.activity().await
}

#[tauri::command]
async fn kill_session(
    state: State<'_, AppState>,
    id: String,
    session: String,
    terminate: bool,
) -> CmdResult<()> {
    state
        .conn(&id)
        .await?
        .kill_session(&session, terminate)
        .await
}

#[tauri::command]
async fn list_indexes(
    state: State<'_, AppState>,
    id: String,
    schema: String,
    table: String,
) -> CmdResult<Vec<rowhouse_core::structure::IndexInfo>> {
    state.conn(&id).await?.list_indexes(&schema, &table).await
}

#[tauri::command]
async fn table_foreign_keys(
    state: State<'_, AppState>,
    id: String,
    schema: String,
    table: String,
) -> CmdResult<Vec<rowhouse_core::erd::ForeignKey>> {
    let all = state.conn(&id).await?.foreign_keys().await?;
    Ok(all
        .into_iter()
        .filter(|f| f.schema == schema && f.table == table)
        .collect())
}

#[tauri::command]
async fn list_objects(
    state: State<'_, AppState>,
    id: String,
) -> CmdResult<Vec<rowhouse_core::objects::DbObject>> {
    state.conn(&id).await?.list_objects().await
}

#[tauri::command]
async fn object_definition(
    state: State<'_, AppState>,
    id: String,
    object: rowhouse_core::objects::DbObject,
) -> CmdResult<String> {
    state.conn(&id).await?.object_definition(&object).await
}

#[tauri::command]
async fn list_roles(
    state: State<'_, AppState>,
    id: String,
) -> CmdResult<Vec<rowhouse_core::roles::RoleInfo>> {
    state.conn(&id).await?.list_roles().await
}

#[tauri::command]
async fn role_grants(
    state: State<'_, AppState>,
    id: String,
    role: String,
) -> CmdResult<Vec<rowhouse_core::roles::Grant>> {
    state.conn(&id).await?.role_grants(&role).await
}

#[tauri::command]
async fn role_action(
    state: State<'_, AppState>,
    id: String,
    op: rowhouse_core::roles::RoleOp,
    preview: bool,
) -> CmdResult<Vec<String>> {
    let c = state.conn(&id).await?;
    if preview {
        c.role_sql(&op, true)
    } else {
        c.apply_role(&op).await
    }
}

// ---- SQL files & dummy data -------------------------------------------------

static SCRIPT_CANCEL: std::sync::LazyLock<
    std::sync::Mutex<HashMap<String, Arc<std::sync::atomic::AtomicBool>>>,
> = std::sync::LazyLock::new(Default::default);

#[tauri::command]
async fn run_sql_file(
    state: State<'_, AppState>,
    id: String,
    path: String,
    options: rowhouse_core::script::RunOptions,
    run_id: String,
    on_progress: tauri::ipc::Channel<rowhouse_core::script::ScriptProgress>,
) -> CmdResult<rowhouse_core::script::ScriptProgress> {
    let c = state.conn(&id).await?;
    let flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
    SCRIPT_CANCEL
        .lock()
        .unwrap()
        .insert(run_id.clone(), flag.clone());
    let r = c
        .run_sql_file(std::path::Path::new(&path), &options, &flag, |p| {
            let _ = on_progress.send(p.clone());
        })
        .await;
    SCRIPT_CANCEL.lock().unwrap().remove(&run_id);
    r
}

#[tauri::command]
fn cancel_sql_file(run_id: String) {
    if let Some(f) = SCRIPT_CANCEL.lock().unwrap().get(&run_id) {
        f.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

#[tauri::command]
async fn fake_plan(
    state: State<'_, AppState>,
    id: String,
    schema: String,
    table: String,
) -> CmdResult<Vec<rowhouse_core::fakegen::ColumnGen>> {
    state.conn(&id).await?.fake_plan(&schema, &table).await
}

#[tauri::command]
async fn fake_preview(
    state: State<'_, AppState>,
    id: String,
    plan: Vec<rowhouse_core::fakegen::ColumnGen>,
    seed: u64,
) -> CmdResult<rowhouse_core::QueryResult> {
    state.conn(&id).await?.fake_preview(&plan, 8, seed).await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn fake_insert(
    state: State<'_, AppState>,
    id: String,
    schema: String,
    table: String,
    plan: Vec<rowhouse_core::fakegen::ColumnGen>,
    count: usize,
    seed: u64,
    on_progress: tauri::ipc::Channel<usize>,
) -> CmdResult<usize> {
    state
        .conn(&id)
        .await?
        .fake_insert(&schema, &table, &plan, count, seed, |n| {
            let _ = on_progress.send(n);
        })
        .await
}

// ---- extensions ---------------------------------------------------------------

use rowhouse_core::extensions::{Installed, Registry};

#[tauri::command]
fn ext_list() -> CmdResult<Vec<Installed>> {
    Registry::default().list()
}

#[tauri::command]
fn ext_install(path: String) -> CmdResult<Installed> {
    Registry::default().install(std::path::Path::new(&path))
}

#[tauri::command]
fn ext_uninstall(ext: String) -> CmdResult<()> {
    Registry::default().uninstall(&ext)
}

#[tauri::command]
fn ext_set_enabled(ext: String, enabled: bool) -> CmdResult<Installed> {
    Registry::default().set_enabled(&ext, enabled)
}

#[tauri::command]
fn ext_entry(ext: String) -> CmdResult<String> {
    Registry::default().entry_html(&ext)
}

/// A query on behalf of an extension: read-only unless it was granted `query:write`.
#[tauri::command]
async fn ext_query(
    state: State<'_, AppState>,
    ext: String,
    id: String,
    sql: String,
    limit: Option<usize>,
) -> CmdResult<QueryOutput> {
    let x = Registry::default().get(&ext)?;
    let can = |p: &str| x.enabled && x.granted.iter().any(|g| g == p);
    let c = state.conn(&id).await?;
    let limit = limit.unwrap_or(1000).min(100_000);
    if can("query:write") {
        c.query(&sql, limit).await
    } else if can("query:read") {
        c.query_read_only(&sql, limit).await
    } else {
        Err(Error::Other(format!(
            "{} isn't allowed to run queries",
            x.manifest.name
        )))
    }
}

#[tauri::command]
async fn ai_detect_local() -> Vec<rowhouse_core::ai_local::Detected> {
    rowhouse_core::ai_local::detect().await
}

// ---- dashboards ---------------------------------------------------------------

use rowhouse_core::dashboards::{Dashboard, DashboardStore};

#[tauri::command]
fn dashboards_list() -> CmdResult<Vec<Dashboard>> {
    DashboardStore::default().list()
}

#[tauri::command]
fn dashboard_save(dashboard: Dashboard) -> CmdResult<Dashboard> {
    DashboardStore::default().save(dashboard)
}

#[tauri::command]
fn dashboard_delete(dashboard_id: String) -> CmdResult<()> {
    DashboardStore::default().delete(&dashboard_id)
}

/// Snapshots the frontend is rendering right now (id -> started at), so the
/// scheduler doesn't fire the same one twice. Entries expire after 5 minutes.
static SNAPSHOTS_IN_FLIGHT: std::sync::Mutex<Option<HashMap<String, i64>>> =
    std::sync::Mutex::new(None);

fn claim_snapshot(id: &str, now: i64) -> bool {
    let mut g = SNAPSHOTS_IN_FLIGHT.lock().unwrap();
    let m = g.get_or_insert_with(HashMap::new);
    if m.get(id).is_some_and(|t| now - t < 300) {
        return false;
    }
    m.insert(id.to_string(), now);
    true
}

/// The frontend finished (or failed) rendering a scheduled snapshot.
#[tauri::command]
fn dashboard_snapshot_done(
    app: tauri::AppHandle,
    dashboard_id: String,
    path: Option<String>,
    error: Option<String>,
) -> CmdResult<()> {
    if let Some(m) = SNAPSHOTS_IN_FLIGHT.lock().unwrap().as_mut() {
        m.remove(&dashboard_id);
    }
    let store = DashboardStore::default();
    store.mark_snapshot(&dashboard_id, schedules::now(), error.clone())?;
    let d = store.list()?.into_iter().find(|d| d.id == dashboard_id);
    if let Some(d) = d.filter(|d| d.snapshot.as_ref().is_some_and(|s| s.notify) || error.is_some())
    {
        use tauri_plugin_notification::NotificationExt;
        let body = match (&error, &path) {
            (Some(e), _) => format!("Snapshot failed: {}", e.lines().next().unwrap_or(e)),
            (None, Some(p)) => format!("Snapshot saved to {p}"),
            (None, None) => "Snapshot saved".to_string(),
        };
        let _ = app
            .notification()
            .builder()
            .title(format!("Rowhouse · {}", d.name))
            .body(body)
            .show();
    }
    Ok(())
}

/// Tiles always run read-only.
#[tauri::command]
async fn dashboard_query(
    state: State<'_, AppState>,
    id: String,
    sql: String,
) -> CmdResult<QueryOutput> {
    state.conn(&id).await?.query_read_only(&sql, 2000).await
}

#[derive(serde::Serialize)]
struct Relations {
    outgoing: Vec<rowhouse_core::erd::ForeignKey>,
    incoming: Vec<rowhouse_core::erd::ForeignKey>,
}

/// Foreign keys from this table and into it (for following links in the grid).
#[tauri::command]
async fn table_relations(
    state: State<'_, AppState>,
    id: String,
    schema: String,
    table: String,
) -> CmdResult<Relations> {
    let all = state.conn(&id).await?.foreign_keys().await?;
    let (mut outgoing, mut incoming) = (vec![], vec![]);
    for f in all {
        if f.schema == schema && f.table == table {
            outgoing.push(f.clone());
        }
        if f.ref_schema == schema && f.ref_table == table {
            incoming.push(f);
        }
    }
    Ok(Relations { outgoing, incoming })
}

#[tauri::command]
async fn list_databases(state: State<'_, AppState>, id: String) -> CmdResult<Vec<String>> {
    state.conn(&id).await?.list_databases().await
}

/// Reconnect to another database on the same server (this session only).
#[tauri::command]
async fn switch_database(
    state: State<'_, AppState>,
    id: String,
    database: String,
) -> CmdResult<()> {
    if let Ok(mut m) = state.db_override.lock() {
        m.insert(id.clone(), database);
    }
    state.drop_conn(&id).await;
    state.conn(&id).await?.ping().await
}

#[tauri::command]
async fn list_tables(state: State<'_, AppState>, id: String) -> CmdResult<Vec<TableInfo>> {
    state.conn(&id).await?.list_tables().await
}

#[tauri::command]
async fn describe_table(
    state: State<'_, AppState>,
    id: String,
    schema: String,
    table: String,
) -> CmdResult<Vec<ColumnInfo>> {
    state.conn(&id).await?.describe_table(&schema, &table).await
}

#[tauri::command]
async fn table_sql(
    state: State<'_, AppState>,
    id: String,
    schema: String,
    table: String,
    limit: usize,
) -> CmdResult<String> {
    Ok(state
        .conn(&id)
        .await?
        .select_table_sql(&schema, &table, limit))
}

#[tauri::command]
async fn run_query(
    state: State<'_, AppState>,
    id: String,
    sql: String,
    limit: Option<usize>,
    run_id: Option<String>,
    timeout_ms: Option<u64>,
) -> CmdResult<QueryOutput> {
    let c = state.conn(&id).await?;
    let started = std::time::Instant::now();
    let limit = limit.unwrap_or(rowhouse_core::driver::DEFAULT_ROW_LIMIT);
    let tag = run_id.unwrap_or_else(|| format!("run-{}", started.elapsed().as_nanos()));
    // run in its own task so cancel_query can abort it
    let task = {
        let (c, sql, tag) = (c.clone(), sql.clone(), tag.clone());
        tokio::spawn(async move { c.query_tagged(&sql, limit, &tag).await })
    };
    if let Ok(mut runs) = state.runs.lock() {
        runs.insert(tag.clone(), task.abort_handle());
    }
    let joined = match timeout_ms.filter(|t| *t > 0) {
        Some(ms) => match tokio::time::timeout(std::time::Duration::from_millis(ms), task).await {
            Ok(j) => j,
            Err(_) => {
                let _ = c.cancel(&tag).await;
                if let Some(h) = state.runs.lock().ok().and_then(|mut r| r.remove(&tag)) {
                    h.abort();
                }
                Ok(Err(Error::Other(format!(
                    "query timed out after {:.0} s (Settings → Query timeout)",
                    ms as f64 / 1000.0
                ))))
            }
        },
        None => task.await,
    };
    if let Ok(mut runs) = state.runs.lock() {
        runs.remove(&tag);
    }
    let res = match joined {
        Ok(r) => r,
        Err(e) if e.is_cancelled() => Err(Error::Other("query cancelled".into())),
        Err(e) => Err(Error::Other(e.to_string())),
    };
    if let Some(h) = &state.history {
        let err = res.as_ref().err().map(|e| e.to_string());
        let rows = res
            .as_ref()
            .map(|o| {
                o.results
                    .iter()
                    .map(|r| r.rows.len().max(r.rows_affected as usize) as i64)
                    .sum()
            })
            .unwrap_or(0);
        let _ = h
            .record(NewEntry {
                connection_id: &id,
                sql: &sql,
                source: "app",
                error: err.as_deref(),
                rows,
                elapsed_ms: started.elapsed().as_millis() as i64,
            })
            .await;
    }
    res
}

/// Stop a running query: cancel it on the server (Postgres / MySQL family) and
/// abort the task so the UI gets control back immediately.
#[tauri::command]
async fn cancel_query(state: State<'_, AppState>, id: String, run_id: String) -> CmdResult<()> {
    if let Some(c) = state.open.lock().await.get(&id).cloned() {
        let _ = c.cancel(&run_id).await;
    }
    if let Some(h) = state.runs.lock().ok().and_then(|mut r| r.remove(&run_id)) {
        h.abort();
    }
    Ok(())
}

#[tauri::command]
async fn list_history(
    state: State<'_, AppState>,
    id: Option<String>,
    search: Option<String>,
    limit: Option<i64>,
) -> CmdResult<Vec<HistoryEntry>> {
    match &state.history {
        Some(h) => {
            h.list(id.as_deref(), search.as_deref(), limit.unwrap_or(200))
                .await
        }
        None => Ok(vec![]),
    }
}

#[tauri::command]
async fn clear_history(state: State<'_, AppState>, id: Option<String>) -> CmdResult<()> {
    match &state.history {
        Some(h) => h.clear(id.as_deref()).await,
        None => Ok(()),
    }
}

#[tauri::command]
async fn preview_changes(
    state: State<'_, AppState>,
    id: String,
    changes: ChangeSet,
) -> CmdResult<Vec<String>> {
    state.conn(&id).await?.preview_changes(&changes)
}

#[tauri::command]
async fn apply_changes(
    state: State<'_, AppState>,
    id: String,
    changes: ChangeSet,
) -> CmdResult<u64> {
    state.conn(&id).await?.apply_changes(&changes).await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn export_query(
    state: State<'_, AppState>,
    id: String,
    sql: String,
    format: ExportFormat,
    path: String,
    schema: Option<String>,
    table: Option<String>,
) -> CmdResult<u64> {
    let c = state.conn(&id).await?;
    let target = schema.as_deref().zip(table.as_deref());
    c.export(&sql, format, std::path::Path::new(&path), target)
        .await
}

/// `SELECT * FROM table` without a row limit (for full-table export).
#[tauri::command]
async fn table_sql_all(
    state: State<'_, AppState>,
    id: String,
    schema: String,
    table: String,
) -> CmdResult<String> {
    Ok(state.conn(&id).await?.select_all_sql(&schema, &table))
}

#[tauri::command]
async fn import_csv(
    state: State<'_, AppState>,
    id: String,
    schema: String,
    table: String,
    path: String,
    options: Option<ImportOptions>,
) -> CmdResult<u64> {
    let opts = options.unwrap_or(ImportOptions {
        empty_as_null: true,
    });
    state
        .conn(&id)
        .await?
        .import_csv(&schema, &table, std::path::Path::new(&path), &opts)
        .await
}

// ---- AI (bring your own model) ------------------------------------------------

use rowhouse_core::ai::{AiAnswer, AiClient, AiSettings};

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct AiSettingsView {
    settings: AiSettings,
    has_key: bool,
}

#[tauri::command]
fn ai_get_settings() -> CmdResult<AiSettingsView> {
    Ok(AiSettingsView {
        settings: AiSettings::load()?,
        has_key: AiSettings::api_key().ok().flatten().is_some(),
    })
}

/// `api_key`: `None` keeps the stored key, `Some("")` removes it.
#[tauri::command]
fn ai_save_settings(settings: AiSettings, api_key: Option<String>) -> CmdResult<()> {
    if let Some(k) = api_key.as_deref() {
        AiSettings::set_api_key(Some(k))?;
    }
    settings.save()
}

fn client_for(settings: AiSettings, api_key: Option<String>) -> CmdResult<AiClient> {
    let key = api_key
        .filter(|k| !k.is_empty())
        .or_else(|| AiSettings::api_key().ok().flatten());
    AiClient::new(settings, key)
}

#[tauri::command]
async fn ai_list_models(settings: AiSettings, api_key: Option<String>) -> CmdResult<Vec<String>> {
    let mut s = settings;
    if s.model.is_empty() {
        s.model = "-".into(); // listing models doesn't need one
    }
    client_for(s, api_key)?.list_models().await
}

#[tauri::command]
async fn ai_test(settings: AiSettings, api_key: Option<String>) -> CmdResult<String> {
    client_for(settings, api_key)?
        .complete("You are a connectivity check.", "Reply with exactly: OK")
        .await
}

/// Streams text deltas to the UI over `on_delta` while the model answers.
fn delta_sink(ch: &tauri::ipc::Channel<String>) -> Option<impl FnMut(&str) + Send + '_> {
    Some(move |s: &str| {
        let _ = ch.send(s.to_string());
    })
}

#[tauri::command]
async fn ai_generate(
    state: State<'_, AppState>,
    id: String,
    prompt: String,
    current_sql: Option<String>,
    on_delta: tauri::ipc::Channel<String>,
) -> CmdResult<AiAnswer> {
    let c = state.conn(&id).await?;
    let mut sink = delta_sink(&on_delta);
    AiClient::from_saved()?
        .generate(
            &c,
            &prompt,
            current_sql.as_deref(),
            sink.as_mut().map(|f| f as &mut (dyn FnMut(&str) + Send)),
        )
        .await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn ai_edit(
    state: State<'_, AppState>,
    id: String,
    instruction: String,
    selection: String,
    before: String,
    after: String,
    on_delta: tauri::ipc::Channel<String>,
) -> CmdResult<rowhouse_core::ai::AiAnswer> {
    let c = state.conn(&id).await?;
    let mut sink = delta_sink(&on_delta);
    rowhouse_core::ai::AiClient::from_saved()?
        .edit(
            &c,
            &instruction,
            &selection,
            &before,
            &after,
            sink.as_mut().map(|f| f as &mut (dyn FnMut(&str) + Send)),
        )
        .await
}

#[tauri::command]
async fn ai_fix(
    state: State<'_, AppState>,
    id: String,
    sql: String,
    error: String,
    on_delta: tauri::ipc::Channel<String>,
) -> CmdResult<AiAnswer> {
    let c = state.conn(&id).await?;
    let mut sink = delta_sink(&on_delta);
    AiClient::from_saved()?
        .fix(
            &c,
            &sql,
            &error,
            sink.as_mut().map(|f| f as &mut (dyn FnMut(&str) + Send)),
        )
        .await
}

#[tauri::command]
async fn ai_explain(
    state: State<'_, AppState>,
    id: String,
    sql: String,
    on_delta: tauri::ipc::Channel<String>,
) -> CmdResult<AiAnswer> {
    let c = state.conn(&id).await?;
    let mut sink = delta_sink(&on_delta);
    AiClient::from_saved()?
        .explain(
            &c,
            &sql,
            sink.as_mut().map(|f| f as &mut (dyn FnMut(&str) + Send)),
        )
        .await
}

/// Ghost-text suggestion at the cursor; empty when inline completions are off.
#[tauri::command]
async fn ai_complete_inline(
    state: State<'_, AppState>,
    id: String,
    before: String,
    after: String,
) -> CmdResult<String> {
    let settings = AiSettings::load()?;
    if !settings.enabled || !settings.inline_completions {
        return Ok(String::new());
    }
    let c = state.conn(&id).await?;
    AiClient::new(settings, AiSettings::api_key()?)?
        .complete_inline(&c, &before, &after)
        .await
}

// ---- safety, ERD, import ---------------------------------------------------

// ---- snippets, plans, compare -------------------------------------------------

#[tauri::command]
async fn list_snippets(
    state: State<'_, AppState>,
    id: Option<String>,
    search: Option<String>,
) -> CmdResult<Vec<rowhouse_core::Snippet>> {
    match &state.history {
        Some(h) => h.list_snippets(id.as_deref(), search.as_deref()).await,
        None => Ok(vec![]),
    }
}

// ---------------------------------------------------------------- scheduled queries

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ScheduleRunEvent {
    schedule: Schedule,
    run: Run,
}

/// Schedules currently running (so a slow one isn't started twice).
static RUNNING: std::sync::LazyLock<std::sync::Mutex<std::collections::HashSet<String>>> =
    std::sync::LazyLock::new(Default::default);

async fn run_schedule(app: &tauri::AppHandle, s: Schedule) -> Option<Run> {
    if !RUNNING.lock().ok()?.insert(s.id.clone()) {
        return None;
    }
    let state = app.state::<AppState>();
    let run = schedules::run_with_store(&state.store, &s).await;
    RUNNING.lock().ok()?.remove(&s.id);
    if let Some(h) = &state.history {
        let _ = h
            .record(NewEntry {
                connection_id: &s.connection_id,
                sql: &s.sql,
                source: "schedule",
                error: run.error.as_deref(),
                rows: run.rows as i64,
                elapsed_ms: run.elapsed_ms as i64,
            })
            .await;
    }
    let saved = ScheduleStore::open_default()
        .record(&s.id, run.clone())
        .ok()
        .flatten()
        .unwrap_or(s);
    if saved.should_notify(&run) {
        use tauri_plugin_notification::NotificationExt;
        let body = match (&run.error, run.changed) {
            (Some(e), _) => format!("Failed: {}", e.lines().next().unwrap_or(e)),
            (None, true) => format!("Result changed · {} rows", run.rows),
            (None, false) => format!("{} rows in {} ms", run.rows, run.elapsed_ms),
        };
        let _ = app
            .notification()
            .builder()
            .title(format!("Rowhouse · {}", saved.name))
            .body(body)
            .show();
    }
    let _ = app.emit(
        "schedule-run",
        ScheduleRunEvent {
            schedule: saved,
            run: run.clone(),
        },
    );
    Some(run)
}

#[tauri::command]
fn schedules_list() -> CmdResult<Vec<Schedule>> {
    ScheduleStore::open_default().list()
}

#[tauri::command]
fn license_status() -> rowhouse_core::license::LicenseStatus {
    rowhouse_core::license::status()
}

#[tauri::command]
fn license_activate(key: String) -> CmdResult<rowhouse_core::license::LicenseStatus> {
    rowhouse_core::license::activate(&key)
}

#[tauri::command]
fn license_deactivate() -> CmdResult<rowhouse_core::license::LicenseStatus> {
    rowhouse_core::license::deactivate()?;
    Ok(rowhouse_core::license::status())
}

#[tauri::command]
fn schedule_save(schedule: Schedule) -> CmdResult<Schedule> {
    if schedule.enabled {
        rowhouse_core::license::require_pro("Scheduled queries")?;
    }
    ScheduleStore::open_default().save(schedule)
}

#[tauri::command]
fn schedule_delete(schedule_id: String) -> CmdResult<()> {
    ScheduleStore::open_default().delete(&schedule_id)
}

#[tauri::command]
async fn schedule_run_now(app: tauri::AppHandle, schedule_id: String) -> CmdResult<Run> {
    rowhouse_core::license::require_pro("Scheduled queries")?;
    let s = ScheduleStore::open_default()
        .list()?
        .into_iter()
        .find(|s| s.id == schedule_id)
        .ok_or_else(|| Error::NotFound(schedule_id.clone()))?;
    run_schedule(&app, s)
        .await
        .ok_or_else(|| Error::Other("this schedule is already running".into()))
}

#[tauri::command]
async fn save_snippet(
    state: State<'_, AppState>,
    snippet: rowhouse_core::Snippet,
) -> CmdResult<rowhouse_core::Snippet> {
    match &state.history {
        Some(h) => h.save_snippet(&snippet).await,
        None => Err(Error::Other("storage unavailable".into())),
    }
}

#[tauri::command]
async fn delete_snippet(state: State<'_, AppState>, snippet_id: i64) -> CmdResult<()> {
    match &state.history {
        Some(h) => h.delete_snippet(snippet_id).await,
        None => Ok(()),
    }
}

#[tauri::command]
async fn explain_plan(
    state: State<'_, AppState>,
    id: String,
    sql: String,
    analyze: bool,
) -> CmdResult<rowhouse_core::plan::Plan> {
    state.conn(&id).await?.explain_plan(&sql, analyze).await
}

#[tauri::command]
async fn compare_schema(
    state: State<'_, AppState>,
    source_id: String,
    source_schema: String,
    target_id: String,
    target_schema: String,
) -> CmdResult<rowhouse_core::compare::SchemaDiff> {
    let source = state.conn(&source_id).await?;
    let target = state.conn(&target_id).await?;
    target
        .compare_schema(&target_schema, &source, &source_schema)
        .await
}

#[derive(serde::Serialize)]
struct ParsedUrl {
    config: ConnectionConfig,
    password: Option<String>,
}

/// Paste-a-connection-string support (Neon, Supabase, any postgres:// / mysql:// URL).
#[tauri::command]
fn parse_connection_url(url: String) -> CmdResult<ParsedUrl> {
    let (config, password) = rowhouse_core::conn_url::parse_connection_url(&url)?;
    Ok(ParsedUrl { config, password })
}

#[tauri::command]
async fn table_page(
    state: State<'_, AppState>,
    id: String,
    request: rowhouse_core::browse::PageRequest,
) -> CmdResult<rowhouse_core::browse::Page> {
    state.conn(&id).await?.table_page(&request).await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn compare_data(
    state: State<'_, AppState>,
    source_id: String,
    source_schema: String,
    source_table: String,
    target_id: String,
    target_schema: String,
    target_table: String,
) -> CmdResult<rowhouse_core::compare::DataDiff> {
    let source = state.conn(&source_id).await?;
    let target = state.conn(&target_id).await?;
    target
        .compare_data(
            &target_schema,
            &target_table,
            &source,
            &source_schema,
            &source_table,
        )
        .await
}

// ---- structure editor ---------------------------------------------------------

#[tauri::command]
async fn structure_preview(
    state: State<'_, AppState>,
    id: String,
    schema: String,
    table: String,
    ops: Vec<rowhouse_core::structure::ColumnOp>,
) -> CmdResult<Vec<String>> {
    state.conn(&id).await?.structure_sql(&schema, &table, &ops)
}

#[tauri::command]
async fn apply_structure(
    state: State<'_, AppState>,
    id: String,
    schema: String,
    table: String,
    ops: Vec<rowhouse_core::structure::ColumnOp>,
) -> CmdResult<Vec<String>> {
    state
        .conn(&id)
        .await?
        .apply_structure(&schema, &table, &ops)
        .await
}

#[tauri::command]
async fn table_ddl(
    state: State<'_, AppState>,
    id: String,
    schema: String,
    table: String,
) -> CmdResult<String> {
    state.conn(&id).await?.table_ddl(&schema, &table).await
}

#[tauri::command]
async fn table_action(
    state: State<'_, AppState>,
    id: String,
    schema: String,
    table: String,
    is_view: bool,
    action: rowhouse_core::table_ops::TableAction,
    preview: bool,
) -> CmdResult<Vec<String>> {
    let c = state.conn(&id).await?;
    if preview {
        c.table_action_sql(&schema, &table, is_view, &action).await
    } else {
        c.table_action(&schema, &table, is_view, &action).await
    }
}

#[tauri::command]
async fn create_table(
    state: State<'_, AppState>,
    id: String,
    schema: String,
    table: String,
    columns: Vec<rowhouse_core::ColumnInfo>,
    preview: bool,
) -> CmdResult<String> {
    let c = state.conn(&id).await?;
    if preview {
        c.create_table_sql(&schema, &table, &columns)
    } else {
        c.create_table(&schema, &table, &columns).await
    }
}

// ---- folder sync (Pro) -----------------------------------------------------

use rowhouse_core::sync::{self as fsync, SyncReport, SyncStatus, Syncer};

#[tauri::command]
fn sync_status() -> SyncStatus {
    fsync::status()
}

async fn do_sync(
    state: &AppState,
    folder: &str,
    pass: &str,
    include_secrets: bool,
) -> CmdResult<SyncReport> {
    let history = state
        .history
        .as_ref()
        .ok_or_else(|| Error::Other("history database unavailable".into()))?;
    Syncer {
        store: &state.store,
        history,
        state_path: fsync::SyncState::path(),
        keychain: true,
        dashboards: Some(rowhouse_core::dashboards::DashboardStore::default_path()),
    }
    .sync(std::path::Path::new(folder), pass, include_secrets)
    .await
}

/// Turn sync on (or change folder / passphrase) and sync once.
#[tauri::command]
async fn sync_setup(
    state: State<'_, AppState>,
    folder: String,
    passphrase: String,
    include_secrets: bool,
) -> CmdResult<SyncReport> {
    rowhouse_core::license::require_pro("Sync")?;
    let history = state
        .history
        .as_ref()
        .ok_or_else(|| Error::Other("history database unavailable".into()))?;
    let path = fsync::SyncState::path();
    // a different folder starts a fresh three-way base
    let mut st = fsync::SyncState::load_from(&path);
    if st.folder.as_deref() != Some(folder.as_str()) {
        st.conns.clear();
        st.snippets.clear();
        st.dashboards.clear();
        st.save_to(&path)?;
    }
    Syncer {
        store: &state.store,
        history,
        state_path: path,
        keychain: true,
        dashboards: Some(rowhouse_core::dashboards::DashboardStore::default_path()),
    }
    .check(std::path::Path::new(&folder), &passphrase)?;
    fsync::save_passphrase(Some(&passphrase))?;
    let r = do_sync(&state, &folder, &passphrase, include_secrets).await;
    if let Err(e) = &r {
        fsync::record_error(&e.to_string());
    }
    r
}

#[tauri::command]
async fn sync_now(state: State<'_, AppState>) -> CmdResult<SyncReport> {
    rowhouse_core::license::require_pro("Sync")?;
    let st = fsync::status();
    let folder = st
        .state
        .folder
        .ok_or_else(|| Error::Other("sync is off".into()))?;
    let pass = fsync::saved_passphrase()
        .ok_or_else(|| Error::Other("enter the sync passphrase again in Settings → Sync".into()))?;
    let r = do_sync(&state, &folder, &pass, st.state.include_secrets).await;
    if let Err(e) = &r {
        fsync::record_error(&e.to_string());
    }
    r
}

#[tauri::command]
fn sync_disable() -> CmdResult<()> {
    fsync::disable()
}

// ---- MCP write approvals ----------------------------------------------------

use rowhouse_core::approvals::{ApprovalAnswer, ApprovalRequest, Approvals};

#[tauri::command]
fn approvals_pending() -> CmdResult<Vec<ApprovalRequest>> {
    Approvals::open_default()?.pending()
}

#[tauri::command]
fn approvals_answer(id: String, approved: bool, reason: Option<String>) -> CmdResult<()> {
    Approvals::open_default()?.answer(&id, &ApprovalAnswer { approved, reason })
}

/// Write a text file chosen by the user in a save dialog (ERD export).
#[tauri::command]
fn write_text_file(path: String, contents: String) -> CmdResult<()> {
    std::fs::write(path, contents)?;
    Ok(())
}

#[tauri::command]
fn write_base64_file(path: String, data: String) -> CmdResult<()> {
    let bytes = rowhouse_core::backend::libsql::base64_decode(&data)
        .ok_or_else(|| Error::Other("invalid file data".into()))?;
    let path = std::path::PathBuf::from(rowhouse_core::tunnel::expand_home(&path));
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, bytes)?;
    Ok(())
}

/// Statements that need a typed confirmation before they run.
#[tauri::command]
fn check_destructive(sql: String) -> Vec<rowhouse_core::guard::Destructive> {
    rowhouse_core::guard::destructive(&sql)
}

#[tauri::command]
async fn erd(
    state: State<'_, AppState>,
    id: String,
    schema: Option<String>,
) -> CmdResult<rowhouse_core::erd::Erd> {
    state.conn(&id).await?.erd(schema.as_deref()).await
}

#[tauri::command]
fn import_detect() -> Vec<rowhouse_core::import::DetectedFile> {
    rowhouse_core::import::detect()
}

#[derive(serde::Serialize)]
struct ImportPreview {
    source: rowhouse_core::import::Source,
    candidates: Vec<rowhouse_core::import::Candidate>,
}

#[tauri::command]
fn import_parse(path: String) -> CmdResult<ImportPreview> {
    let (source, candidates) = rowhouse_core::import::parse_file(std::path::Path::new(&path))?;
    Ok(ImportPreview { source, candidates })
}

/// Save the selected imported profiles (fresh ids, no passwords).
#[tauri::command]
fn import_save(
    state: State<'_, AppState>,
    configs: Vec<ConnectionConfig>,
) -> CmdResult<Vec<ConnectionConfig>> {
    let mut saved = Vec::new();
    for mut c in configs {
        c.id = ConnectionConfig::new("", c.kind).id;
        saved.push(state.store.save(c)?);
    }
    Ok(saved)
}

fn mcp_bin() -> String {
    let exe = std::env::current_exe().ok();
    let bin_name = if cfg!(windows) {
        "rowhouse-mcp.exe"
    } else {
        "rowhouse-mcp"
    };
    exe.as_ref()
        .and_then(|p| p.parent())
        .map(|d| d.join(bin_name).to_string_lossy().to_string())
        .unwrap_or_else(|| bin_name.to_string())
}

#[tauri::command]
fn mcp_clients() -> CmdResult<Vec<rowhouse_core::mcp_clients::McpClient>> {
    rowhouse_core::mcp_clients::list(&mcp_bin())
}

#[tauri::command]
fn mcp_install(client: String) -> CmdResult<String> {
    rowhouse_core::mcp_clients::install(&client, &mcp_bin())
}

#[tauri::command]
fn mcp_remove(client: String) -> CmdResult<()> {
    rowhouse_core::mcp_clients::remove(&client)
}

/// Info for the "Connect an AI assistant" panel.
#[tauri::command]
fn mcp_info(state: State<'_, AppState>) -> serde_json::Value {
    let bin = mcp_bin();
    serde_json::json!({
        "command": bin,
        "configPath": state.store.path().to_string_lossy(),
        "snippet": { "mcpServers": { "rowhouse": { "command": bin } } },
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            let history = tauri::async_runtime::block_on(History::open_default())
                .map_err(|e| eprintln!("history disabled: {e}"))
                .ok();
            // Auto-update only once a signing key is configured (see RELEASING.md):
            // the updater plugin refuses to start without a public key.
            let updater_ready = app
                .config()
                .plugins
                .0
                .get("updater")
                .and_then(|u| u.get("pubkey"))
                .and_then(|k| k.as_str())
                .is_some_and(|k| !k.trim().is_empty());
            if updater_ready {
                app.handle()
                    .plugin(tauri_plugin_updater::Builder::new().build())?;
            }

            // Tell rowhouse-mcp the app is open, so write requests can be approved.
            tauri::async_runtime::spawn(async {
                if let Ok(a) = Approvals::open_default() {
                    loop {
                        let _ = a.heartbeat();
                        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    }
                }
            });
            // Scheduled queries: check every 20 s while the app is open.
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(20)).await;
                    let now = schedules::now();
                    if !rowhouse_core::license::status().pro {
                        continue;
                    }
                    let Ok(due) = ScheduleStore::open_default().list() else {
                        continue;
                    };
                    for s in due.into_iter().filter(|s| s.is_due(now)) {
                        let h = handle.clone();
                        tauri::async_runtime::spawn(async move {
                            run_schedule(&h, s).await;
                        });
                    }
                }
            });
            // Dashboard snapshots: the frontend renders them, we only say when.
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                    if !rowhouse_core::license::status().pro {
                        continue;
                    }
                    let Ok(all) = DashboardStore::default().list() else {
                        continue;
                    };
                    let now = schedules::now();
                    for d in Dashboard::due(&all, now) {
                        if claim_snapshot(&d.id, now) {
                            let _ = handle.emit("dashboard-snapshot-due", d);
                        }
                    }
                }
            });
            // Folder sync: every 60 s while the app is open.
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                    if !rowhouse_core::license::status().pro {
                        continue;
                    }
                    let st = fsync::status();
                    let (Some(folder), Some(pass)) =
                        (st.state.folder.clone(), fsync::saved_passphrase())
                    else {
                        continue;
                    };
                    let state = handle.state::<AppState>();
                    match do_sync(&state, &folder, &pass, st.state.include_secrets).await {
                        Ok(r) if r.pulled_connections + r.pulled_snippets > 0 => {
                            let _ = handle.emit("sync-pulled", r);
                        }
                        Ok(_) => {}
                        Err(e) => fsync::record_error(&e.to_string()),
                    }
                }
            });
            app.manage(AppState {
                store: ConnectionStore::open_default(),
                history,
                open: Mutex::new(HashMap::new()),
                runs: Default::default(),
                db_override: Default::default(),
                last_ok: Default::default(),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_connections,
            save_connection,
            delete_connection,
            test_connection,
            connect,
            disconnect,
            list_tables,
            describe_table,
            table_sql,
            run_query,
            list_databases,
            backup_database,
            restore_database,
            dump_tools,
            server_activity,
            kill_session,
            list_indexes,
            table_ddl,
            table_relations,
            list_objects,
            dashboards_list,
            dashboard_save,
            dashboard_delete,
            dashboard_query,
            dashboard_snapshot_done,
            ai_detect_local,
            mcp_clients,
            mcp_install,
            mcp_remove,
            ext_list,
            ext_install,
            ext_uninstall,
            ext_set_enabled,
            ext_entry,
            ext_query,
            run_sql_file,
            cancel_sql_file,
            fake_plan,
            fake_preview,
            fake_insert,
            list_roles,
            role_grants,
            role_action,
            object_definition,
            table_action,
            create_table,
            table_foreign_keys,
            switch_database,
            cancel_query,
            list_history,
            clear_history,
            preview_changes,
            apply_changes,
            export_query,
            table_sql_all,
            import_csv,
            mcp_info,
            write_text_file,
            write_base64_file,
            sync_status,
            sync_setup,
            sync_now,
            sync_disable,
            approvals_pending,
            structure_preview,
            apply_structure,
            approvals_answer,
            parse_connection_url,
            table_page,
            compare_data,
            list_snippets,
            save_snippet,
            delete_snippet,
            explain_plan,
            compare_schema,
            check_destructive,
            erd,
            import_detect,
            import_parse,
            import_save,
            ai_get_settings,
            ai_save_settings,
            ai_list_models,
            ai_test,
            ai_generate,
            ai_fix,
            ai_edit,
            ai_explain,
            ai_complete_inline,
            schedules_list,
            license_status,
            license_activate,
            license_deactivate,
            schedule_save,
            schedule_delete,
            schedule_run_now,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Rowhouse");
}
