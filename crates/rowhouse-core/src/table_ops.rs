//! Table-level operations for the sidebar: show DDL, rename, duplicate,
//! truncate, drop, and creating a new table from a column list.

use serde::Deserialize;
use serde_json::Value;

use crate::{
    backend::Expect,
    config::{DbKind, Driver},
    ColumnInfo, Connection, Error, Result,
};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "op")]
pub enum TableAction {
    Rename { new_name: String },
    Duplicate { new_name: String, with_data: bool },
    Truncate,
    Drop,
}

fn cell(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(o) => o.to_string(),
    }
}

fn first_cell(out: crate::QueryOutput, col: usize) -> Option<String> {
    out.results
        .into_iter()
        .next()
        .and_then(|r| r.rows.into_iter().next())
        .map(|row| cell(row.get(col)))
        .filter(|s| !s.trim().is_empty())
}

fn ensure_semicolon(s: &str) -> String {
    let t = s.trim_end();
    if t.ends_with(';') {
        t.to_string()
    } else {
        format!("{t};")
    }
}

/// `CREATE TABLE "x" …` → `CREATE TABLE "new" …` (SQLite keeps the original text).
fn rename_create(sql: &str, new_ident: &str) -> Option<String> {
    let upper = sql.to_ascii_uppercase();
    let mut i = upper.find("TABLE")? + 5;
    let rest = &upper[i..];
    let skip = rest.len() - rest.trim_start().len();
    i += skip;
    if upper[i..].starts_with("IF NOT EXISTS") {
        i += "IF NOT EXISTS".len();
        i += upper[i..].len() - upper[i..].trim_start().len();
    }
    let b = sql.as_bytes();
    let end = match b.get(i)? {
        b'"' | b'`' | b'[' => {
            let close = match b[i] {
                b'[' => b']',
                c => c,
            };
            let mut j = i + 1;
            loop {
                if *b.get(j)? == close {
                    if b.get(j + 1) == Some(&close) && close != b']' {
                        j += 2;
                        continue;
                    }
                    break j + 1;
                }
                j += 1;
            }
        }
        _ => {
            i + sql[i..]
                .find(|c: char| c.is_whitespace() || c == '(')
                .unwrap_or(sql.len() - i)
        }
    };
    Some(format!("{}{}{}", &sql[..i], new_ident, &sql[end..]))
}

impl Connection {
    /// Best-effort `CREATE` statement(s) for a table or view.
    pub async fn table_ddl(&self, schema: &str, table: &str) -> Result<String> {
        let native = self.native_ddl(schema, table).await;
        match native {
            Ok(Some(s)) => Ok(s),
            Ok(None) | Err(_) => match self.driver() {
                Driver::Redis => Err(Error::Other("Redis keys have no DDL".into())),
                Driver::Mongo => self.mongo_ddl(table).await,
                _ => {
                    let cols = self.describe_table(schema, table).await?;
                    if cols.is_empty() {
                        if let Err(e) = native {
                            return Err(e);
                        }
                        return Err(Error::Other(format!("`{table}` not found")));
                    }
                    let idx = self.list_indexes(schema, table).await.unwrap_or_default();
                    let mut out = self.create_table_sql(schema, table, &cols)?;
                    for i in idx.iter().filter(|i| !i.primary) {
                        out.push_str("\n\n");
                        out.push_str(&ensure_semicolon(&i.definition.clone().unwrap_or_else(
                            || {
                                format!(
                                    "CREATE {}INDEX {} ON {} ({})",
                                    if i.unique { "UNIQUE " } else { "" },
                                    self.ident(&i.name),
                                    self.target(schema, table),
                                    i.columns
                                        .iter()
                                        .map(|c| self.ident(c))
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                )
                            },
                        )));
                    }
                    Ok(out)
                }
            },
        }
    }

