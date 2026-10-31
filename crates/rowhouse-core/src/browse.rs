//! Server-side table browsing: filters, sort and pagination pushed into SQL,
//! so big tables never have to be pulled into the app.

use serde::{Deserialize, Serialize};

use crate::{config::Driver, Connection, QueryResult, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FilterOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Contains,
    StartsWith,
    IsNull,
    NotNull,
    /// comma-separated list
    In,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Filter {
    pub column: String,
    pub op: FilterOp,
    #[serde(default)]
    pub value: String,
    /// column is numeric → compare without quotes
    #[serde(default)]
    pub numeric: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sort {
    pub column: String,
    #[serde(default)]
    pub desc: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageRequest {
    pub schema: String,
    pub table: String,
    #[serde(default)]
    pub filters: Vec<Filter>,
    #[serde(default)]
    pub sort: Option<Sort>,
    pub limit: usize,
    #[serde(default)]
    pub offset: usize,
    /// run COUNT(*) as well (skip it on huge tables if it's slow)
    #[serde(default = "yes")]
    pub count: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Serialize)]
pub struct Page {
    pub result: QueryResult,
    pub total: Option<u64>,
    /// the SELECT that produced the page (shown in the toolbar)
    pub sql: String,
    pub elapsed_ms: u128,
}

impl Connection {
    fn value_sql(&self, f: &Filter, v: &str) -> String {
        let v = v.trim();
        if f.numeric && !v.is_empty() && v.parse::<f64>().is_ok() {
            v.to_string()
        } else {
            self.literal(v)
        }
    }

    fn as_text(&self, col: &str) -> String {
        match self.driver() {
            Driver::Mysql => format!("CAST({col} AS CHAR)"),
            Driver::Mssql => format!("CAST({col} AS NVARCHAR(MAX))"),
            Driver::Clickhouse => format!("toString({col})"),
            Driver::Duckdb => format!("CAST({col} AS VARCHAR)"),
            Driver::Oracle => format!("TO_CHAR({col})"),
            Driver::Elastic => format!("CAST({col} AS VARCHAR)"),
            Driver::Bigquery => format!("CAST({col} AS STRING)"),
            _ => format!("CAST({col} AS TEXT)"),
        }
    }

    fn like(&self) -> &'static str {
        // case-insensitive "contains" everywhere
        match self.driver() {
            Driver::Postgres | Driver::Duckdb | Driver::Clickhouse | Driver::Snowflake => "ILIKE",
            _ => "LIKE",
        }
    }

    /// `WHERE …` for the filters (empty string when there are none).
    pub fn where_sql(&self, filters: &[Filter]) -> String {
        let parts: Vec<String> = filters
            .iter()
            .filter(|f| !f.column.is_empty())
            .map(|f| {
                let c = self.ident(&f.column);
                match f.op {
                    FilterOp::IsNull => format!("{c} IS NULL"),
                    FilterOp::NotNull => format!("{c} IS NOT NULL"),
                    FilterOp::Contains => format!(
                        "{} {} {}",
                        self.as_text(&c),
                        self.like(),
                        self.literal(&format!("%{}%", f.value))
                    ),
                    FilterOp::StartsWith => format!(
                        "{} {} {}",
                        self.as_text(&c),
                        self.like(),
                        self.literal(&format!("{}%", f.value))
                    ),
                    FilterOp::In => {
                        let items: Vec<String> = f
                            .value
                            .split(',')
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(|v| self.value_sql(f, v))
                            .collect();
                        if items.is_empty() {
                            "1 = 0".into()
                        } else {
                            format!("{c} IN ({})", items.join(", "))
                        }
                    }
                    op => {
                        let sym = match op {
                            FilterOp::Eq => "=",
                            FilterOp::Ne => "<>",
                            FilterOp::Lt => "<",
                            FilterOp::Le => "<=",
                            FilterOp::Gt => ">",
                            _ => ">=",
                        };
                        format!("{c} {sym} {}", self.value_sql(f, &f.value))
                    }
                }
            })
            .collect();
        if parts.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", parts.join(" AND "))
        }
    }

    /// The SELECT for one page.
    pub fn page_sql(&self, r: &PageRequest) -> String {
        let t = self.target(&r.schema, &r.table);
        let w = self.where_sql(&r.filters);
        let order = r.sort.as_ref().map(|s| {
            format!(
                " ORDER BY {} {}",
                self.ident(&s.column),
                if s.desc { "DESC" } else { "ASC" }
            )
        });
        if self.driver() == Driver::Mssql {
            // OFFSET/FETCH needs an ORDER BY
            let order = order.unwrap_or_else(|| " ORDER BY (SELECT NULL)".into());
            return format!(
                "SELECT * FROM {t}{w}{order} OFFSET {} ROWS FETCH NEXT {} ROWS ONLY",
                r.offset, r.limit
            );
        }
        if self.driver() == Driver::Oracle {
            return format!(
                "SELECT * FROM {t}{w}{} OFFSET {} ROWS FETCH NEXT {} ROWS ONLY",
                order.unwrap_or_default(),
                r.offset,
                r.limit
            );
        }
        if self.driver() == Driver::Elastic {
            // no OFFSET in Elasticsearch SQL: fetch through the page, table_page skips ahead
            return format!(
                "SELECT * FROM {t}{w}{} LIMIT {}",
                order.unwrap_or_default(),
                r.offset + r.limit
            );
        }
        let mut sql = format!(
            "SELECT * FROM {t}{w}{} LIMIT {}",
            order.unwrap_or_default(),
            r.limit
        );
        if r.offset > 0 {
            sql.push_str(&format!(" OFFSET {}", r.offset));
        }
        sql
    }

    pub async fn table_page(&self, r: &PageRequest) -> Result<Page> {
        let started = std::time::Instant::now();
        if let Some(c) = self.cql() {
            let target = self.target(&r.schema, &r.table);
            let (result, total, sql) = c.page(r, &target, |x| self.ident(x)).await?;
            return Ok(Page {
                result,
                total,
                sql,
                elapsed_ms: started.elapsed().as_millis(),
            });
        }
        if let Some(m) = self.mongo() {
            let (result, total, sql) = m.page(r).await?;
            return Ok(Page {
                result,
                total,
                sql,
                elapsed_ms: started.elapsed().as_millis(),
            });
        }
        if let Some(redis) = self.redis() {
            let (result, total, sql) = redis.key_page(&r.table, r.offset, r.limit).await?;
            return Ok(Page {
                result,
                total,
                sql,
                elapsed_ms: started.elapsed().as_millis(),
            });
        }
        let sql = self.page_sql(r);
        let skip = if self.driver() == Driver::Elastic {
            r.offset
        } else {
            0
        };
        let out = self.query(&sql, r.limit + skip).await?;
        let mut result = out.results.into_iter().next().unwrap_or_default();
        if skip > 0 {
            result.rows.drain(..skip.min(result.rows.len()));
        }
        let total = if r.count {
            let count_sql = format!(
                "SELECT COUNT(*) FROM {}{}",
                self.target(&r.schema, &r.table),
                self.where_sql(&r.filters)
            );
            let c = self.query(&count_sql, 1).await?;
            c.results
                .first()
                .and_then(|x| x.rows.first())
                .and_then(|row| row.first())
                .and_then(|v| match v {
                    serde_json::Value::Number(n) => n.as_u64(),
                    serde_json::Value::String(s) => s.parse().ok(),
                    _ => None,
                })
        } else {
            None
        };
        Ok(Page {
            result,
            total,
            sql,
            elapsed_ms: started.elapsed().as_millis(),
        })
    }
}
