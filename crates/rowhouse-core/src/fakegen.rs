//! Dummy data: a generator per column (guessed from its name and type, or
//! picked by the user), previewed, then inserted in batches. Foreign key
//! columns pick values that exist in the referenced table.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    backend::Expect, config::Driver, ColumnInfo, Connection, Error, QueryResult, Result,
    ResultColumn,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
pub enum Gen {
    /// leave out of the INSERT (database default / identity)
    Skip,
    Null,
    Fixed {
        value: String,
    },
    Sequence {
        start: i64,
    },
    Int {
        min: i64,
        max: i64,
    },
    Float {
        min: f64,
        max: f64,
        decimals: u32,
    },
    Bool,
    FirstName,
    LastName,
    FullName,
    Email,
    Phone,
    City,
    Country,
    Company,
    Url,
    Uuid,
    Date {
        from: String,
        to: String,
    },
    Timestamp {
        from: String,
        to: String,
    },
    Words {
        min: u32,
        max: u32,
    },
    Json,
    Pick {
        values: Vec<String>,
    },
    /// a value that exists in another table (foreign keys)
    Reference {
        schema: String,
        table: String,
        column: String,
    },
    /// NULL in about `percent`% of rows, otherwise `inner`
    Nullable {
        percent: u32,
        inner: Box<Gen>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnGen {
    pub column: String,
    pub data_type: String,
    pub gen: Gen,
}

// ---------------------------------------------------------------------------
// random source (xorshift; seeded per run)
// ---------------------------------------------------------------------------

pub struct Rng(u64);
impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed.max(1))
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn range(&mut self, lo: i64, hi: i64) -> i64 {
        if hi <= lo {
            return lo;
        }
        lo + (self.next() % ((hi - lo + 1) as u64)) as i64
    }
    fn f(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[(self.next() % xs.len() as u64) as usize]
    }
}

const FIRST: &[&str] = &[
    "Ayu", "Budi", "Citra", "Dimas", "Eka", "Fajar", "Gita", "Hana", "Indra", "Joko", "Kirana",
    "Lukas", "Maya", "Nadia", "Oscar", "Putri", "Rizky", "Sari", "Tono", "Wulan", "Alex", "Emma",
    "Liam", "Olivia", "Noah", "Mia", "Lucas", "Sofia", "Hiro", "Yuki", "Mateo", "Lena", "Omar",
    "Zara", "Ivan", "Elena",
];
const LAST: &[&str] = &[
    "Santoso", "Wijaya", "Pratama", "Lestari", "Hidayat", "Nugroho", "Saputra", "Kusuma", "Smith",
    "Johnson", "Garcia", "Müller", "Rossi", "Tanaka", "Kim", "Nguyen", "Silva", "Novak", "Dubois",
    "Hansen", "Okafor", "Patel", "Cohen", "Ivanova",
];
const CITIES: &[&str] = &[
    "Jakarta",
    "Bandung",
    "Surabaya",
    "Yogyakarta",
    "Denpasar",
    "Medan",
    "Singapore",
    "Tokyo",
    "Berlin",
    "Lisbon",
    "Austin",
    "Toronto",
    "Nairobi",
    "São Paulo",
    "Melbourne",
    "Seoul",
    "Paris",
    "Amsterdam",
];
const COUNTRIES: &[&str] = &[
    "Indonesia",
    "Singapore",
    "Japan",
    "Germany",
    "Portugal",
    "United States",
    "Canada",
    "Kenya",
    "Brazil",
    "Australia",
    "South Korea",
    "France",
    "Netherlands",
    "India",
    "Mexico",
];
const COMPANY_A: &[&str] = &[
    "Blue", "North", "Bright", "Quiet", "Swift", "Iron", "Green", "Silver", "Nusa", "Delta",
    "Atlas",
];
const COMPANY_B: &[&str] = &[
    "Labs",
    "Works",
    "Systems",
    "Logistics",
    "Foods",
    "Studio",
    "Capital",
    "Health",
    "Cloud",
    "Retail",
];
const WORDS: &[&str] = &[
    "lorem", "ipsum", "dolor", "sit", "amet", "quick", "order", "ready", "update", "note",
    "shipping", "invoice", "review", "customer", "pending", "great", "service", "account",
    "monthly", "report", "support", "team", "delivery", "status",
];
const DOMAINS: &[&str] = &["example.com", "example.org", "mail.test", "acme.test"];

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}
fn parse_day(s: &str) -> Option<i64> {
    let mut it = s.trim().split('-');
    let (y, m, d) = (
        it.next()?.parse().ok()?,
        it.next()?.parse().ok()?,
        it.next()?.get(..2)?.parse().ok()?,
    );
    Some(days_from_civil(y, m, d))
}