    async fn native_ddl(&self, schema: &str, table: &str) -> Result<Option<String>> {
        let (s, t) = (self.literal(schema), self.literal(table));
        let target = self.target(schema, table);
        match self.driver() {
            Driver::Postgres if self.kind() == DbKind::Cockroach => Ok(first_cell(
                self.query_admin(&format!("SHOW CREATE TABLE {target}"))
                    .await?,
                1,
            )
            .map(|s| ensure_semicolon(&s))),
            Driver::Postgres => {
                let kind = first_cell(
                    self.query_admin(&format!(
                        "SELECT c.relkind::text FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
                         WHERE n.nspname = {s} AND c.relname = {t}"
                    ))
                    .await?,
                    0,
                );
                let Some(kind) = kind else { return Ok(None) };
                if kind == "v" || kind == "m" {
                    let def = first_cell(
                        self.query_admin(&format!(
                            "SELECT pg_get_viewdef({}::regclass, true)",
                            self.literal(&target)
                        ))
                        .await?,
                        0,
                    )
                    .unwrap_or_default();
                    let head = if kind == "m" {
                        "CREATE MATERIALIZED VIEW"
                    } else {
                        "CREATE OR REPLACE VIEW"
                    };
                    return Ok(Some(format!(
                        "{head} {target} AS\n{}",
                        ensure_semicolon(def.trim())
                    )));
                }
                let sql = format!(
                    "SELECT 'CREATE TABLE ' || quote_ident(n.nspname) || '.' || quote_ident(c.relname) || E' (\\n' || \
                       array_to_string(array( \
                         SELECT '  ' || quote_ident(a.attname) || ' ' || format_type(a.atttypid, a.atttypmod) \
                           || CASE a.attidentity WHEN 'a' THEN ' GENERATED ALWAYS AS IDENTITY' \
                                                 WHEN 'd' THEN ' GENERATED BY DEFAULT AS IDENTITY' ELSE '' END \
                           || CASE WHEN a.attgenerated = 's' THEN ' GENERATED ALWAYS AS (' || pg_get_expr(d.adbin, d.adrelid) || ') STORED' \
                                   WHEN d.adbin IS NOT NULL THEN ' DEFAULT ' || pg_get_expr(d.adbin, d.adrelid) ELSE '' END \
                           || CASE WHEN a.attnotnull THEN ' NOT NULL' ELSE '' END \
                         FROM pg_attribute a LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum \
                         WHERE a.attrelid = c.oid AND a.attnum > 0 AND NOT a.attisdropped ORDER BY a.attnum) \
                       || array( \
                         SELECT '  CONSTRAINT ' || quote_ident(co.conname) || ' ' || pg_get_constraintdef(co.oid) \
                         FROM pg_constraint co WHERE co.conrelid = c.oid AND co.contype IN ('p','u','c','f','x') \
                         ORDER BY CASE co.contype WHEN 'p' THEN 0 WHEN 'u' THEN 1 WHEN 'c' THEN 2 ELSE 3 END, co.conname), \
                       E',\\n') || E'\\n);' \
                     || coalesce((SELECT string_agg(E'\\n\\n' || pg_get_indexdef(ix.indexrelid) || ';', '' ORDER BY i.relname) \
                         FROM pg_index ix JOIN pg_class i ON i.oid = ix.indexrelid \
                         WHERE ix.indrelid = c.oid AND NOT EXISTS (SELECT 1 FROM pg_constraint k WHERE k.conindid = ix.indexrelid)), '') \
                     || coalesce(E'\\n\\nCOMMENT ON TABLE ' || quote_ident(n.nspname) || '.' || quote_ident(c.relname) \
                         || ' IS ' || quote_literal(obj_description(c.oid, 'pg_class')) || ';', '') \
                   FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
                   WHERE n.nspname = {s} AND c.relname = {t}"
                );
                Ok(first_cell(self.query_admin(&sql).await?, 0))
            }
            Driver::Mysql | Driver::Clickhouse => {
                let col = if self.driver() == Driver::Mysql { 1 } else { 0 };
                Ok(first_cell(
                    self.query_admin(&format!("SHOW CREATE TABLE {target}"))
                        .await?,
                    col,
                )
                .map(|s| ensure_semicolon(&s)))
            }
            Driver::Sqlite => {
                let out = self
                    .query_admin(&format!(
                        "SELECT sql FROM sqlite_master WHERE tbl_name = {t} AND sql IS NOT NULL \
                         ORDER BY CASE type WHEN 'table' THEN 0 WHEN 'view' THEN 0 ELSE 1 END, name"
                    ))
                    .await?;
                let parts: Vec<String> = out
                    .results
                    .into_iter()
                    .next()
                    .map(|r| {
                        r.rows
                            .iter()
                            .map(|row| ensure_semicolon(&cell(row.first())))
                            .collect()
                    })
                    .unwrap_or_default();
                Ok((!parts.is_empty()).then(|| parts.join("\n\n")))
            }
            Driver::Duckdb => {
                let out = self
                    .query_admin(&format!(
                        "SELECT sql FROM duckdb_tables() WHERE schema_name = {s} AND table_name = {t} \
                         UNION ALL SELECT sql FROM duckdb_views() WHERE schema_name = {s} AND view_name = {t} \
                         UNION ALL SELECT sql FROM duckdb_indexes() WHERE schema_name = {s} AND table_name = {t} AND sql IS NOT NULL"
                    ))
                    .await?;
                let parts: Vec<String> = out
                    .results
                    .into_iter()
                    .next()
                    .map(|r| {
                        r.rows
                            .iter()
                            .map(|row| ensure_semicolon(&cell(row.first())))
                            .collect()
                    })
                    .unwrap_or_default();
                Ok((!parts.is_empty()).then(|| parts.join("\n\n")))
            }
            Driver::Oracle => {
                let kind = if first_cell(
                    self.query_admin(&format!(
                        "SELECT 1 FROM all_views WHERE owner = {s} AND view_name = {t}"
                    ))
                    .await?,
                    0,
                )
                .is_some()
                {
                    "VIEW"
                } else {
                    "TABLE"
                };
                Ok(first_cell(
                    self.query_admin(&format!(
                        "SELECT DBMS_METADATA.GET_DDL('{kind}', {t}, {s}) FROM dual"
                    ))
                    .await?,
                    0,
                )
                .map(|d| ensure_semicolon(d.trim())))
            }
            Driver::Mssql => {
                let view = first_cell(
                    self.query_admin(&format!(
                        "SELECT OBJECT_DEFINITION(OBJECT_ID({}))",
                        self.literal(&format!("{schema}.{table}"))
                    ))
                    .await?,
                    0,
                );
                Ok(view.map(|v| v.trim().to_string()))
            }
            Driver::Cassandra => Ok(first_cell(
                self.query_admin(&format!("DESCRIBE TABLE {target}"))
                    .await?,
                3,
            )),
            Driver::Mongo | Driver::Redis => Ok(None),
        }
    }

