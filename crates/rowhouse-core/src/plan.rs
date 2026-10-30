//! Query plans, normalised into one tree shape for the plan viewer.
//!
//! * PostgreSQL / Greenplum: `EXPLAIN (FORMAT JSON [, ANALYZE, BUFFERS])`
//! * MySQL / MariaDB: `EXPLAIN FORMAT=JSON`
//! * SQLite: `EXPLAIN QUERY PLAN`
//! * Everything else: the engine's text plan (shown as-is)
//!
//! `ANALYZE` actually executes the statement, so it is only allowed for
//! read-only statements and runs inside a read-only transaction.

use serde::Serialize;
use serde_json::{Map, Value};

use crate::{
    config::{DbKind, Driver},
    guard, Connection, Error, Result,
};

#[derive(Debug, Clone, Default, Serialize)]
pub struct PlanNode {
    /// e.g. "Seq Scan", "Hash Join", "table: orders"
    pub label: String,
    /// relation / index / condition summary
    pub detail: Option<String>,
    pub cost: Option<f64>,
    pub rows: Option<f64>,
    pub actual_ms: Option<f64>,
    pub actual_rows: Option<f64>,
    pub loops: Option<f64>,
    /// human hints such as "full table scan"
    pub warnings: Vec<String>,
    pub children: Vec<PlanNode>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Plan {
    /// empty when only a text plan is available
    pub nodes: Vec<PlanNode>,
    /// the engine's own output (JSON or text)
    pub raw: String,
    pub analyzed: bool,
    pub execution_ms: Option<f64>,
    pub planning_ms: Option<f64>,
}

const BIG_SCAN_ROWS: f64 = 10_000.0;

fn f(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

fn first_text_col(c: &Connection, rows: &[Vec<Value>]) -> String {
    let _ = c;
    rows.iter()
        .map(|r| {
            r.iter()
                .filter_map(|v| match v {
                    Value::String(s) => Some(s.clone()),
                    Value::Null => None,
                    o => Some(o.to_string()),
                })
                .collect::<Vec<_>>()
                .join("  ")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ------------------------------------------------------------- Postgres ---

fn pg_node(p: &Map<String, Value>) -> PlanNode {
    let s = |k: &str| p.get(k).and_then(Value::as_str).map(str::to_string);
    let mut label = s("Node Type").unwrap_or_else(|| "?".into());
    if let Some(j) = s("Join Type").filter(|j| j != "Inner") {
        label = format!("{label} ({j})");
    }
    let mut detail = Vec::new();
    if let Some(rel) = s("Relation Name") {
        let alias = s("Alias").filter(|a| a != &rel);
        detail.push(match alias {
            Some(a) => format!("{rel} {a}"),
            None => rel,
        });
    }
    if let Some(i) = s("Index Name") {
        detail.push(format!("using {i}"));
    }
    for k in [
        "Index Cond",
        "Hash Cond",
        "Merge Cond",
        "Join Filter",
        "Filter",
        "Sort Key",
        "Group Key",
    ] {
        if let Some(v) = p.get(k) {
            let text = match v {
                Value::Array(a) => a
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", "),
                Value::String(s) => s.clone(),
                _ => continue,
            };
            detail.push(format!("{}: {text}", k.to_lowercase()));
        }
    }
    let rows = p.get("Plan Rows").and_then(f);
    let actual_rows = p.get("Actual Rows").and_then(f);
    let mut warnings = Vec::new();
    if label.starts_with("Seq Scan") && actual_rows.or(rows).unwrap_or(0.0) >= BIG_SCAN_ROWS {
        warnings.push("full table scan on a large table — consider an index".into());
    }
    if let (Some(est), Some(act)) = (rows, actual_rows) {
        if est > 0.0
            && (act / est > 10.0 || est / act.max(1.0) > 10.0)
            && (act - est).abs() > 1000.0
        {
            warnings.push(format!(
                "row estimate off ({est:.0} est. vs {act:.0} actual) — run ANALYZE"
            ));
        }
    }
    if p.get("Sort Space Type").and_then(Value::as_str) == Some("Disk") {
        warnings.push("sort spilled to disk — raise work_mem".into());
    }
    PlanNode {
        label,
        detail: (!detail.is_empty()).then(|| detail.join(" · ")),
        cost: p.get("Total Cost").and_then(f),
        rows,
        actual_ms: p.get("Actual Total Time").and_then(f),
        actual_rows,
        loops: p.get("Actual Loops").and_then(f),
        warnings,
        children: p
            .get("Plans")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_object).map(pg_node).collect())
            .unwrap_or_default(),
    }
}

// ---------------------------------------------------------------- MySQL ---

const MY_GROUPS: &[&str] = &[
    "query_block",
    "nested_loop",
    "ordering_operation",
    "grouping_operation",
    "duplicates_removal",
    "union_result",
    "materialized_from_subquery",
    "attached_subqueries",
    "subqueries",
    "read_sorted_file",
    "filesort",
    "temporary_table",
    "having_subqueries",
    "select_list_subqueries",
];

fn my_table(t: &Map<String, Value>) -> PlanNode {
    let s = |k: &str| t.get(k).and_then(Value::as_str).map(str::to_string);
    let access = s("access_type").unwrap_or_default();
    let name = s("table_name").unwrap_or_else(|| "?".into());
    let rows = t
        .get("rows_examined_per_scan")
        .or(t.get("rows"))
        .and_then(f);
    let mut detail = Vec::new();
    if let Some(k) = s("key") {
        detail.push(format!("using {k}"));
    }
    if let Some(c) = s("attached_condition") {
        detail.push(format!("filter: {c}"));
    }
    let mut warnings = Vec::new();
    if access == "ALL" && rows.unwrap_or(0.0) >= BIG_SCAN_ROWS {
        warnings.push("full table scan on a large table — consider an index".into());
    }
    if t.get("using_filesort").and_then(Value::as_bool) == Some(true) {
        warnings.push("filesort".into());
    }
    let cost = t
        .get("cost_info")
        .and_then(|c| c.get("prefix_cost").or(c.get("read_cost")))
        .and_then(f)
        .or(t.get("cost").and_then(f));
    let mut node = PlanNode {
        label: format!(
            "{} {name}",
            if access.is_empty() {
                "table".into()
            } else {
                access.to_lowercase()
            }
        ),
        detail: (!detail.is_empty()).then(|| detail.join(" · ")),
        cost,
        rows,
        warnings,
        ..Default::default()
    };
    // derived tables / subqueries hang below the table
    for (k, v) in t {
        if MY_GROUPS.contains(&k.as_str()) {
            node.children.extend(my_walk(k, v));
        }
    }
    node
}

fn my_walk(key: &str, v: &Value) -> Vec<PlanNode> {
    match v {
        Value::Array(items) => items.iter().flat_map(|i| my_walk(key, i)).collect(),
        Value::Object(o) => {
            if key == "table" {
                return vec![my_table(o)];
            }
            let mut children = Vec::new();
            for (k, v) in o {
                if k == "table" || MY_GROUPS.contains(&k.as_str()) {
                    children.extend(my_walk(k, v));
                }
            }
            if key == "nested_loop" || key == "attached_subqueries" || key == "subqueries" {
                // these are plain lists of members, not operators
                return children;
            }
            let cost = o
                .get("cost_info")
                .and_then(|c| c.get("query_cost"))
                .and_then(f)
                .or(o.get("cost").and_then(f));
            vec![PlanNode {
                label: key.replace('_', " "),
                cost,
                children,
                ..Default::default()
            }]
        }
        _ => vec![],
    }
}

// --------------------------------------------------------------- SQLite ---

fn sqlite_tree(rows: &[Vec<Value>]) -> Vec<PlanNode> {
    // (id, parent, notused, detail)
    let items: Vec<(i64, i64, String)> = rows
        .iter()
        .map(|r| {
            (
                f(&r[0]).unwrap_or(0.0) as i64,
                f(&r[1]).unwrap_or(0.0) as i64,
                r[3].as_str().unwrap_or("").to_string(),
            )
        })
        .collect();
    fn build(items: &[(i64, i64, String)], parent: i64) -> Vec<PlanNode> {
        items
            .iter()
            .filter(|(_, p, _)| *p == parent)
            .map(|(id, _, d)| {
                let mut warnings = vec![];
                if d.starts_with("SCAN ")
                    && !d.contains("COVERING INDEX")
                    && !d.contains("USING INDEX")
                {
                    warnings.push("full table scan".into());
                }
                if d.contains("TEMP B-TREE") {
                    warnings.push("temporary b-tree (sort / distinct)".into());
                }
                PlanNode {
                    label: d.clone(),
                    warnings,
                    children: build(items, *id),
                    ..Default::default()
                }
            })
            .collect()
    }
    build(&items, 0)
}

impl Connection {
    /// Plan for the *first* statement of `sql`.
    pub async fn explain_plan(&self, sql: &str, analyze: bool) -> Result<Plan> {
        let stmt = guard::split_statements(sql)
            .into_iter()
            .next()
            .ok_or_else(|| Error::Other("nothing to explain".into()))?;
        let stmt = stmt.trim_end_matches(';').to_string();
        if analyze {
            guard::check_read_only(&stmt).map_err(|e| {
                Error::ReadOnly(format!(
                    "{e} — EXPLAIN ANALYZE executes the query, so it's only allowed for reads"
                ))
            })?;
        }
        let pg_json = self.driver() == Driver::Postgres
            && matches!(self.kind(), DbKind::Postgres | DbKind::Greenplum);
        let run = |q: String| async move {
            let out = if analyze {
                self.query_read_only(&q, usize::MAX).await?
            } else {
                self.query(&q, usize::MAX).await?
            };
            Ok::<_, Error>(out.results.into_iter().next().unwrap_or_default().rows)
        };
        match self.driver() {
            Driver::Postgres if pg_json => {
                let opts = if analyze {
                    "FORMAT JSON, ANALYZE, BUFFERS"
                } else {
                    "FORMAT JSON"
                };
                let rows = run(format!("EXPLAIN ({opts}) {stmt}")).await?;
                let raw = rows
                    .first()
                    .and_then(|r| r.first())
                    .cloned()
                    .unwrap_or(Value::Null);
                // the text protocol hands JSON back already parsed or as a string
                let v = match &raw {
                    Value::String(s) => serde_json::from_str(s)?,
                    other => other.clone(),
                };
                let top = v.get(0).cloned().unwrap_or(Value::Null);
                let root = top.get("Plan").and_then(Value::as_object).map(pg_node);
                Ok(Plan {
                    nodes: root.into_iter().collect(),
                    raw: serde_json::to_string_pretty(&v)?,
                    analyzed: analyze,
                    execution_ms: top.get("Execution Time").and_then(f),
                    planning_ms: top.get("Planning Time").and_then(f),
                })
            }
            Driver::Mysql if !analyze => {
                let rows = run(format!("EXPLAIN FORMAT=JSON {stmt}")).await?;
                let text = rows
                    .first()
                    .and_then(|r| r.first())
                    .and_then(Value::as_str)
                    .unwrap_or("{}")
                    .to_string();
                let v: Value = serde_json::from_str(&text)?;
                let nodes = match v.get("query_block") {
                    Some(q) => my_walk("query_block", q),
                    None => vec![],
                };
                Ok(Plan {
                    nodes,
                    raw: serde_json::to_string_pretty(&v)?,
                    ..Default::default()
                })
            }
            Driver::Sqlite if !analyze => {
                let rows = run(format!("EXPLAIN QUERY PLAN {stmt}")).await?;
                let raw = rows
                    .iter()
                    .map(|r| r[3].as_str().unwrap_or("").to_string())
                    .collect::<Vec<_>>()
                    .join("\n");
                Ok(Plan {
                    nodes: sqlite_tree(&rows),
                    raw,
                    ..Default::default()
                })
            }
            Driver::Redis => Err(Error::Other("Redis has no query plans".into())),
            Driver::Cassandra => Err(Error::Other("query plans aren't available for CQL — use TRACING ON in cqlsh".into())),
            Driver::Mongo => Err(Error::Other("query plans aren't available for MongoDB yet — run db.runCommand({ explain: { find: \"coll\", filter: {…} } })".into())),
            Driver::Mssql => Err(Error::Other(
                "query plans for SQL Server are not supported yet".into(),
            )),
            Driver::Mysql | Driver::Sqlite => Err(Error::Other(
                "EXPLAIN ANALYZE is only supported for PostgreSQL right now".into(),
            )),
            // CockroachDB, Redshift, DuckDB, ClickHouse: show the engine's text plan
            _ => {
                let prefix = if analyze {
                    "EXPLAIN ANALYZE"
                } else {
                    "EXPLAIN"
                };
                let rows = run(format!("{prefix} {stmt}")).await?;
                Ok(Plan {
                    raw: first_text_col(self, &rows),
                    analyzed: analyze,
                    ..Default::default()
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mysql_json_walk() {
        let v: Value = serde_json::from_str(
            r#"{"query_block":{"select_id":1,"cost_info":{"query_cost":"12.5"},
               "ordering_operation":{"using_filesort":true,
                 "nested_loop":[
                   {"table":{"table_name":"o","access_type":"ALL","rows_examined_per_scan":50000,"cost_info":{"prefix_cost":"5"}}},
                   {"table":{"table_name":"u","access_type":"eq_ref","key":"PRIMARY","rows_examined_per_scan":1}}
                 ]}}}"#,
        )
        .unwrap();
        let nodes = my_walk("query_block", &v["query_block"]);
        assert_eq!(nodes.len(), 1);
        let qb = &nodes[0];
        assert_eq!(qb.label, "query block");
        assert_eq!(qb.cost, Some(12.5));
        let order = &qb.children[0];
        assert_eq!(order.label, "ordering operation");
        assert_eq!(order.children.len(), 2);
        assert_eq!(order.children[0].label, "all o");
        assert!(order.children[0].warnings[0].contains("full table scan"));
        assert_eq!(order.children[1].detail.as_deref(), Some("using PRIMARY"));
    }
}
