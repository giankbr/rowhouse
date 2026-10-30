//! Schema compare: what has to change in a *target* schema so it matches a
//! *source* schema, plus a reviewable migration script in the target's dialect.
//!
//! Drops are always emitted commented-out — the user opts in explicitly.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::{config::Driver, ColumnInfo, Connection, Result};

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Change {
    /// table exists only in the source
    TableAdded {
        table: String,
        columns: Vec<ColumnInfo>,
    },
    /// table exists only in the target
    TableRemoved {
        table: String,
    },
    ColumnAdded {
        table: String,
        column: ColumnInfo,
    },
    ColumnRemoved {
        table: String,
        column: String,
    },
    ColumnChanged {
        table: String,
        column: String,
        from: ColumnInfo,
        to: ColumnInfo,
        /// which aspects differ: "type", "nullable", "default"
        aspects: Vec<String>,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaDiff {
    pub changes: Vec<Change>,
    /// tables present on both sides with no differences
    pub identical: Vec<String>,
    /// migration for the target, in its dialect
    pub sql: String,
    /// source and target use different engines: type names may not line up
    pub cross_dialect: bool,
}

type Tables = BTreeMap<String, Vec<ColumnInfo>>;

async fn snapshot(c: &Connection, schema: &str) -> Result<Tables> {
    let mut out = Tables::new();
    for t in c.list_tables().await? {
        if t.kind != "table" || (!schema.is_empty() && t.schema != schema) {
            continue;
        }
        let cols = c.describe_table(&t.schema, &t.name).await?;
        out.insert(t.name, cols);
    }
    Ok(out)
}

fn norm_type(t: &str) -> String {
    let t = t.trim().to_lowercase();
    // spelling variants that mean the same thing
    let t = t
        .replace("character varying", "varchar")
        .replace("timestamp without time zone", "timestamp")
        .replace("timestamp with time zone", "timestamptz")
        .replace("integer", "int")
        .replace("int4", "int")
        .replace("int8", "bigint")
        .replace("bool", "boolean")
        .replace("booleanean", "boolean");
    t.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn norm_default(d: &Option<String>) -> Option<String> {
    d.as_ref()
        .map(|s| {
            s.trim()
                .trim_start_matches('(')
                .trim_end_matches(')')
                .to_lowercase()
        })
        .filter(|s| !s.is_empty() && s != "null")
}

/// Pure diff of two snapshots (exposed for tests).
pub fn diff(source: &Tables, target: &Tables) -> (Vec<Change>, Vec<String>) {
    let mut changes = Vec::new();
    let mut identical = Vec::new();
    for (name, cols) in source {
        let Some(tcols) = target.get(name) else {
            changes.push(Change::TableAdded {
                table: name.clone(),
                columns: cols.clone(),
            });
            continue;
        };
        let before = changes.len();
        for c in cols {
            match tcols.iter().find(|t| t.name == c.name) {
                None => changes.push(Change::ColumnAdded {
                    table: name.clone(),
                    column: c.clone(),
                }),
                Some(t) => {
                    let mut aspects = Vec::new();
                    if norm_type(&c.data_type) != norm_type(&t.data_type) {
                        aspects.push("type".to_string());
                    }
                    if c.nullable != t.nullable {
                        aspects.push("nullable".to_string());
                    }
                    if norm_default(&c.default) != norm_default(&t.default) {
                        aspects.push("default".to_string());
                    }
                    if !aspects.is_empty() {
                        changes.push(Change::ColumnChanged {
                            table: name.clone(),
                            column: c.name.clone(),
                            from: t.clone(),
                            to: c.clone(),
                            aspects,
                        });
                    }
                }
            }
        }
        for t in tcols {
            if !cols.iter().any(|c| c.name == t.name) {
                changes.push(Change::ColumnRemoved {
                    table: name.clone(),
                    column: t.name.clone(),
                });
            }
        }
        if changes.len() == before {
            identical.push(name.clone());
        }
    }
    for name in target.keys() {
        if !source.contains_key(name) {
            changes.push(Change::TableRemoved {
                table: name.clone(),
            });
        }
    }
    (changes, identical)
}

impl Connection {
    fn column_def(&self, c: &ColumnInfo) -> String {
        let mut s = format!("{} {}", self.ident(&c.name), c.data_type);
        if !c.nullable {
            s.push_str(" NOT NULL");
        }
        if let Some(d) = &c.default {
            s.push_str(&format!(" DEFAULT {d}"));
        }
        s
    }

    /// Migration script (target dialect) for `changes`.
    pub fn migration_sql(&self, schema: &str, changes: &[Change]) -> String {
        let t = |name: &str| self.target(schema, name);
        let d = self.driver();
        let mut out = vec![
            "-- Generated by Rowhouse schema compare. Review before running.".to_string(),
            "-- Drops are commented out on purpose.".to_string(),
        ];
        for ch in changes {
            match ch {
                Change::TableAdded { table, columns } => {
                    let mut defs: Vec<String> = columns
                        .iter()
                        .map(|c| format!("  {}", self.column_def(c)))
                        .collect();
                    let pk: Vec<String> = columns
                        .iter()
                        .filter(|c| c.primary_key)
                        .map(|c| self.ident(&c.name))
                        .collect();
                    if !pk.is_empty() {
                        defs.push(format!("  PRIMARY KEY ({})", pk.join(", ")));
                    }
                    let engine = if d == Driver::Clickhouse {
                        format!(
                            " ENGINE = MergeTree ORDER BY ({})",
                            if pk.is_empty() {
                                "tuple()".into()
                            } else {
                                pk.join(", ")
                            }
                        )
                    } else {
                        String::new()
                    };
                    out.push(format!(
                        "CREATE TABLE {} (\n{}\n){engine};",
                        t(table),
                        defs.join(",\n")
                    ));
                }
                Change::TableRemoved { table } => out.push(format!("-- DROP TABLE {};", t(table))),
                Change::ColumnAdded { table, column } => {
                    let kw = if d == Driver::Mssql {
                        "ADD"
                    } else {
                        "ADD COLUMN"
                    };
                    out.push(format!(
                        "ALTER TABLE {} {kw} {};",
                        t(table),
                        self.column_def(column)
                    ));
                }
                Change::ColumnRemoved { table, column } => {
                    out.push(format!(
                        "-- ALTER TABLE {} DROP COLUMN {};",
                        t(table),
                        self.ident(column)
                    ));
                }
                Change::ColumnChanged {
                    table,
                    column,
                    to,
                    aspects,
                    ..
                } => {
                    let (tb, col) = (t(table), self.ident(column));
                    let has = |a: &str| aspects.iter().any(|x| x == a);
                    match d {
                        Driver::Postgres | Driver::Duckdb => {
                            if has("type") {
                                out.push(format!("ALTER TABLE {tb} ALTER COLUMN {col} TYPE {} USING {col}::{};", to.data_type, to.data_type));
                            }
                            if has("nullable") {
                                let act = if to.nullable { "DROP NOT NULL" } else { "SET NOT NULL" };
                                out.push(format!("ALTER TABLE {tb} ALTER COLUMN {col} {act};"));
                            }
                            if has("default") {
                                out.push(match &to.default {
                                    Some(v) => format!("ALTER TABLE {tb} ALTER COLUMN {col} SET DEFAULT {v};"),
                                    None => format!("ALTER TABLE {tb} ALTER COLUMN {col} DROP DEFAULT;"),
                                });
                            }
                        }
                        Driver::Mysql => out.push(format!("ALTER TABLE {tb} MODIFY COLUMN {};", self.column_def(to))),
                        Driver::Clickhouse => out.push(format!("ALTER TABLE {tb} MODIFY COLUMN {col} {};", to.data_type)),
                        Driver::Mssql => {
                            if has("type") || has("nullable") {
                                let null = if to.nullable { "NULL" } else { "NOT NULL" };
                                out.push(format!("ALTER TABLE {tb} ALTER COLUMN {col} {} {null};", to.data_type));
                            }
                            if has("default") {
                                out.push(format!("-- default of {col} differs ({:?}); SQL Server defaults are named constraints — change by hand", to.default));
                            }
                        }
                        Driver::Redis | Driver::Mongo => out.push("-- no column schema to alter".into()),
                        Driver::Cassandra => out.push(format!("-- CQL can't change {col} in place ({}); add a new column and copy the data", aspects.join(", "))),
                        Driver::Sqlite => out.push(format!(
                            "-- SQLite can't alter {col} in place ({}); recreate {tb} with the new definition",
                            aspects.join(", ")
                        )),
                    }
                }
            }
        }
        if out.len() == 2 {
            out.push("-- Schemas match. Nothing to do.".into());
        }
        out.join("\n")
    }

    /// Compare `source_schema` on `source` with `target_schema` on `self`.
    pub async fn compare_schema(
        &self,
        target_schema: &str,
        source: &Connection,
        source_schema: &str,
    ) -> Result<SchemaDiff> {
        let a = snapshot(source, source_schema).await?;
        let b = snapshot(self, target_schema).await?;
        let (changes, identical) = diff(&a, &b);
        Ok(SchemaDiff {
            sql: self.migration_sql(target_schema, &changes),
            changes,
            identical,
            cross_dialect: source.driver() != self.driver(),
        })
    }
}

// ------------------------------------------------------------- data compare ---

/// Rows fetched per side at most.
pub const MAX_DATA_ROWS: usize = 200_000;
/// Example rows returned per category for the UI.
const SAMPLES: usize = 200;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RowDiff {
    pub key: serde_json::Map<String, serde_json::Value>,
    /// column → (target value, source value); only for changed rows
    pub changes: BTreeMap<String, (serde_json::Value, serde_json::Value)>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DataDiff {
    pub key_columns: Vec<String>,
    pub compared_columns: Vec<String>,
    pub source_rows: usize,
    pub target_rows: usize,
    pub only_in_source: usize,
    pub only_in_target: usize,
    pub changed: usize,
    pub identical: usize,
    pub samples_only_in_source: Vec<RowDiff>,
    pub samples_only_in_target: Vec<RowDiff>,
    pub samples_changed: Vec<RowDiff>,
    /// a side hit MAX_DATA_ROWS; results cover only the first rows by key
    pub truncated: bool,
    /// statements that make the target match the source (deletes commented out)
    pub sql: String,
}

fn canon(v: &serde_json::Value) -> String {
    use serde_json::Value::*;
    match v {
        Null => "\u{0}null".into(),
        Bool(b) => (if *b { "1" } else { "0" }).into(),
        Number(n) => n
            .as_f64()
            .map(|f| f.to_string())
            .unwrap_or_else(|| n.to_string()),
        String(s) => s
            .trim()
            .parse::<f64>()
            .map(|f| f.to_string())
            .unwrap_or_else(|_| s.clone()),
        other => other.to_string(),
    }
}

async fn fetch_keyed(
    c: &Connection,
    schema: &str,
    table: &str,
    keys: &[String],
) -> Result<(Vec<String>, Vec<Vec<serde_json::Value>>, bool)> {
    let order = keys
        .iter()
        .map(|k| c.ident(k))
        .collect::<Vec<_>>()
        .join(", ");
    let t = c.target(schema, table);
    let sql = if c.driver() == Driver::Mssql {
        format!(
            "SELECT TOP {} * FROM {t} ORDER BY {order}",
            MAX_DATA_ROWS + 1
        )
    } else {
        format!(
            "SELECT * FROM {t} ORDER BY {order} LIMIT {}",
            MAX_DATA_ROWS + 1
        )
    };
    let r = c
        .query(&sql, MAX_DATA_ROWS + 1)
        .await?
        .results
        .into_iter()
        .next()
        .unwrap_or_default();
    let cols: Vec<String> = if r.columns.is_empty() {
        c.describe_table(schema, table)
            .await?
            .into_iter()
            .map(|c| c.name)
            .collect()
    } else {
        r.columns.into_iter().map(|c| c.name).collect()
    };
    let mut rows = r.rows;
    let truncated = rows.len() > MAX_DATA_ROWS;
    rows.truncate(MAX_DATA_ROWS);
    Ok((cols, rows, truncated))
}

impl Connection {
    /// Compare rows of `source_table` (on `source`) with `target_table` (on `self`), matched by
    /// the target's primary key.
    pub async fn compare_data(
        &self,
        target_schema: &str,
        target_table: &str,
        source: &Connection,
        source_schema: &str,
        source_table: &str,
    ) -> Result<DataDiff> {
        use crate::changes::{ChangeSet, RowUpdate};
        use serde_json::{Map, Value};

        let keys: Vec<String> = self
            .describe_table(target_schema, target_table)
            .await?
            .into_iter()
            .filter(|c| c.primary_key)
            .map(|c| c.name)
            .collect();
        if keys.is_empty() {
            return Err(crate::Error::Other(format!(
                "{target_table} has no primary key — rows can't be matched"
            )));
        }
        let (scols, srows, st) = fetch_keyed(source, source_schema, source_table, &keys).await?;
        let (tcols, trows, tt) = fetch_keyed(self, target_schema, target_table, &keys).await?;
        if let Some(k) = keys.iter().find(|k| !scols.contains(k)) {
            return Err(crate::Error::Other(format!(
                "key column `{k}` is missing in the source table"
            )));
        }
        let common: Vec<String> = scols
            .iter()
            .filter(|c| tcols.contains(c))
            .cloned()
            .collect();
        let idx = |cols: &[String], name: &str| cols.iter().position(|c| c == name).unwrap();
        let key_of = |cols: &[String], row: &[Value]| -> String {
            keys.iter()
                .map(|k| canon(&row[idx(cols, k)]))
                .collect::<Vec<_>>()
                .join("\u{1}")
        };
        let key_map = |cols: &[String], row: &[Value]| -> Map<String, Value> {
            keys.iter()
                .map(|k| (k.clone(), row[idx(cols, k)].clone()))
                .collect()
        };

        let mut target_by_key = std::collections::HashMap::with_capacity(trows.len());
        for r in &trows {
            target_by_key.insert(key_of(&tcols, r), r);
        }
        let mut seen = std::collections::HashSet::with_capacity(srows.len());
        let mut cs = ChangeSet {
            schema: target_schema.into(),
            table: target_table.into(),
            ..Default::default()
        };
        let (mut add_s, mut del_s, mut chg_s) = (vec![], vec![], vec![]);
        let mut identical = 0;

        for s in &srows {
            let k = key_of(&scols, s);
            seen.insert(k.clone());
            match target_by_key.get(&k) {
                None => {
                    let row: Map<String, Value> = common
                        .iter()
                        .map(|c| (c.clone(), s[idx(&scols, c)].clone()))
                        .collect();
                    if add_s.len() < SAMPLES {
                        add_s.push(RowDiff {
                            key: key_map(&scols, s),
                            changes: BTreeMap::new(),
                        });
                    }
                    cs.inserts.push(row);
                }
                Some(t) => {
                    let mut changes = BTreeMap::new();
                    for c in common.iter().filter(|c| !keys.contains(c)) {
                        let (sv, tv) = (&s[idx(&scols, c)], &t[idx(&tcols, c)]);
                        if canon(sv) != canon(tv) {
                            changes.insert(c.clone(), (tv.clone(), sv.clone()));
                        }
                    }
                    if changes.is_empty() {
                        identical += 1;
                    } else {
                        cs.updates.push(RowUpdate {
                            key: key_map(&scols, s),
                            set: changes
                                .iter()
                                .map(|(c, (_, sv))| (c.clone(), sv.clone()))
                                .collect(),
                        });
                        if chg_s.len() < SAMPLES {
                            chg_s.push(RowDiff {
                                key: key_map(&scols, s),
                                changes,
                            });
                        }
                    }
                }
            }
        }
        for t in &trows {
            if !seen.contains(&key_of(&tcols, t)) {
                if del_s.len() < SAMPLES {
                    del_s.push(RowDiff {
                        key: key_map(&tcols, t),
                        changes: BTreeMap::new(),
                    });
                }
                cs.deletes.push(key_map(&tcols, t));
            }
        }

        let mut lines = vec![
            format!("-- Generated by Rowhouse data compare: make {target_table} match the source."),
            "-- Deletes are commented out on purpose. Review before running.".to_string(),
        ];
        for stmt in self.preview_changes(&cs)? {
            if stmt.starts_with("DELETE") {
                lines.push(format!("-- {stmt};"));
            } else {
                lines.push(format!("{stmt};"));
            }
        }
        Ok(DataDiff {
            key_columns: keys,
            compared_columns: common,
            source_rows: srows.len(),
            target_rows: trows.len(),
            only_in_source: cs.inserts.len(),
            only_in_target: cs.deletes.len(),
            changed: cs.updates.len(),
            identical,
            samples_only_in_source: add_s,
            samples_only_in_target: del_s,
            samples_changed: chg_s,
            truncated: st || tt,
            sql: lines.join("\n"),
        })
    }
}
