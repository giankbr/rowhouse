//! Tauri shell: thin command layer over `rowhouse-core`.

use std::{collections::HashMap, sync::Arc};

use rowhouse_core::{
    ColumnInfo, Connection, ConnectionConfig, ConnectionStore, Error, QueryOutput, TableInfo,
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
        let cfg = self.store.get(id)?;
        let c = Arc::new(Connection::connect(&cfg).await?);
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
) -> CmdResult<ConnectionConfig> {
    // `None` = keep existing password, `Some("")` = clear it.
    if let Some(p) = password.as_deref() {
        config.set_password(Some(p))?;
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

#[tauri::command]
async fn test_connection(config: ConnectionConfig, password: Option<String>) -> CmdResult<()> {
    let password = match password {
        Some(p) => Some(p),
        None => config.password().ok().flatten(),
    };
    let c = Connection::connect_with(&config, password.as_deref()).await?;
    c.ping().await?;
    c.close().await;
    Ok(())
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
            mcp_info,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Rowhouse");
}
