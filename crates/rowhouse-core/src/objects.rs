//! Schema objects besides tables and views: functions, procedures, triggers,
//! sequences, enum types and materialized views — listed for the sidebar,
//! with their definitions for editing in a query tab.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{config::Driver, Connection, Error, Result};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DbObject {
    pub schema: String,
    pub name: String,
    /// function | procedure | trigger | sequence | type | matview | macro
    pub kind: String,
    /// arguments (functions) or the table (triggers); also identifies overloads
    #[serde(default)]
    pub detail: Option<String>,
}

fn s(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(x)) => x.clone(),
        Some(Value::Null) | None => String::new(),
        Some(o) => o.to_string(),
    }
}

impl Connection {
    async fn rows(&self, sql: &str) -> Result<Vec<Vec<Value>>> {
        Ok(self
            .query_admin(sql)
            .await?
            .results
            .into_iter()
            .next()
            .map(|r| r.rows)
            .unwrap_or_default())
    }

    /// Everything except tables/views; empty for engines without such objects.
    pub async fn list_objects(&self) -> Result<Vec<DbObject>> {
        let sql = match self.driver() {
            Driver::Postgres => {
                "SELECT n.nspname, p.proname, CASE p.prokind WHEN 'p' THEN 'procedure' ELSE 'function' END, \
                   pg_get_function_identity_arguments(p.oid) \
                 FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace \
                 WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND n.nspname NOT LIKE 'pg_%' \
                   AND p.prokind IN ('f','p') \
                   AND NOT EXISTS (SELECT 1 FROM pg_depend d WHERE d.objid = p.oid AND d.deptype = 'e') \
                 UNION ALL \
                 SELECT n.nspname, t.tgname, 'trigger', c.relname FROM pg_trigger t \
                   JOIN pg_class c ON c.oid = t.tgrelid JOIN pg_namespace n ON n.oid = c.relnamespace \
                   WHERE NOT t.tgisinternal AND n.nspname NOT LIKE 'pg_%' \
                 UNION ALL \
                 SELECT n.nspname, c.relname, CASE c.relkind WHEN 'S' THEN 'sequence' ELSE 'matview' END, NULL \
                   FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
                   WHERE c.relkind IN ('S','m') AND n.nspname NOT IN ('pg_catalog','information_schema') AND n.nspname NOT LIKE 'pg_%' \
                   AND NOT EXISTS (SELECT 1 FROM pg_depend d WHERE d.objid = c.oid AND d.deptype IN ('e','i','a')) \
                 UNION ALL \
                 SELECT n.nspname, t.typname, 'type', NULL FROM pg_type t JOIN pg_namespace n ON n.oid = t.typnamespace \
                   WHERE t.typtype = 'e' AND n.nspname NOT IN ('pg_catalog','information_schema') \
                   AND NOT EXISTS (SELECT 1 FROM pg_depend d WHERE d.objid = t.oid AND d.deptype = 'e') \
                 ORDER BY 3, 1, 2"
            }
            Driver::Mysql => {
                "SELECT routine_schema, routine_name, LOWER(routine_type), NULL FROM information_schema.routines \
                   WHERE routine_schema = DATABASE() \
                 UNION ALL \
                 SELECT trigger_schema, trigger_name, 'trigger', event_object_table FROM information_schema.triggers \
                   WHERE trigger_schema = DATABASE() ORDER BY 3, 1, 2"
            }
            Driver::Mssql => {
                "SELECT SCHEMA_NAME(o.schema_id), o.name, \
                   CASE WHEN o.type IN ('FN','IF','TF') THEN 'function' WHEN o.type = 'P' THEN 'procedure' \
                        WHEN o.type = 'TR' THEN 'trigger' ELSE 'sequence' END, \
                   CASE WHEN o.type = 'TR' THEN OBJECT_NAME(o.parent_object_id) END \
                 FROM sys.objects o WHERE o.type IN ('FN','IF','TF','P','TR','SO') AND o.is_ms_shipped = 0 \
                 ORDER BY 3, 1, 2"
            }
            Driver::Sqlite => {
                "SELECT 'main', name, 'trigger', tbl_name FROM sqlite_master WHERE type = 'trigger' ORDER BY 2"
            }
            Driver::Duckdb => {
                "SELECT schema_name, function_name, 'macro', array_to_string(parameters, ', ') FROM duckdb_functions() \
                   WHERE function_type IN ('macro', 'table_macro') AND NOT internal \
                 UNION ALL SELECT schema_name, sequence_name, 'sequence', NULL FROM duckdb_sequences() WHERE NOT temporary \
                 UNION ALL SELECT schema_name, type_name, 'type', NULL FROM duckdb_types() WHERE NOT internal AND logical_type = 'ENUM' \
                 ORDER BY 3, 1, 2"
            }
            Driver::Clickhouse => {
                "SELECT '', name, 'function', NULL FROM system.functions WHERE origin = 'SQLUserDefined' ORDER BY 2"
            }
            Driver::Mongo | Driver::Redis | Driver::Cassandra => return Ok(vec![]),
        };
        Ok(self
            .rows(sql)
            .await?
            .into_iter()
            .map(|r| DbObject {
                schema: s(r.first()),
                name: s(r.get(1)),
                kind: s(r.get(2)),
                detail: Some(s(r.get(3))).filter(|d| !d.is_empty()),
            })
            .collect())
    }