/// Guess a generator from the column's name and type.
pub fn guess(c: &ColumnInfo) -> Gen {
    let n = c.name.to_lowercase();
    let t = c.data_type.to_lowercase();
    let has_default = c.default.as_deref().is_some_and(|d| !d.trim().is_empty());
    let auto_id = t.contains("identity")
        || t.contains("serial")
        || t.contains("auto_increment")
        || c.default
            .as_deref()
            .is_some_and(|d| d.contains("nextval") || d.to_lowercase().contains("identity"));
    if c.primary_key && (auto_id || (has_default && !t.contains("char") && !t.contains("text"))) {
        return Gen::Skip;
    }
    if c.primary_key && t.contains("int") {
        return Gen::Sequence { start: 1 };
    }
    let this_year = crate::schedules::now() / 31_556_952 + 1970;
    let ts = || Gen::Timestamp {
        from: format!("{}-01-01", this_year - 2),
        to: format!("{this_year}-12-31"),
    };
    let g = if t.contains("uuid") || t.contains("uniqueidentifier") {
        Gen::Uuid
    } else if t.contains("bool") || t == "bit" || t == "tinyint(1)" {
        Gen::Bool
    } else if t.contains("json") {
        Gen::Json
    } else if t.contains("timestamp") || t.contains("datetime") || n.ends_with("_at") {
        ts()
    } else if t == "date" || n.ends_with("_on") || n.ends_with("date") {
        Gen::Date {
            from: format!("{}-01-01", this_year - 2),
            to: format!("{this_year}-12-31"),
        }
    } else if n.contains("email") {
        Gen::Email
    } else if n.contains("phone") || n.contains("mobile") {
        Gen::Phone
    } else if n == "first_name" || n == "firstname" || n == "given_name" {
        Gen::FirstName
    } else if n == "last_name" || n == "lastname" || n == "surname" || n == "family_name" {
        Gen::LastName
    } else if n.contains("name") && (n.contains("company") || n.contains("org")) || n == "company" {
        Gen::Company
    } else if n == "name"
        || n.contains("full_name")
        || n.contains("fullname")
        || n == "username"
        || n.ends_with("_name")
    {
        Gen::FullName
    } else if n.contains("city") {
        Gen::City
    } else if n.contains("country") {
        Gen::Country
    } else if n.contains("url") || n.contains("website") || n.contains("link") {
        Gen::Url
    } else if n == "status" || n == "state" {
        Gen::Pick {
            values: vec!["active".into(), "pending".into(), "inactive".into()],
        }
    } else if t.contains("int") || t.contains("serial") {
        if n.contains("qty") || n.contains("quantity") || n.contains("count") {
            Gen::Int { min: 1, max: 20 }
        } else if n.contains("age") {
            Gen::Int { min: 18, max: 80 }
        } else {
            Gen::Int { min: 1, max: 1000 }
        }
    } else if t.contains("numeric")
        || t.contains("decimal")
        || t.contains("real")
        || t.contains("double")
        || t.contains("float")
        || t.contains("money")
    {
        Gen::Float {
            min: 1.0,
            max: 1000.0,
            decimals: 2,
        }
    } else if t.contains("char") || t.contains("text") || t.contains("string") || t.is_empty() {
        let max = t
            .split(|c| c == '(' || c == ')')
            .nth(1)
            .and_then(|x| x.parse::<u32>().ok())
            .unwrap_or(255);
        if max < 12 {
            Gen::Pick {
                values: vec!["A".into(), "B".into(), "C".into()],
            }
        } else {
            Gen::Words {
                min: 2,
                max: if max < 60 { 3 } else { 8 },
            }
        }
    } else if has_default {
        Gen::Skip
    } else {
        Gen::Null
    };
    if c.nullable && !matches!(g, Gen::Skip | Gen::Null) && !c.primary_key {
        Gen::Nullable {
            percent: 10,
            inner: Box::new(g),
        }
    } else {
        g
    }
}

