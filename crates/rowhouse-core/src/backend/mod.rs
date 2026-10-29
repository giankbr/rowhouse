//! Drivers that are not sqlx-based. Each exposes the same small surface:
//! `run` (script → result sets), `exec_tx` (statements in one transaction).

pub mod clickhouse;
pub mod duck;
pub mod mssql;

/// What a statement in `exec_tx` must affect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expect {
    Any,
    One,
    AtMostOne,
}

impl Expect {
    pub fn check(self, n: u64) -> bool {
        match self {
            Expect::Any => true,
            Expect::One => n == 1,
            Expect::AtMostOne => n <= 1,
        }
    }
}