    /// The object's `CREATE` statement.
    pub async fn object_definition(&self, o: &DbObject) -> Result<String> {
        let (sc, nm) = (self.literal(&o.schema), self.literal(&o.name));
        let first = |rows: Vec<Vec<Value>>, col: usize| -> Result<String> {
            rows.into_iter()
                .next()
                .map(|r| s(r.get(col)))
                .filter(|x| !x.trim().is_empty())
                .ok_or_else(|| Error::Other(format!("no definition found for {}", o.name)))
        };
        let semi = |x: String| {
            let t = x.trim_end().to_string();
            if t.ends_with(';') {
                t
            } else {
                format!("{t};")
            }
        };
        match (self.driver(), o.kind.as_str()) {
            (_, "matview") => self.table_ddl(&o.schema, &o.name).await,
            (Driver::Postgres, "function" | "procedure") => first(
                self.rows(&format!(
                    "SELECT pg_get_functiondef(p.oid) FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace \
                     WHERE n.nspname = {sc} AND p.proname = {nm} AND pg_get_function_identity_arguments(p.oid) = {}",
                    self.literal(o.detail.as_deref().unwrap_or(""))
                ))
                .await?,
                0,
            )
            .map(semi),
            (Driver::Postgres, "trigger") => first(
                self.rows(&format!(
                    "SELECT pg_get_triggerdef(t.oid, true) FROM pg_trigger t JOIN pg_class c ON c.oid = t.tgrelid \
                     JOIN pg_namespace n ON n.oid = c.relnamespace WHERE n.nspname = {sc} AND t.tgname = {nm} AND c.relname = {}",
                    self.literal(o.detail.as_deref().unwrap_or(""))
                ))
                .await?,
                0,
            )
            .map(semi),
            (Driver::Postgres, "sequence") => {
                let rows = self
                    .rows(&format!(
                        "SELECT data_type::text, start_value, min_value, max_value, increment_by, cycle, cache_size \
                         FROM pg_sequences WHERE schemaname = {sc} AND sequencename = {nm}"
                    ))
                    .await?;
                let r = rows.into_iter().next().ok_or_else(|| Error::Other("sequence not found".into()))?;
                Ok(format!(
                    "CREATE SEQUENCE {}.{} AS {} INCREMENT BY {} MINVALUE {} MAXVALUE {} START WITH {} CACHE {}{};",
                    self.ident(&o.schema),
                    self.ident(&o.name),
                    s(r.first()),
                    s(r.get(4)),
                    s(r.get(2)),
                    s(r.get(3)),
                    s(r.get(1)),
                    s(r.get(6)),
                    if s(r.get(5)) == "true" { " CYCLE" } else { "" }
                ))
            }
            (Driver::Postgres, "type") => {
                let labels: Vec<String> = self
                    .rows(&format!(
                        "SELECT e.enumlabel FROM pg_enum e JOIN pg_type t ON t.oid = e.enumtypid \
                         JOIN pg_namespace n ON n.oid = t.typnamespace WHERE n.nspname = {sc} AND t.typname = {nm} ORDER BY e.enumsortorder"
                    ))
                    .await?
                    .into_iter()
                    .map(|r| self.literal(&s(r.first())))
                    .collect();
                Ok(format!(
                    "CREATE TYPE {}.{} AS ENUM ({});",
                    self.ident(&o.schema),
                    self.ident(&o.name),
                    labels.join(", ")
                ))
            }
            (Driver::Mysql, k @ ("function" | "procedure" | "trigger")) => {
                let what = k.to_uppercase();
                first(
                    self.rows(&format!("SHOW CREATE {what} {}.{}", self.ident(&o.schema), self.ident(&o.name)))
                        .await?,
                    2,
                )
                .map(|d| format!("DELIMITER ;;\n{d};;\nDELIMITER ;"))
            }
            (Driver::Mssql, "sequence") => {
                let rows = self
                    .rows(&format!(
                        "SELECT TYPE_NAME(user_type_id), start_value, increment, minimum_value, maximum_value, is_cycling \
                         FROM sys.sequences WHERE object_id = OBJECT_ID({})",
                        self.literal(&format!("{}.{}", o.schema, o.name))
                    ))
                    .await?;
                let r = rows.into_iter().next().ok_or_else(|| Error::Other("sequence not found".into()))?;
                Ok(format!(
                    "CREATE SEQUENCE {}.{} AS {} START WITH {} INCREMENT BY {} MINVALUE {} MAXVALUE {}{};",
                    self.ident(&o.schema),
                    self.ident(&o.name),
                    s(r.first()),
                    s(r.get(1)),
                    s(r.get(2)),
                    s(r.get(3)),
                    s(r.get(4)),
                    if s(r.get(5)) == "true" || s(r.get(5)) == "1" { " CYCLE" } else { "" }
                ))
            }
            (Driver::Mssql, _) => first(
                self.rows(&format!(
                    "SELECT OBJECT_DEFINITION(OBJECT_ID({}))",
                    self.literal(&format!("{}.{}", o.schema, o.name))
                ))
                .await?,
                0,
            ),
            (Driver::Sqlite, "trigger") => first(
                self.rows(&format!("SELECT sql FROM sqlite_master WHERE type = 'trigger' AND name = {nm}")).await?,
                0,
            )
            .map(semi),
            (Driver::Duckdb, "macro") => first(
                self.rows(&format!(
                    "SELECT macro_definition FROM duckdb_functions() WHERE schema_name = {sc} AND function_name = {nm} \
                     AND function_type IN ('macro', 'table_macro') LIMIT 1"
                ))
                .await?,
                0,
            )
            .map(|body| {
                format!(
                    "CREATE OR REPLACE MACRO {}.{}({}) AS {};",
                    self.ident(&o.schema),
                    self.ident(&o.name),
                    o.detail.clone().unwrap_or_default(),
                    body.trim().trim_end_matches(';')
                )
            }),
            (Driver::Duckdb, "sequence") => first(
                self.rows(&format!("SELECT sql FROM duckdb_sequences() WHERE schema_name = {sc} AND sequence_name = {nm}"))
                    .await?,
                0,
            )
            .map(semi),
            (Driver::Duckdb, "type") => first(
                self.rows(&format!(
                    "SELECT 'CREATE TYPE ' || type_name || ' AS ' || logical_type || '(' || \
                       '''' || array_to_string(enum_range(NULL::{}), ''', ''') || ''');' \
                     FROM duckdb_types() WHERE schema_name = {sc} AND type_name = {nm}",
                    self.ident(&o.name)
                ))
                .await?,
                0,
            ),
            (Driver::Clickhouse, _) => first(
                self.rows(&format!("SELECT create_query FROM system.functions WHERE name = {nm}")).await?,
                0,
            )
            .map(semi),
            _ => Err(Error::Other(format!("no definition available for this {}", o.kind))),
        }
    }
}
