//! Tauri shell: thin command layer over `rowhouse-core`.

use std::{collections::HashMap, sync::Arc};

use rowhouse_core::{
    ChangeSet, ColumnInfo, Connection, ConnectionConfig, ConnectionStore, Error, ExportFormat,
    ImportOptions, QueryOutput, TableInfo,
};
use tauri::{Manager, State};
use tokio::sync::Mutex;

type CmdResult<T> = Result<T, Error>;

struct AppState {
    store: ConnectionStore,
    open: Mutex<HashMap<String, Arc<Connection>>>,
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
) -> CmdResult<QueryOutput> {
    let c = state.conn(&id).await?;
    c.query(
        &sql,
        limit.unwrap_or(rowhouse_core::driver::DEFAULT_ROW_LIMIT),
    )
    .await
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
        .setup(|app| {
            app.manage(AppState {
                store: ConnectionStore::open_default(),
                open: Mutex::new(HashMap::new()),
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
            preview_changes,
            apply_changes,
            export_query,
            table_sql_all,
            import_csv,
            mcp_info,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Rowhouse");
}