/// Values for one generator; `refs` holds looked-up values for Reference gens.
fn value(g: &Gen, row: usize, r: &mut Rng, refs: &[Value]) -> Value {
    match g {
        Gen::Skip | Gen::Null => Value::Null,
        Gen::Fixed { value } => json!(value),
        Gen::Sequence { start } => json!(start + row as i64),
        Gen::Int { min, max } => json!(r.range(*min, *max)),
        Gen::Float { min, max, decimals } => {
            let v = min + r.f() * (max - min);
            let p = 10f64.powi(*decimals as i32);
            json!((v * p).round() / p)
        }
        Gen::Bool => json!(r.next() % 2 == 0),
        Gen::FirstName => json!(r.pick(FIRST)),
        Gen::LastName => json!(r.pick(LAST)),
        Gen::FullName => json!(format!("{} {}", r.pick(FIRST), r.pick(LAST))),
        Gen::Email => json!(format!(
            "{}.{}{}@{}",
            r.pick(FIRST).to_lowercase(),
            r.pick(LAST).to_lowercase().replace('ü', "u"),
            row,
            r.pick(DOMAINS)
        )),
        Gen::Phone => json!(format!(
            "+62 8{} {:04} {:04}",
            r.range(11, 99),
            r.range(0, 9999),
            r.range(0, 9999)
        )),
        Gen::City => json!(r.pick(CITIES)),
        Gen::Country => json!(r.pick(COUNTRIES)),
        Gen::Company => json!(format!("{} {}", r.pick(COMPANY_A), r.pick(COMPANY_B))),
        Gen::Url => json!(format!(
            "https://{}.example.com/{}",
            r.pick(COMPANY_A).to_lowercase(),
            r.pick(WORDS)
        )),
        Gen::Uuid => {
            let (a, b) = (r.next(), r.next());
            json!(format!(
                "{:08x}-{:04x}-4{:03x}-{:04x}-{:012x}",
                a >> 32,
                (a >> 16) & 0xffff,
                a & 0xfff,
                (b >> 48) & 0x3fff | 0x8000,
                b & 0xffff_ffff_ffff
            ))
        }
        Gen::Date { from, to } | Gen::Timestamp { from, to } => {
            let (a, b) = (
                parse_day(from).unwrap_or(19_000),
                parse_day(to).unwrap_or(20_500),
            );
            let (y, m, d) = civil_from_days(r.range(a.min(b), a.max(b)));
            if matches!(g, Gen::Date { .. }) {
                json!(format!("{y:04}-{m:02}-{d:02}"))
            } else {
                json!(format!(
                    "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}",
                    r.range(0, 23),
                    r.range(0, 59),
                    r.range(0, 59)
                ))
            }
        }
        Gen::Words { min, max } => {
            let n = r.range(*min as i64, (*max).max(*min) as i64);
            let mut w: Vec<&str> = (0..n).map(|_| *r.pick(WORDS)).collect();
            if let Some(f) = w.first_mut() {
                let cap: String = f
                    .chars()
                    .take(1)
                    .flat_map(char::to_uppercase)
                    .chain(f.chars().skip(1))
                    .collect();
                return json!(std::iter::once(cap)
                    .chain(w.iter().skip(1).map(|s| s.to_string()))
                    .collect::<Vec<_>>()
                    .join(" "));
            }
            json!("")
        }
        Gen::Json => {
            json!({ "source": "rowhouse", "score": r.range(1, 100), "tags": [r.pick(WORDS), r.pick(WORDS)] })
        }
        Gen::Pick { values } => {
            if values.is_empty() {
                Value::Null
            } else {
                json!(r.pick(values))
            }
        }
        Gen::Reference { .. } => {
            if refs.is_empty() {
                Value::Null
            } else {
                r.pick(refs).clone()
            }
        }
        Gen::Nullable { percent, inner } => {
            if r.range(0, 99) < *percent as i64 {
                Value::Null
            } else {
                value(inner, row, r, refs)
            }
        }
    }
}

fn reference_of(g: &Gen) -> Option<(&str, &str, &str)> {
    match g {
        Gen::Reference {
            schema,
            table,
            column,
        } => Some((schema, table, column)),
        Gen::Nullable { inner, .. } => reference_of(inner),
        _ => None,
    }
}

impl Connection {
    /// Generators for each column: guessed, with foreign keys pointing at existing rows.
    pub async fn fake_plan(&self, schema: &str, table: &str) -> Result<Vec<ColumnGen>> {
        let cols = self.describe_table(schema, table).await?;
        let fks = self.foreign_keys().await.unwrap_or_default();
        Ok(cols
            .iter()
            .map(|c| {
                let fk = fks.iter().find(|f| {
                    f.schema == schema
                        && f.table == table
                        && f.columns.len() == 1
                        && f.columns[0] == c.name
                });
                let gen = match fk {
                    Some(f) => {
                        let r = Gen::Reference {
                            schema: f.ref_schema.clone(),
                            table: f.ref_table.clone(),
                            column: f.ref_columns[0].clone(),
                        };
                        if c.nullable {
                            Gen::Nullable {
                                percent: 5,
                                inner: Box::new(r),
                            }
                        } else {
                            r
                        }
                    }
                    None => guess(c),
                };
                ColumnGen {
                    column: c.name.clone(),
                    data_type: c.data_type.clone(),
                    gen,
                }
            })
            .collect())
    }

