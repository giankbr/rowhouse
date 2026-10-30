//! Pending grid edits → SQL, applied atomically.
//!
//! The UI collects edits into a [`ChangeSet`]. [`Connection::preview_changes`]
//! renders the exact statements for review, and [`Connection::apply_changes`]
//! runs them in one transaction. Every UPDATE / DELETE is keyed by the primary
//! key and must touch exactly one row, otherwise the whole transaction is
//! rolled back — so a stale grid can never clobber more than it shows.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{backend::Expect, config::Driver, Connection, Error, Result};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChangeSet {
    pub schema: String,
    pub table: String,
    #[serde(default)]
    pub updates: Vec<RowUpdate>,
    #[serde(default)]
    pub inserts: Vec<Map<String, Value>>,
    #[serde(default)]
    pub deletes: Vec<Map<String, Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RowUpdate {
    /// Primary-key column → original value.
    pub key: Map<String, Value>,
    /// Column → new value.
    pub set: Map<String, Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Insert,
    Update,
    Delete,
}

impl Connection {
    pub(crate) fn target(&self, schema: &str, table: &str) -> String {
        if self.driver() == Driver::Sqlite {
            self.ident(table)
        } else {
            format!("{}.{}", self.ident(schema), self.ident(table))
        }
    }

    /// JSON value → SQL literal.
    pub(crate) fn sql_value(&self, v: &Value) -> String {
        match v {
            Value::Null => "NULL".into(),
            Value::Bool(b) => match self.driver() {
                Driver::Sqlite | Driver::Mssql | Driver::Clickhouse => {
                    (if *b { "1" } else { "0" }).into()
                }
                _ => (if *b { "TRUE" } else { "FALSE" }).into(),
            },
            Value::Number(n) => n.to_string(),
            Value::String(s) => self.literal(s),
            other => self.literal(&other.to_string()),
        }
    }

    fn where_key(&self, key: &Map<String, Value>) -> Result<String> {
        if key.is_empty() {
            return Err(Error::Other(
                "row has no primary key; refusing to edit".into(),
            ));
        }
        Ok(key
            .iter()
            .map(|(k, v)| match v {
                Value::Null => format!("{} IS NULL", self.ident(k)),
                _ => format!("{} = {}", self.ident(k), self.sql_value(v)),
            })
            .collect::<Vec<_>>()
            .join(" AND "))
    }

    fn statements(&self, cs: &ChangeSet) -> Result<Vec<(Kind, String)>> {
        let t = self.target(&cs.schema, &cs.table);
        let mut out = Vec::new();
        for u in &cs.updates {
            if u.set.is_empty() {
                continue;
            }
            let set = u
                .set
                .iter()
                .map(|(k, v)| format!("{} = {}", self.ident(k), self.sql_value(v)))
                .collect::<Vec<_>>()
                .join(", ");
            out.push((
                Kind::Update,
                format!("UPDATE {t} SET {set} WHERE {}", self.where_key(&u.key)?),
            ));
        }
        for row in &cs.inserts {
            if row.is_empty() {
                out.push((Kind::Insert, format!("INSERT INTO {t} DEFAULT VALUES")));
                continue;
            }
            let cols = row
                .keys()
                .map(|k| self.ident(k))
                .collect::<Vec<_>>()
                .join(", ");
            let vals = row
                .values()
                .map(|v| self.sql_value(v))
                .collect::<Vec<_>>()
                .join(", ");
            out.push((
                Kind::Insert,
                format!("INSERT INTO {t} ({cols}) VALUES ({vals})"),
            ));
        }
        for key in &cs.deletes {
            out.push((
                Kind::Delete,
                format!("DELETE FROM {t} WHERE {}", self.where_key(key)?),
            ));
        }
        Ok(out)
    }

    /// The exact SQL that [`Self::apply_changes`] will run.
    pub fn preview_changes(&self, cs: &ChangeSet) -> Result<Vec<String>> {
        if let Some(m) = self.mongo() {
            return m.preview(cs);
        }
        Ok(self.statements(cs)?.into_iter().map(|(_, s)| s).collect())
    }

    /// Apply everything in one transaction. Returns total rows affected.
    pub async fn apply_changes(&self, cs: &ChangeSet) -> Result<u64> {
        if self.config.read_only {
            return Err(Error::ReadOnly("connection is read-only".into()));
        }
        if let Some(m) = self.mongo() {
            return m.apply(cs).await;
        }
        // MySQL reports 0 for an UPDATE that sets identical values; accept that.
        let lenient = self.driver() == Driver::Mysql;
        let stmts = self
            .statements(cs)?
            .into_iter()
            .map(|(kind, sql)| {
                let expect = match kind {
                    Kind::Insert => Expect::Any,
                    Kind::Update if lenient => Expect::AtMostOne,
                    _ => Expect::One,
                };
                (expect, sql)
            })
            .collect();
        self.exec_tx(stmts).await
    }
}
