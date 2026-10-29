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