    async fn reference_values(&self, plan: &[ColumnGen]) -> Result<Vec<Vec<Value>>> {
        let mut out = Vec::with_capacity(plan.len());
        for cg in plan {
            out.push(match reference_of(&cg.gen) {
                Some((s, t, c)) => {
                    let sql = format!(
                        "SELECT DISTINCT {} FROM {}",
                        self.ident(c),
                        self.target(s, t)
                    );
                    let r = self.query_admin(&sql).await?;
                    let vals: Vec<Value> = r
                        .results
                        .into_iter()
                        .next()
                        .map(|r| {
                            r.rows
                                .into_iter()
                                .map(|mut x| x.remove(0))
                                .take(5000)
                                .collect()
                        })
                        .unwrap_or_default();
                    if vals.is_empty() {
                        return Err(Error::Other(format!(
                            "{} references {t}, which has no rows yet — fill {t} first",
                            cg.column
                        )));
                    }
                    vals
                }
                None => vec![],
            });
        }
        Ok(out)
    }

    fn fake_rows(
        plan: &[ColumnGen],
        refs: &[Vec<Value>],
        from: usize,
        n: usize,
        rng: &mut Rng,
    ) -> Vec<Vec<Value>> {
        (from..from + n)
            .map(|i| {
                plan.iter()
                    .zip(refs)
                    .filter(|(c, _)| c.gen != Gen::Skip)
                    .map(|(c, rv)| value(&c.gen, i, rng, rv))
                    .collect()
            })
            .collect()
    }

    /// A few sample rows, as a result grid.
    pub async fn fake_preview(
        &self,
        plan: &[ColumnGen],
        rows: usize,
        seed: u64,
    ) -> Result<QueryResult> {
        let refs = self.reference_values(plan).await?;
        let mut rng = Rng::new(seed);
        Ok(QueryResult {
            columns: plan
                .iter()
                .filter(|c| c.gen != Gen::Skip)
                .map(|c| ResultColumn {
                    name: c.column.clone(),
                    data_type: c.data_type.clone(),
                })
                .collect(),
            rows: Self::fake_rows(plan, &refs, 0, rows, &mut rng),
            rows_affected: 0,
            truncated: false,
        })
    }

    /// Insert `count` generated rows in batches; returns rows inserted.
    pub async fn fake_insert(
        &self,
        schema: &str,
        table: &str,
        plan: &[ColumnGen],
        count: usize,
        seed: u64,
        mut progress: impl FnMut(usize),
    ) -> Result<usize> {
        if self.config.read_only {
            return Err(Error::ReadOnly("connection is read-only".into()));
        }
        if matches!(self.driver(), Driver::Redis | Driver::Mongo) {
            return Err(Error::Other(
                "dummy data works with SQL databases and Cassandra".into(),
            ));
        }
        let cols: Vec<&ColumnGen> = plan.iter().filter(|c| c.gen != Gen::Skip).collect();
        if cols.is_empty() {
            return Err(Error::Other(
                "every column is skipped — nothing to insert".into(),
            ));
        }
        let refs = self.reference_values(plan).await?;
        let mut rng = Rng::new(seed);
        let target = self.target(schema, table);
        let names = cols
            .iter()
            .map(|c| self.ident(&c.column))
            .collect::<Vec<_>>()
            .join(", ");
        let batch = if self.driver() == Driver::Cassandra {
            1
        } else {
            500usize.min(65_000 / cols.len().max(1)).max(1)
        };
        let mut done = 0;
        while done < count {
            let n = batch.min(count - done);
            let rows = Self::fake_rows(plan, &refs, done, n, &mut rng);
            let tuples: Vec<String> = rows
                .iter()
                .map(|r| {
                    format!(
                        "({})",
                        r.iter()
                            .map(|v| self.sql_value(v))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                })
                .collect();
            let stmt = format!(
                "INSERT INTO {target} ({names}) VALUES {}",
                tuples.join(", ")
            );
            self.exec_tx(vec![(Expect::Any, stmt)]).await?;
            done += n;
            progress(done);
        }
        Ok(done)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guesses_and_dates() {
        let c = |n: &str, t: &str, pk: bool, def: Option<&str>| ColumnInfo {
            name: n.into(),
            data_type: t.into(),
            nullable: false,
            default: def.map(Into::into),
            primary_key: pk,
        };
        assert_eq!(
            guess(&c("id", "bigint", true, Some("nextval('x')"))),
            Gen::Skip
        );
        assert_eq!(guess(&c("email", "text", false, None)), Gen::Email);
        assert!(matches!(
            guess(&c("created_at", "timestamptz", false, Some("now()"))),
            Gen::Timestamp { .. }
        ));
        assert_eq!(guess(&c("active", "boolean", false, None)), Gen::Bool);
        assert_eq!(
            civil_from_days(days_from_civil(2026, 10, 31)),
            (2026, 10, 31)
        );
        let mut r = Rng::new(7);
        let v = value(
            &Gen::Date {
                from: "2025-01-01".into(),
                to: "2025-01-31".into(),
            },
            0,
            &mut r,
            &[],
        );
        assert!(v.as_str().unwrap().starts_with("2025-01-"));
    }
}
