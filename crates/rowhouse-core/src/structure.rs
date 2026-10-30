//! Table structure editing: column add / rename / change / drop as DDL in the
//! connection's dialect, previewed first and then applied together.

use serde::{Deserialize, Serialize};

use crate::{
    backend::Expect, compare::Change, config::Driver, ColumnInfo, Connection, Error, Result,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "op")]
pub enum ColumnOp {
    Add {
        column: ColumnInfo,
    },
    Drop {
        name: String,
    },
    Rename {
        from: String,
        to: String,
    },
    /// `from` is the current definition, `to` the wanted one (name unchanged).
    Alter {
        from: ColumnInfo,
        to: ColumnInfo,
    },
}

impl Connection {
    /// DDL for the operations, in order. Lines starting with `--` are notes
    /// (e.g. things SQLite can't do in place) and are not executed.
    pub fn structure_sql(
        &self,
        schema: &str,
        table: &str,
        ops: &[ColumnOp],
    ) -> Result<Vec<String>> {
        let t = self.target(schema, table);
        let d = self.driver();
        let mut out = Vec::new();
        for op in ops {
            match op {
                ColumnOp::Add { column } => {
                    if column.name.trim().is_empty() || column.data_type.trim().is_empty() {
                        return Err(Error::Other("a new column needs a name and a type".into()));
                    }
                    let kw = if d == Driver::Mssql {
                        "ADD"
                    } else {
                        "ADD COLUMN"
                    };
                    let mut def = format!("{} {}", self.ident(&column.name), column.data_type);
                    if !column.nullable {
                        def.push_str(" NOT NULL");
                    }
                    if let Some(v) = column.default.as_deref().filter(|v| !v.trim().is_empty()) {
                        def.push_str(&format!(" DEFAULT {v}"));
                    }
                    out.push(format!("ALTER TABLE {t} {kw} {def}"));
                }
                ColumnOp::Drop { name } => {
                    out.push(format!("ALTER TABLE {t} DROP COLUMN {}", self.ident(name)))
                }
                ColumnOp::Rename { from, to } => out.push(match d {
                    Driver::Mssql => {
                        let obj = format!("{schema}.{table}.{from}").replace('\'', "''");
                        format!(
                            "EXEC sp_rename N'{obj}', N'{}', N'COLUMN'",
                            to.replace('\'', "''")
                        )
                    }
                    _ => format!(
                        "ALTER TABLE {t} RENAME COLUMN {} TO {}",
                        self.ident(from),
                        self.ident(to)
                    ),
                }),
                ColumnOp::Alter { from, to } => {
                    let mut aspects = Vec::new();
                    if from.data_type.trim() != to.data_type.trim() {
                        aspects.push("type".to_string());
                    }
                    if from.nullable != to.nullable {
                        aspects.push("nullable".to_string());
                    }
                    let dflt = |c: &ColumnInfo| c.default.clone().filter(|v| !v.trim().is_empty());
                    if dflt(from) != dflt(to) {
                        aspects.push("default".to_string());
                    }
                    if aspects.is_empty() {
                        continue;
                    }
                    let to = ColumnInfo {
                        default: dflt(to),
                        ..to.clone()
                    };
                    let change = Change::ColumnChanged {
                        table: table.into(),
                        column: to.name.clone(),
                        from: from.clone(),
                        to,
                        aspects,
                    };
                    // reuse the dialect rules from schema compare, minus its header
                    for line in self.migration_sql(schema, &[change]).lines().skip(2) {
                        out.push(line.trim_end_matches(';').to_string());
                    }
                }
            }
        }
        Ok(out)
    }

    /// Run the DDL (in one transaction where the engine supports transactional DDL).
    pub async fn apply_structure(
        &self,
        schema: &str,
        table: &str,
        ops: &[ColumnOp],
    ) -> Result<Vec<String>> {
        if self.config.read_only {
            return Err(Error::ReadOnly("connection is read-only".into()));
        }
        let stmts = self.structure_sql(schema, table, ops)?;
        if let Some(note) = stmts.iter().find(|s| s.starts_with("--")) {
            return Err(Error::Other(format!(
                "can't apply as-is: {}",
                note.trim_start_matches("- ").trim_start_matches('-').trim()
            )));
        }
        self.exec_tx(stmts.iter().map(|s| (Expect::Any, s.clone())).collect())
            .await?;
        Ok(stmts)
    }
}
