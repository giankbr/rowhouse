//! rowhouse-core — shared engine used by the desktop app and the MCP server.
//!
//! * [`config`]  — connection profiles on disk + secrets in the OS keychain
//! * [`driver`]  — one `Connection` type over Postgres / MySQL / SQLite
//! * [`guard`]   — read-only SQL classifier (used by MCP + read-only profiles)

pub mod activity;
pub mod ai;
pub mod approvals;
pub mod backend;
pub mod browse;
pub mod changes;
pub mod compare;
pub mod config;
pub mod conn_url;
pub mod driver;
pub mod dump;
pub mod erd;
pub mod error;
pub mod guard;
pub mod history;
pub mod import;
pub mod io;
pub mod model;
pub mod plan;
pub mod schedules;
pub mod structure;
pub mod tunnel;

pub use changes::{ChangeSet, RowUpdate};
pub use config::{ConnectionConfig, ConnectionStore, DbKind};
pub use driver::Connection;
pub use error::{Error, Result};
pub use history::{History, HistoryEntry, Snippet};
pub use io::{ExportFormat, ImportOptions};
pub use model::*;

/// Pick one rustls crypto provider for the whole process (several crates pull
/// TLS in; without this, rustls can't decide and panics on the first handshake).
pub fn init_crypto() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}
