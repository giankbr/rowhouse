//! Schema snapshot for the ER diagram: tables, columns and foreign keys.

use serde::Serialize;
use serde_json::Value;

use crate::{config::Driver, ColumnInfo, Connection, Result};

/// Above this many tables the diagram stops being useful; the UI asks for a schema filter.
pub const MAX_ERD_TABLES: usize = 250;

#[derive(Debug, Clone, Serialize)]
pub struct ForeignKey {
    pub name: String,
    pub schema: String,
    pub table: String,
    pub columns: Vec<String>,
    pub ref_schema: String,
    pub ref_table: String,
    pub ref_columns: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ErdTable {
    pub schema: String,
    pub name: String,
    pub kind: String,
    pub columns: Vec<ColumnInfo>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Erd {
    pub tables: Vec<ErdTable>,
    pub foreign_keys: Vec<ForeignKey>,
    /// true when the table list was cut at [`MAX_ERD_TABLES`]
    pub truncated: bool,
}

fn s(v: &Value) -> String {
    match v {
        Value::String(x) => x.clone(),
        Value::Null => String::new(),
        o => o.to_string(),
    }
}

impl Connection {
    /// Foreign keys as (fk name, schema, table, column, ref schema, ref table, ref column, position) rows,
    /// grouped into multi-column keys.
    pub async fn foreign_keys(&self) -> Result<Vec<ForeignKey>> {
        let sql = match self.driver() {
            Driver::Postgres => Some(
                "SELECT c.conname, ns.nspname, cl.relname, a.attname, rns.nspname, rcl.relname, ra.attname, k.ord
                 FROM pg_constraint c
                 JOIN pg_class cl ON cl.oid = c.conrelid JOIN pg_namespace ns ON ns.oid = cl.relnamespace
                 JOIN pg_class rcl ON rcl.oid = c.confrelid JOIN pg_namespace rns ON rns.oid = rcl.relnamespace
                 CROSS JOIN LATERAL unnest(c.conkey, c.confkey) WITH ORDINALITY AS k(attnum, rattnum, ord)
                 JOIN pg_attribute a ON a.attrelid = c.conrelid AND a.attnum = k.attnum
                 JOIN pg_attribute ra ON ra.attrelid = c.confrelid AND ra.attnum = k.rattnum
                 WHERE c.contype = 'f' AND ns.nspname NOT IN ('pg_catalog','information_schema')
                 ORDER BY 1, 2, 3, 8"
                    .to_string(),
            ),
            Driver::Mysql => Some(
                "SELECT constraint_name, table_schema, table_name, column_name,
                        referenced_table_schema, referenced_table_name, referenced_column_name, ordinal_position
                 FROM information_schema.key_column_usage
                 WHERE referenced_table_name IS NOT NULL
                   AND (table_schema = DATABASE() OR (DATABASE() IS NULL AND LOWER(table_schema) NOT IN
                        ('mysql','information_schema','performance_schema','sys')))
                 ORDER BY 1, 2, 3, 8"
                    .to_string(),
            ),
            Driver::Sqlite => Some(
                "SELECT 'fk_' || m.name || '_' || p.id, 'main', m.name, p.\"from\", 'main', p.\"table\",
                        COALESCE(p.\"to\", ''), p.seq
                 FROM sqlite_master m JOIN pragma_foreign_key_list(m.name) p
                 WHERE m.type = 'table' ORDER BY 1, 8"
                    .to_string(),
            ),
            Driver::Mssql => Some(
                "SELECT fk.name, s.name, t.name, c.name, rs.name, rt.name, rc.name, fkc.constraint_column_id
                 FROM sys.foreign_keys fk
                 JOIN sys.foreign_key_columns fkc ON fkc.constraint_object_id = fk.object_id
                 JOIN sys.tables t ON t.object_id = fk.parent_object_id JOIN sys.schemas s ON s.schema_id = t.schema_id
                 JOIN sys.columns c ON c.object_id = t.object_id AND c.column_id = fkc.parent_column_id
                 JOIN sys.tables rt ON rt.object_id = fk.referenced_object_id JOIN sys.schemas rs ON rs.schema_id = rt.schema_id
                 JOIN sys.columns rc ON rc.object_id = rt.object_id AND rc.column_id = fkc.referenced_column_id
                 ORDER BY 1, 8"
                    .to_string(),
            ),
            Driver::Oracle => Some(
                "SELECT c.constraint_name, c.owner, c.table_name, cc.column_name, r.owner, r.table_name, rc.column_name, cc.position
                 FROM all_constraints c
                 JOIN all_cons_columns cc ON cc.owner = c.owner AND cc.constraint_name = c.constraint_name
                 JOIN all_constraints r ON r.owner = c.r_owner AND r.constraint_name = c.r_constraint_name
                 JOIN all_cons_columns rc ON rc.owner = r.owner AND rc.constraint_name = r.constraint_name AND rc.position = cc.position
                 WHERE c.constraint_type = 'R' AND c.owner IN (SELECT username FROM all_users WHERE oracle_maintained = 'N')
                 ORDER BY 1, 8"
                    .to_string(),
            ),
            Driver::Duckdb => Some(
                // one row per constraint; columns arrive as lists
                "SELECT constraint_name, schema_name, table_name, constraint_column_names,
                        schema_name, referenced_table, referenced_column_names, 0
                 FROM duckdb_constraints() WHERE constraint_type = 'FOREIGN KEY'"
                    .to_string(),
            ),
            Driver::Clickhouse | Driver::Redis | Driver::Mongo | Driver::Cassandra => None, // no foreign keys
        };
        let Some(sql) = sql else { return Ok(vec![]) };
        let rows = self
            .query(&sql, usize::MAX)
            .await?
            .results
            .into_iter()
            .next()
            .unwrap_or_default()
            .rows;

        let mut out: Vec<ForeignKey> = Vec::new();
        for r in rows {
            let as_list = |v: &Value| match v {
                Value::Array(a) => a.iter().map(s).collect(),
                other => vec![s(other)],
            };
            let (name, schema, table) = (s(&r[0]), s(&r[1]), s(&r[2]));
            match out.last_mut() {
                Some(fk) if fk.name == name && fk.schema == schema && fk.table == table => {
                    fk.columns.extend(as_list(&r[3]));
                    fk.ref_columns.extend(as_list(&r[6]));
                }
                _ => out.push(ForeignKey {
                    name,
                    schema,
                    table,
                    columns: as_list(&r[3]),
                    ref_schema: s(&r[4]),
                    ref_table: s(&r[5]),
                    ref_columns: as_list(&r[6]),
                }),
            }
        }
        Ok(out)
    }

    /// Tables (optionally one schema) with columns, plus foreign keys between them.
    pub async fn erd(&self, schema: Option<&str>) -> Result<Erd> {
        let mut tables = self.list_tables().await?;
        if let Some(sc) = schema.filter(|s| !s.is_empty()) {
            tables.retain(|t| t.schema == sc);
        }
        let truncated = tables.len() > MAX_ERD_TABLES;
        tables.truncate(MAX_ERD_TABLES);
        let mut out = Vec::with_capacity(tables.len());
        for t in tables {
            let columns = self
                .describe_table(&t.schema, &t.name)
                .await
                .unwrap_or_default();
            out.push(ErdTable {
                schema: t.schema,
                name: t.name,
                kind: t.kind,
                columns,
            });
        }
        let mut fks = self.foreign_keys().await?;
        // SQLite's pragma may omit the referenced column (= the referenced PK)
        for fk in fks.iter_mut() {
            if fk.ref_columns.iter().all(|c| c.is_empty()) {
                if let Some(t) = out.iter().find(|t| t.name == fk.ref_table) {
                    fk.ref_columns = t
                        .columns
                        .iter()
                        .filter(|c| c.primary_key)
                        .map(|c| c.name.clone())
                        .collect();
                }
            }
        }
        fks.retain(|fk| {
            out.iter()
                .any(|t| t.name == fk.table && t.schema == fk.schema)
        });
        Ok(Erd {
            tables: out,
            foreign_keys: fks,
            truncated,
        })
    }
}
