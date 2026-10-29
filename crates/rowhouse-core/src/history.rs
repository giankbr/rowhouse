//! Query history, shared by the desktop app and the MCP server.
//! Stored in `history.db` (SQLite) next to `connections.json`.

use std::path::Path;

use serde::{Deserialize, Serialize};
use sqlx::{
    sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions},
    Row,
};

use crate::Result;

/// Entries older than the newest `KEEP` are pruned on insert.
const KEEP: i64 = 5_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub id: i64,
    pub connection_id: String,
    pub sql: String,
    /// "app" | "mcp"
    pub source: String,
    pub ok: bool,
    pub error: Option<String>,
    pub rows: i64,
    pub elapsed_ms: i64,
    /// unix seconds
    pub at: i64,
}

#[derive(Clone)]
pub struct History {
    pool: SqlitePool,
}

#[derive(Debug, Clone, Default)]
pub struct NewEntry<'a> {
    pub connection_id: &'a str,
    pub sql: &'a str,
    pub source: &'a str,
    pub error: Option<&'a str>,
    pub rows: i64,
    pub elapsed_ms: i64,
}

impl History {
    pub async fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(path)
                    .create_if_missing(true)
                    .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
                    .busy_timeout(std::time::Duration::from_secs(3)),
            )
            .await?;
        sqlx::raw_sql(
            "CREATE TABLE IF NOT EXISTS history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                connection_id TEXT NOT NULL,
                sql TEXT NOT NULL,
                source TEXT NOT NULL DEFAULT 'app',
                error TEXT,
                rows INTEGER NOT NULL DEFAULT 0,
                elapsed_ms INTEGER NOT NULL DEFAULT 0,
                at INTEGER NOT NULL DEFAULT (unixepoch())
             );
             CREATE INDEX IF NOT EXISTS history_conn_at ON history (connection_id, at DESC);",
        )
        .execute(&pool)
        .await?;
        Ok(Self { pool })
    }

    pub async fn open_default() -> Result<Self> {
        Self::open(&crate::ConnectionStore::default_dir().join("history.db")).await
    }

    pub async fn record(&self, e: NewEntry<'_>) -> Result<()> {
        sqlx::query(
            "INSERT INTO history (connection_id, sql, source, error, rows, elapsed_ms) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(e.connection_id)
        .bind(e.sql.trim())
        .bind(e.source)
        .bind(e.error)
        .bind(e.rows)
        .bind(e.elapsed_ms)
        .execute(&self.pool)
        .await?;
        sqlx::query("DELETE FROM history WHERE id <= (SELECT max(id) FROM history) - ?")
            .bind(KEEP)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Newest first. `search` matches anywhere in the SQL (case-insensitive).
    pub async fn list(
        &self,
        connection_id: Option<&str>,
        search: Option<&str>,
        limit: i64,
    ) -> Result<Vec<HistoryEntry>> {
        let rows = sqlx::query(
            "SELECT id, connection_id, sql, source, error, rows, elapsed_ms, at FROM history
             WHERE (?1 IS NULL OR connection_id = ?1)
               AND (?2 IS NULL OR sql LIKE '%' || ?2 || '%')
             ORDER BY id DESC LIMIT ?3",
        )
        .bind(connection_id)
        .bind(search.filter(|s| !s.is_empty()))
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| {
                let error: Option<String> = r.get("error");
                HistoryEntry {
                    id: r.get("id"),
                    connection_id: r.get("connection_id"),
                    sql: r.get("sql"),
                    source: r.get("source"),
                    ok: error.is_none(),
                    error,
                    rows: r.get("rows"),
                    elapsed_ms: r.get("elapsed_ms"),
                    at: r.get("at"),
                }
            })
            .collect())
    }

    pub async fn clear(&self, connection_id: Option<&str>) -> Result<()> {
        sqlx::query("DELETE FROM history WHERE ?1 IS NULL OR connection_id = ?1")
            .bind(connection_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}
