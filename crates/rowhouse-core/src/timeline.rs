//! Schema timeline: a snapshot of every table's columns, kept whenever the
//! schema changes (checked on connect and after DDL), so you can see what
//! changed when and get the migration SQL between any two points.
//!
//! Stored per connection in `schema-history/<id>.json`, newest last.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{compare::Change, ColumnInfo, Connection, ConnectionStore, Error, Result};

/// schema → table → columns
pub type Schema = BTreeMap<String, BTreeMap<String, Vec<ColumnInfo>>>;

/// Snapshots kept per connection.
const KEEP: usize = 200;
/// Databases with more tables are only snapshotted on request.
pub const AUTO_MAX_TABLES: usize = 300;
/// Automatic checks at most this often (seconds).
const MIN_INTERVAL: i64 = 600;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub at: i64,
    pub hash: String,
    pub schema: Schema,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct File {
    #[serde(default)]
    snapshots: Vec<Snapshot>,
    /// last time we looked (even if nothing changed)
    #[serde(default)]
    checked_at: i64,
}

/// One point on the timeline, with what changed since the one before.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub at: i64,
    pub tables: usize,
    pub columns: usize,
    pub tables_added: usize,
    pub tables_removed: usize,
    pub columns_added: usize,
    pub columns_removed: usize,
    pub columns_changed: usize,
    /// the first snapshot: nothing to compare with
    pub initial: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaChanges {
    pub schema: String,
    pub changes: Vec<Change>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineDiff {
    pub from: i64,
    pub to: i64,
    pub schemas: Vec<SchemaChanges>,
    /// migration from `from` to `to`, in this connection's dialect
    pub sql: String,
}

fn path(conn_id: &str) -> PathBuf {
    let safe: String = conn_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    ConnectionStore::default_dir()
        .join("schema-history")
        .join(format!("{safe}.json"))
}

fn load(conn_id: &str) -> File {
    std::fs::read(path(conn_id))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save(conn_id: &str, f: &File) -> Result<()> {
    let p = path(conn_id);
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec(f)?)?;
    std::fs::rename(tmp, p)?;
    Ok(())
}

fn hash(s: &Schema) -> String {
    let b = serde_json::to_vec(s).unwrap_or_default();
    let d = ring::digest::digest(&ring::digest::SHA256, &b);
    crate::sync::base64_encode(&d.as_ref()[..12])
}

fn counts(s: &Schema) -> (usize, usize) {
    let t = s.values().map(|x| x.len()).sum();
    let c = s.values().flat_map(|x| x.values()).map(|c| c.len()).sum();
    (t, c)
}

/// What changed from `old` to `new`, per schema.
pub fn changes(old: &Schema, new: &Schema) -> Vec<SchemaChanges> {
    let empty = BTreeMap::new();
    let mut names: Vec<&String> = old.keys().chain(new.keys()).collect();
    names.sort();
    names.dedup();
    names
        .into_iter()
        .filter_map(|s| {
            let (c, _) =
                crate::compare::diff(new.get(s).unwrap_or(&empty), old.get(s).unwrap_or(&empty));
            (!c.is_empty()).then(|| SchemaChanges {
                schema: s.clone(),
                changes: c,
            })
        })
        .collect()
}

fn entry(prev: Option<&Snapshot>, s: &Snapshot) -> Entry {
    let (tables, columns) = counts(&s.schema);
    let mut e = Entry {
        at: s.at,
        tables,
        columns,
        tables_added: 0,
        tables_removed: 0,
        columns_added: 0,
        columns_removed: 0,
        columns_changed: 0,
        initial: prev.is_none(),
    };
    if let Some(p) = prev {
        for sc in changes(&p.schema, &s.schema) {
            for c in sc.changes {
                match c {
                    Change::TableAdded { .. } => e.tables_added += 1,
                    Change::TableRemoved { .. } => e.tables_removed += 1,
                    Change::ColumnAdded { .. } => e.columns_added += 1,
                    Change::ColumnRemoved { .. } => e.columns_removed += 1,
                    Change::ColumnChanged { .. } => e.columns_changed += 1,
                }
            }
        }
    }
    e
}

/// The timeline, newest first.
pub fn entries(conn_id: &str) -> Vec<Entry> {
    let f = load(conn_id);
    let mut out: Vec<Entry> = f
        .snapshots
        .iter()
        .enumerate()
        .map(|(i, s)| entry(i.checked_sub(1).map(|j| &f.snapshots[j]), s))
        .collect();
    out.reverse();
    out
}

pub fn forget(conn_id: &str) -> Result<()> {
    match std::fs::remove_file(path(conn_id)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

impl Connection {
    /// Every table's columns, grouped by schema.
    pub async fn schema_snapshot(&self, max_tables: Option<usize>) -> Result<Schema> {
        let tables: Vec<_> = self
            .list_tables()
            .await?
            .into_iter()
            .filter(|t| t.kind == "table")
            .collect();
        if let Some(m) = max_tables {
            if tables.len() > m {
                return Err(Error::Other(format!(
                    "{} tables — snapshot it from the timeline when you need it",
                    tables.len()
                )));
            }
        }
        let mut out = Schema::new();
        for t in tables {
            let cols = self.describe_table(&t.schema, &t.name).await?;
            out.entry(t.schema.clone())
                .or_default()
                .insert(t.name, cols);
        }
        Ok(out)
    }

    /// Take a snapshot and keep it if the schema changed. `force` skips the
    /// "checked recently" and table-count limits. Returns the new entry, if any.
    pub async fn record_schema(&self, force: bool) -> Result<Option<Entry>> {
        let id = self.config.id.clone();
        let mut f = load(&id);
        let now = crate::schedules::now();
        if !force && now - f.checked_at < MIN_INTERVAL {
            return Ok(None);
        }
        let schema = self
            .schema_snapshot((!force).then_some(AUTO_MAX_TABLES))
            .await?;
        f.checked_at = now;
        let h = hash(&schema);
        if f.snapshots.last().is_some_and(|s| s.hash == h) {
            save(&id, &f)?;
            return Ok(None);
        }
        // keep times unique (they identify snapshots)
        let at = f.snapshots.last().map_or(now, |l| now.max(l.at + 1));
        let snap = Snapshot {
            at,
            hash: h,
            schema,
        };
        let e = entry(f.snapshots.last(), &snap);
        f.snapshots.push(snap);
        if f.snapshots.len() > KEEP {
            // keep the first one (the baseline) and the newest
            let drop = f.snapshots.len() - KEEP;
            f.snapshots.drain(1..=drop);
        }
        save(&id, &f)?;
        Ok(Some(e))
    }

    /// What changed between two snapshots (by time), and the migration SQL.
    /// `to = None` compares with the newest.
    pub fn schema_timeline_diff(&self, from: i64, to: Option<i64>) -> Result<TimelineDiff> {
        let f = load(&self.config.id);
        let find = |at: i64| f.snapshots.iter().find(|s| s.at == at);
        let a = find(from).ok_or_else(|| Error::Other("that snapshot is gone".into()))?;
        let b = match to {
            Some(t) => find(t).ok_or_else(|| Error::Other("that snapshot is gone".into()))?,
            None => f
                .snapshots
                .last()
                .ok_or_else(|| Error::Other("no snapshots yet".into()))?,
        };
        let schemas = changes(&a.schema, &b.schema);
        let sql = if schemas.is_empty() {
            "-- No schema changes between these points.".to_string()
        } else {
            schemas
                .iter()
                .map(|s| {
                    format!(
                        "-- schema {}\n{}",
                        s.schema,
                        self.migration_sql(&s.schema, &s.changes)
                    )
                })
                .collect::<Vec<_>>()
                .join("\n\n")
        };
        Ok(TimelineDiff {
            from: a.at,
            to: b.at,
            schemas,
            sql,
        })
    }
}