    async fn mongo_ddl(&self, coll: &str) -> Result<String> {
        let name = serde_json::to_string(coll).unwrap_or_default();
        let mut out = format!("db.createCollection({name});");
        if let Ok(idx) = self.list_indexes("", coll).await {
            for i in idx.iter().filter(|i| i.name != "_id_") {
                let keys: Vec<String> = i
                    .columns
                    .iter()
                    .map(|c| format!("{}: 1", serde_json::to_string(c).unwrap_or_default()))
                    .collect();
                out.push_str(&format!(
                    "\ndb.getCollection({name}).createIndex({{ {} }}, {{ name: {}{} }});",
                    keys.join(", "),
                    serde_json::to_string(&i.name).unwrap_or_default(),
                    if i.unique { ", unique: true" } else { "" }
                ));
            }
        }
        Ok(out)
    }

    /// `CREATE TABLE` for a new table from a column list.
    pub fn create_table_sql(
        &self,
        schema: &str,
        table: &str,
        columns: &[ColumnInfo],
    ) -> Result<String> {
        if table.trim().is_empty() {
            return Err(Error::Other("the table needs a name".into()));
        }
        if columns.is_empty() {
            return Err(Error::Other("add at least one column".into()));
        }
        let d = self.driver();
        if matches!(d, Driver::Mongo | Driver::Redis) {
            return Err(Error::Other("not available for this database".into()));
        }
        let mut lines = Vec::new();
        for c in columns {
            if c.name.trim().is_empty() || c.data_type.trim().is_empty() {
                return Err(Error::Other("every column needs a name and a type".into()));
            }
            let mut l = format!("  {} {}", self.ident(c.name.trim()), c.data_type.trim());
            if !c.nullable && d != Driver::Cassandra {
                l.push_str(" NOT NULL");
            }
            if let Some(v) = c.default.as_deref().filter(|v| !v.trim().is_empty()) {
                if d != Driver::Cassandra {
                    l.push_str(&format!(" DEFAULT {}", v.trim()));
                }
            }
            lines.push(l);
        }
        let pk: Vec<String> = columns
            .iter()
            .filter(|c| c.primary_key)
            .map(|c| self.ident(c.name.trim()))
            .collect();
        if !pk.is_empty() && d != Driver::Clickhouse {
            lines.push(format!("  PRIMARY KEY ({})", pk.join(", ")));
        }
        if d == Driver::Cassandra && pk.is_empty() {
            return Err(Error::Other("Cassandra tables need a primary key".into()));
        }
        let mut sql = format!(
            "CREATE TABLE {} (\n{}\n)",
            self.target(schema, table),
            lines.join(",\n")
        );
        if d == Driver::Clickhouse {
            let order = if pk.is_empty() {
                "tuple()".to_string()
            } else {
                format!("({})", pk.join(", "))
            };
            sql.push_str(&format!(" ENGINE = MergeTree ORDER BY {order}"));
        }
        sql.push(';');
        Ok(sql)
    }

