//! Drivers that are not sqlx-based. Each exposes the same small surface:
//! `run` (script → result sets), `exec_tx` (statements in one transaction).

pub mod bigquery;
pub mod cassandra;
pub mod clickhouse;
pub mod d1;
pub mod duck;
pub mod elastic;
pub mod libsql;
pub mod mongo;
pub mod mongo_shell;
pub mod mssql;
pub mod oracle;
pub mod redis;
pub mod snowflake;

/// What a statement in `exec_tx` must affect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expect {
    Any,
    One,
    AtMostOne,
    /// exactly this many (committing a previewed statement)
    Exactly(u64),
}

impl Expect {
    pub fn check(self, n: u64) -> bool {
        match self {
            Expect::Any => true,
            Expect::One => n == 1,
            Expect::AtMostOne => n <= 1,
            Expect::Exactly(x) => n == x,
        }
    }

    /// "1 row", "at most 1 row", "12 rows"
    pub fn describe(self) -> String {
        match self {
            Expect::Any => "any number of rows".into(),
            Expect::One => "1 row".into(),
            Expect::AtMostOne => "at most 1 row".into(),
            Expect::Exactly(1) => "1 row".into(),
            Expect::Exactly(n) => format!("{n} rows"),
        }
    }
}
