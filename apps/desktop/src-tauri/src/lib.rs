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
}

impl AppState {
    async fn conn(&self, id: &str) -> CmdResult<Arc<Connection>> {
        let mut open = self.open.lock().await;
        if let Some(c) = open.get(id) {
            return Ok(c.clone());
        }
        let mut cfg = self.store.get(id)?;
        let c = Connection::connect(&cfg).await?;
        // Trust-on-first-use: pin the SSH host key the first time we see it.
        if let (Some(ssh), Some(fp)) = (cfg.ssh.as_mut(), c.ssh_host_key()) {
            if ssh.host_key.is_none() && !fp.is_empty() {
                ssh.host_key = Some(fp.to_string());
                self.store.save(cfg)?;
            }
        }
        let c = Arc::new(c);
        open.insert(id.to_string(), c.clone());
        Ok(c)
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
fn schedule_save(schedule: Schedule) -> CmdResult<Schedule> {
    ScheduleStore::open_default().save(schedule)
}

#[tauri::command]
fn schedule_delete(schedule_id: String) -> CmdResult<()> {
    ScheduleStore::open_default().delete(&schedule_id)
}

#[tauri::command]
async fn schedule_run_now(app: tauri::AppHandle, schedule_id: String) -> CmdResult<Run> {
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

/// Info for the "Connect an AI assistant" panel.
#[tauri::command]
fn mcp_info(state: State<'_, AppState>) -> serde_json::Value {
    let exe = std::env::current_exe().ok();
    let bin_name = if cfg!(windows) {
        "rowhouse-mcp.exe"
    } else {
        "rowhouse-mcp"
    };
    let bin = exe
        .as_ref()
        .and_then(|p| p.parent())
        .map(|d| d.join(bin_name).to_string_lossy().to_string())
        .unwrap_or_else(|| bin_name.to_string());
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
            app.manage(AppState {
                store: ConnectionStore::open_default(),
                history,
                open: Mutex::new(HashMap::new()),
                runs: Default::default(),
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
            schedule_save,
            schedule_delete,
            schedule_run_now,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Rowhouse");
}
