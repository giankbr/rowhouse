//! Export a query to CSV / JSON / SQL, and import CSV into a table.
//! Export streams row by row, so large tables don't have to fit in memory.

use std::{
    fs::File,
    io::{BufWriter, Write},
    path::Path,
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::AssertSqlSafe;

use crate::{
    driver::{for_each_row, lite_value, my_value, pg_value, PoolRef},
    Connection, Error, Result, ResultColumn,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    Csv,
    Json,
    Sql,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportOptions {
    /// Treat empty CSV fields as NULL (default true).
    #[serde(default = "yes")]
    pub empty_as_null: bool,
}

fn yes() -> bool {
    true
}

const IMPORT_BATCH: usize = 500;

enum Sink<'a> {
    Csv {
        w: csv::Writer<BufWriter<File>>,
        header: bool,
    },
    Json {
        w: BufWriter<File>,
        first: bool,
    },
    Sql {
        w: BufWriter<File>,
        conn: &'a Connection,
        target: String,
    },
}

fn csv_field(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

impl Sink<'_> {
    fn row(&mut self, cols: &[ResultColumn], vals: Vec<Value>) -> Result<()> {
        match self {
            Sink::Csv { w, header } => {
                if !*header {
                    w.write_record(cols.iter().map(|c| c.name.as_str()))?;
                    *header = true;
                }
                w.write_record(vals.iter().map(csv_field))?;
            }
            Sink::Json { w, first } => {
                let obj: serde_json::Map<String, Value> =
                    cols.iter().map(|c| c.name.clone()).zip(vals).collect();
                w.write_all(if *first { b"[\n  " } else { b",\n  " })?;
                serde_json::to_writer(&mut *w, &obj)?;
                *first = false;
            }
            Sink::Sql { w, conn, target } => {
                let names = cols
                    .iter()
                    .map(|c| conn.ident(&c.name))
                    .collect::<Vec<_>>()
                    .join(", ");
                let values = vals
                    .iter()
                    .map(|v| conn.sql_value(v))
                    .collect::<Vec<_>>()
                    .join(", ");
                writeln!(w, "INSERT INTO {target} ({names}) VALUES ({values});")?;
            }
        }
        Ok(())
    }

    fn finish(self) -> Result<()> {
        match self {
            Sink::Csv { mut w, .. } => w.flush()?,
            Sink::Json { mut w, first } => {
                w.write_all(if first { b"[]\n" } else { b"\n]\n" })?;
                w.flush()?
            }
            Sink::Sql { mut w, .. } => w.flush()?,
        }
        Ok(())
    }
}

impl Connection {
    /// Stream the first result set of `sql` into `path`. `table` names the
    /// INSERT target for the SQL format (defaults to `exported`).
    pub async fn export(
        &self,
        sql: &str,
        format: ExportFormat,
        path: &Path,
        table: Option<(&str, &str)>,
    ) -> Result<u64> {
        let file = BufWriter::new(File::create(path)?);
        let mut sink = match format {
            ExportFormat::Csv => Sink::Csv {
                w: csv::Writer::from_writer(file),
                header: false,
            },
            ExportFormat::Json => Sink::Json {
                w: file,
                first: true,
            },
            ExportFormat::Sql => Sink::Sql {
                w: file,
                conn: self,
                target: match table {
                    Some((schema, t)) => self.target(schema, t),
                    None => self.ident("exported"),
                },
            },
        };
        let mut f = |c: &Vec<ResultColumn>, v: Vec<Value>| sink.row(c, v);
        let n = match self.pool() {
            PoolRef::Pg(p) => for_each_row!(p, sql, pg_value, f),
            PoolRef::My(p) => for_each_row!(p, sql, my_value, f),
            PoolRef::Lite(p) => for_each_row!(p, sql, lite_value, f),
        };
        sink.finish()?;
        Ok(n)
    }

    /// Insert every row of a CSV file (first line = column names) in one transaction.
    pub async fn import_csv(
        &self,
        schema: &str,
        table: &str,
        path: &Path,
        opts: &ImportOptions,
    ) -> Result<u64> {
        if self.config.read_only {
            return Err(Error::ReadOnly("connection is read-only".into()));
        }
        let known: Vec<String> = self
            .describe_table(schema, table)
            .await?
            .into_iter()
            .map(|c| c.name)
            .collect();
        let mut rdr = csv::Reader::from_path(path)?;
        let headers: Vec<String> = rdr
            .headers()?
            .iter()
            .map(|h| h.trim().to_string())
            .collect();
        if let Some(bad) = headers.iter().find(|h| !known.contains(h)) {
            return Err(Error::Other(format!(
                "CSV column `{bad}` does not exist in {table}"
            )));
        }
        let target = self.target(schema, table);
        let cols = headers
            .iter()
            .map(|h| self.ident(h))
            .collect::<Vec<_>>()
            .join(", ");

        // Build batched multi-row INSERTs.
        let mut statements = Vec::new();
        let mut batch: Vec<String> = Vec::with_capacity(IMPORT_BATCH);
        let mut total = 0u64;
        for rec in rdr.records() {
            let rec = rec?;
            let vals = rec
                .iter()
                .map(|f| {
                    if f.is_empty() && opts.empty_as_null {
                        "NULL".into()
                    } else {
                        self.literal(f)
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            batch.push(format!("({vals})"));
            total += 1;
            if batch.len() == IMPORT_BATCH {
                statements.push(format!(
                    "INSERT INTO {target} ({cols}) VALUES {}",
                    batch.join(", ")
                ));
                batch.clear();
            }
        }
        if !batch.is_empty() {
            statements.push(format!(
                "INSERT INTO {target} ({cols}) VALUES {}",
                batch.join(", ")
            ));
        }

        macro_rules! run {
            ($pool:expr) => {{
                let mut tx = $pool.begin().await?;
                for s in &statements {
                    sqlx::raw_sql(AssertSqlSafe(s.clone()))
                        .execute(&mut *tx)
                        .await?;
                }
                tx.commit().await?;
            }};
        }
        match self.pool() {
            PoolRef::Pg(p) => run!(p),
            PoolRef::My(p) => run!(p),
            PoolRef::Lite(p) => run!(p),
        }
        Ok(total)
    }
}
