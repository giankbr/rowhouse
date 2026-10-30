//! Table structure editing: column add / rename / change / drop as DDL in the
//! connection's dialect, previewed first and then applied together.

use serde::{Deserialize, Serialize};

use crate::{
    backend::Expect, compare::Change, config::Driver, ColumnInfo, Connection, Error, Result,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "op")]
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
    AddIndex {
        name: String,
        columns: Vec<String>,
        #[serde(default)]
        unique: bool,
    },
    DropIndex {
        name: String,
    },
    AddForeignKey {
        name: String,
        columns: Vec<String>,
        ref_schema: String,
        ref_table: String,
        ref_columns: Vec<String>,
        /// CASCADE / SET NULL / RESTRICT / NO ACTION
        #[serde(default)]
        on_delete: Option<String>,
    },
    DropForeignKey {
        name: String,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexInfo {
    pub name: String,
    pub columns: Vec<String>,
    pub unique: bool,
    pub primary: bool,
    /// engine's own definition when it has one (Postgres indexdef, DuckDB sql)
    pub definition: Option<String>,
}

fn cell(v: Option<&serde_json::Value>) -> String {
    match v {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Null) | None => String::new(),
        Some(o) => o.to_string(),
    }
}
fn truthy(v: Option<&serde_json::Value>) -> bool {
    matches!(cell(v).to_lowercase().as_str(), "1" | "true" | "t" | "yes")
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
                ColumnOp::AddIndex {
                    name,
                    columns,
                    unique,
                } => {
                    if name.trim().is_empty() || columns.is_empty() {
                        return Err(Error::Other(
                            "an index needs a name and at least one column".into(),
                        ));
                    }
                    let cols: Vec<String> = columns.iter().map(|c| self.ident(c)).collect();
                    let u = if *unique { "UNIQUE " } else { "" };
                    out.push(format!(
                        "CREATE {u}INDEX {} ON {t} ({})",
                        self.ident(name),
                        cols.join(", ")
                    ));
                }
                ColumnOp::DropIndex { name } => out.push(match d {
                    Driver::Mysql | Driver::Mssql => {
                        format!("DROP INDEX {} ON {t}", self.ident(name))
                    }
                    Driver::Sqlite => format!("DROP INDEX {}", self.ident(name)),
                    _ => format!("DROP INDEX {}.{}", self.ident(schema), self.ident(name)),
                }),
                ColumnOp::AddForeignKey {
                    name,
                    columns,
                    ref_schema,
                    ref_table,
                    ref_columns,
                    on_delete,
                } => {
                    if d == Driver::Sqlite {
                        out.push(format!(
                            "-- SQLite can't add a foreign key to an existing table; recreate {t} with it"
                        ));
                        continue;
                    }
                    if columns.is_empty() || columns.len() != ref_columns.len() {
                        return Err(Error::Other(
                            "pick the same number of columns on both sides".into(),
                        ));
                    }
                    let list = |cs: &[String]| {
                        cs.iter()
                            .map(|c| self.ident(c))
                            .collect::<Vec<_>>()
                            .join(", ")
                    };
                    let action = on_delete
                        .as_deref()
                        .map(str::trim)
                        .filter(|a| {
                            [
                                "CASCADE",
                                "SET NULL",
                                "RESTRICT",
                                "NO ACTION",
                                "SET DEFAULT",
                            ]
                            .contains(&a.to_uppercase().as_str())
                        })
                        .map(|a| format!(" ON DELETE {}", a.to_uppercase()))
                        .unwrap_or_default();
                    out.push(format!(
                        "ALTER TABLE {t} ADD CONSTRAINT {} FOREIGN KEY ({}) REFERENCES {} ({}){action}",
                        self.ident(name),
                        list(columns),
                        self.target(ref_schema, ref_table),
                        list(ref_columns)
                    ));
                }
                ColumnOp::DropForeignKey { name } => out.push(match d {
                    Driver::Mysql => {
                        format!("ALTER TABLE {t} DROP FOREIGN KEY {}", self.ident(name))
                    }
                    Driver::Sqlite => format!(
                        "-- SQLite can't drop a foreign key in place; recreate {t} without it"
                    ),
                    _ => format!("ALTER TABLE {t} DROP CONSTRAINT {}", self.ident(name)),
                }),
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

    pub async fn list_indexes(&self, schema: &str, table: &str) -> Result<Vec<IndexInfo>> {
        let (s, t) = (self.literal(schema), self.literal(table));
        let rows = match self.driver() {
            Driver::Postgres => {
                let sql = format!(
                    "SELECT i.relname, ix.indisunique, ix.indisprimary, \
                       array_to_string(ARRAY(SELECT a.attname FROM unnest(ix.indkey) WITH ORDINALITY k(n, o) \
                         JOIN pg_attribute a ON a.attrelid = ix.indrelid AND a.attnum = k.n ORDER BY k.o), ','), \
                       pg_get_indexdef(ix.indexrelid) \
                     FROM pg_index ix JOIN pg_class i ON i.oid = ix.indexrelid \
                     JOIN pg_class c ON c.oid = ix.indrelid JOIN pg_namespace n ON n.oid = c.relnamespace \
                     WHERE n.nspname = {s} AND c.relname = {t} ORDER BY ix.indisprimary DESC, 1"
                );
                self.query_admin(&sql).await?.results.remove(0).rows
            }
            Driver::Mysql => {
                let sql = format!(
                    "SELECT index_name, MIN(non_unique) = 0, index_name = 'PRIMARY', \
                       GROUP_CONCAT(column_name ORDER BY seq_in_index), NULL \
                     FROM information_schema.statistics WHERE table_schema = {s} AND table_name = {t} \
                     GROUP BY index_name ORDER BY index_name = 'PRIMARY' DESC, index_name"
                );
                self.query_admin(&sql).await?.results.remove(0).rows
            }
            Driver::Mssql => {
                let sql = format!(
                    "SELECT i.name, i.is_unique, i.is_primary_key, \
                       STUFF((SELECT ',' + c.name FROM sys.index_columns ic JOIN sys.columns c \
                         ON c.object_id = ic.object_id AND c.column_id = ic.column_id \
                         WHERE ic.object_id = i.object_id AND ic.index_id = i.index_id ORDER BY ic.key_ordinal \
                         FOR XML PATH('')), 1, 1, ''), NULL \
                     FROM sys.indexes i WHERE i.object_id = OBJECT_ID({}) AND i.name IS NOT NULL",
                    self.literal(&format!("{schema}.{table}"))
                );
                self.query_admin(&sql).await?.results.remove(0).rows
            }
            Driver::Sqlite => {
                let list = self
                    .query_admin(&format!("PRAGMA index_list({})", self.ident(table)))
                    .await?
                    .results
                    .remove(0);
                let pos = |n: &str| list.columns.iter().position(|c| c.name == n);
                let (ni, ui, oi) = (
                    pos("name").unwrap_or(1),
                    pos("unique").unwrap_or(2),
                    pos("origin"),
                );
                let mut rows = Vec::new();
                for r in &list.rows {
                    let name = cell(r.get(ni));
                    let cols = self
                        .query_admin(&format!("PRAGMA index_info({})", self.ident(&name)))
                        .await?
                        .results
                        .remove(0);
                    let ci = cols
                        .columns
                        .iter()
                        .position(|c| c.name == "name")
                        .unwrap_or(2);
                    let names: Vec<String> = cols.rows.iter().map(|x| cell(x.get(ci))).collect();
                    let primary = oi.map(|i| cell(r.get(i)) == "pk").unwrap_or(false);
                    rows.push(vec![
                        name.into(),
                        r.get(ui).cloned().unwrap_or_default(),
                        primary.into(),
                        names.join(",").into(),
                        serde_json::Value::Null,
                    ]);
                }
                rows
            }
            Driver::Duckdb => {
                let sql = format!(
                    "SELECT index_name, is_unique, is_primary, expressions, sql FROM duckdb_indexes() \
                     WHERE schema_name = {s} AND table_name = {t}"
                );
                self.query_admin(&sql).await?.results.remove(0).rows
            }
            _ => vec![],
        };
        Ok(rows
            .into_iter()
            .map(|r| IndexInfo {
                name: cell(r.first()),
                unique: truthy(r.get(1)),
                primary: truthy(r.get(2)),
                columns: cell(r.get(3))
                    .trim_matches(|c| c == '[' || c == ']')
                    .split(',')
                    .map(|x| x.trim().trim_matches('"').to_string())
                    .filter(|x| !x.is_empty())
                    .collect(),
                definition: Some(cell(r.get(4))).filter(|d| !d.is_empty()),
            })
            .collect())
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