    pub async fn create_table(
        &self,
        schema: &str,
        table: &str,
        columns: &[ColumnInfo],
    ) -> Result<String> {
        if self.config.read_only {
            return Err(Error::ReadOnly("connection is read-only".into()));
        }
        let sql = self.create_table_sql(schema, table, columns)?;
        self.exec_tx(vec![(Expect::Any, sql.trim_end_matches(';').to_string())])
            .await?;
        Ok(sql)
    }

    /// Statements for a table action (shown to the user before they run).
    pub async fn table_action_sql(
        &self,
        schema: &str,
        table: &str,
        is_view: bool,
        action: &TableAction,
    ) -> Result<Vec<String>> {
        let d = self.driver();
        let t = self.target(schema, table);
        let obj = if is_view { "VIEW" } else { "TABLE" };
        let check_name = |n: &str| -> Result<String> {
            let n = n.trim();
            if n.is_empty() {
                Err(Error::Other("enter a new name".into()))
            } else if n == table {
                Err(Error::Other(
                    "the new name is the same as the old one".into(),
                ))
            } else {
                Ok(n.to_string())
            }
        };
        if d == Driver::Mongo {
            let coll = |c: &str| {
                format!(
                    "db.getCollection({})",
                    serde_json::to_string(c).unwrap_or_default()
                )
            };
            return match action {
                TableAction::Drop => Ok(vec![format!("{}.drop()", coll(table))]),
                TableAction::Truncate => Ok(vec![format!("{}.deleteMany({{}})", coll(table))]),
                TableAction::Duplicate {
                    new_name,
                    with_data,
                } => {
                    let n = check_name(new_name)?;
                    let m = if *with_data {
                        "{ $match: {} }"
                    } else {
                        "{ $limit: 0 }"
                    };
                    Ok(vec![format!(
                        "{}.aggregate([{m}, {{ $out: {} }}])",
                        coll(table),
                        serde_json::to_string(&n).unwrap_or_default()
                    )])
                }
                TableAction::Rename { .. } => Err(Error::Other(
                    "renaming collections isn't supported yet".into(),
                )),
            };
        }
        if d == Driver::Redis {
            return Err(Error::Other("not available for Redis".into()));
        }
        Ok(match action {
            TableAction::Drop => vec![format!("DROP {obj} {t}")],
            TableAction::Truncate => {
                if is_view {
                    return Err(Error::Other("views can't be truncated".into()));
                }
                match d {
                    Driver::Sqlite | Driver::Duckdb => vec![format!("DELETE FROM {t}")],
                    Driver::Cassandra => vec![format!("TRUNCATE {t}")],
                    _ => vec![format!("TRUNCATE TABLE {t}")],
                }
            }
            TableAction::Rename { new_name } => {
                let n = check_name(new_name)?;
                match d {
                    Driver::Mysql | Driver::Clickhouse => {
                        vec![format!("RENAME TABLE {t} TO {}", self.target(schema, &n))]
                    }
                    Driver::Mssql => vec![format!(
                        "EXEC sp_rename {}, {}",
                        self.literal(&format!("{schema}.{table}")),
                        self.literal(&n)
                    )],
                    Driver::Cassandra => {
                        return Err(Error::Other("Cassandra can't rename tables".into()))
                    }
                    _ => vec![format!("ALTER {obj} {t} RENAME TO {}", self.ident(&n))],
                }
            }
            TableAction::Duplicate {
                new_name,
                with_data,
            } => {
                if is_view {
                    return Err(Error::Other(
                        "duplicate the view's DDL instead (Show DDL)".into(),
                    ));
                }
                let n = check_name(new_name)?;
                let nt = self.target(schema, &n);
                let copy = format!("INSERT INTO {nt} SELECT * FROM {t}");
                let mut v = match d {
                    Driver::Postgres => vec![format!("CREATE TABLE {nt} (LIKE {t} INCLUDING ALL)")],
                    Driver::Mysql => vec![format!("CREATE TABLE {nt} LIKE {t}")],
                    Driver::Clickhouse => vec![format!("CREATE TABLE {nt} AS {t}")],
                    Driver::Sqlite => {
                        let ddl = first_cell(
                            self.query_admin(&format!(
                                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = {}",
                                self.literal(table)
                            ))
                            .await?,
                            0,
                        )
                        .ok_or_else(|| Error::Other(format!("`{table}` not found")))?;
                        let create = rename_create(&ddl, &self.ident(&n)).ok_or_else(|| {
                            Error::Other("couldn't read the table definition".into())
                        })?;
                        vec![create]
                    }
                    Driver::Oracle => {
                        return Ok(vec![format!(
                            "CREATE TABLE {nt} AS SELECT * FROM {t}{}",
                            if *with_data { "" } else { " WHERE 1 = 0" }
                        )])
                    }
                    Driver::Duckdb => {
                        return Ok(vec![format!(
                            "CREATE TABLE {nt} AS SELECT * FROM {t}{}",
                            if *with_data { "" } else { " LIMIT 0" }
                        )])
                    }
                    Driver::Mssql => {
                        return Ok(vec![format!(
                            "SELECT * INTO {nt} FROM {t}{}",
                            if *with_data { "" } else { " WHERE 1 = 0" }
                        )])
                    }
                    Driver::Cassandra => {
                        return Err(Error::Other(
                            "copy the DDL (Show DDL) to duplicate a Cassandra table".into(),
                        ))
                    }
                    Driver::Mongo | Driver::Redis => unreachable!(),
                };
                if *with_data {
                    v.push(copy);
                }
                v
            }
        })
    }

