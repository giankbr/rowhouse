//! rowhouse-core — shared engine used by the desktop app and the MCP server.
//!
//! * [`config`]  — connection profiles on disk + secrets in the OS keychain
//! * [`driver`]  — one `Connection` type over Postgres / MySQL / SQLite
//! * [`guard`]   — read-only SQL classifier (used by MCP + read-only profiles)

pub mod config;
pub mod driver;
pub mod error;
pub mod guard;
pub mod model;

pub use config::{ConnectionConfig, ConnectionStore, DbKind};
pub use driver::Connection;
pub use error::{Error, Result};
pub use model::*;
