//! rowhouse-core — shared engine used by the desktop app and the MCP server.
//!
//! * [`config`]  — connection profiles on disk + secrets in the OS keychain
//! * [`driver`]  — one `Connection` type over Postgres / MySQL / SQLite
//! * [`guard`]   — read-only SQL classifier (used by MCP + read-only profiles)

pub mod ai;
pub mod backend;
pub mod changes;
pub mod config;
pub mod driver;
pub mod error;
pub mod guard;
pub mod history;
pub mod io;
pub mod model;
pub mod tunnel;

pub use changes::{ChangeSet, RowUpdate};
pub use config::{ConnectionConfig, ConnectionStore, DbKind};
pub use driver::Connection;
pub use error::{Error, Result};
pub use history::{History, HistoryEntry};
pub use io::{ExportFormat, ImportOptions};
pub use model::*;