    pub async fn table_action(
        &self,
        schema: &str,
        table: &str,
        is_view: bool,
        action: &TableAction,
    ) -> Result<Vec<String>> {
        if self.config.read_only {
            return Err(Error::ReadOnly("connection is read-only".into()));
        }
        let stmts = self
            .table_action_sql(schema, table, is_view, action)
            .await?;
        if self.driver() == Driver::Mongo {
            for s in &stmts {
                self.query_admin(s).await?;
            }
        } else {
            self.exec_tx(stmts.iter().map(|s| (Expect::Any, s.clone())).collect())
                .await?;
        }
        Ok(stmts)
    }
}

#[cfg(test)]
mod tests {
    use super::rename_create;

    #[test]
    fn renames_create_statements() {
        assert_eq!(
            rename_create("CREATE TABLE \"a\"\"b\" (id INTEGER)", "\"c\"").unwrap(),
            "CREATE TABLE \"c\" (id INTEGER)"
        );
        assert_eq!(
            rename_create("CREATE TABLE users(id)", "\"u2\"").unwrap(),
            "CREATE TABLE \"u2\"(id)"
        );
        assert_eq!(
            rename_create("create table if not exists `x` (a)", "\"y\"").unwrap(),
            "create table if not exists \"y\" (a)"
        );
    }
}
