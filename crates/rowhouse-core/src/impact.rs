//! "Preview changes": run an UPDATE / DELETE inside a transaction that is
//! rolled back, and show which rows it would touch (before → after). Commit
//! then re-runs it and rolls back unless exactly the previewed number of rows
//! changes. See `Connection::preview_dml` / `commit_dml` in driver.rs.

use serde::Serialize;

use crate::{Error, QueryResult, Result};

/// Rows shown in a preview.
pub const PREVIEW_ROWS: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DmlKind {
    Update,
    Delete,
}

/// A single-table UPDATE / DELETE, split up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dml {
    pub kind: DmlKind,
    /// table as written, possibly with an alias (`orders o`)
    pub target: String,
    /// table name parts, unquoted (`["public", "orders"]`)
    pub table: Vec<String>,
    /// everything from WHERE on (WHERE …, ORDER BY …, LIMIT …), without RETURNING
    pub tail: String,
    pub has_limit: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Impact {
    /// "update" | "delete"
    pub kind: String,
    pub schema: String,
    pub table: String,
    /// rows the statement changed inside the rolled-back transaction
    pub affected: u64,
    /// matching rows before (up to PREVIEW_ROWS)
    pub before: QueryResult,
    /// the same rows after an UPDATE (looked up by primary key)
    pub after: Option<QueryResult>,
    /// primary key columns (empty when the table has none)
    pub key: Vec<String>,
}

/// Top-level words with their byte offsets (outside quotes, comments and parentheses).
fn top_words(sql: &str) -> Vec<(usize, String)> {
    let b = sql.as_bytes();
    let mut out = Vec::new();
    let (mut i, mut depth) = (0, 0i32);
    while i < b.len() {
        let c = b[i];
        match c {
            b'\'' | b'"' | b'`' => {
                let mut j = i + 1;
                while j < b.len() {
                    if b[j] == c {
                        if b.get(j + 1) == Some(&c) {
                            j += 2;
                            continue;
                        }
                        break;
                    }
                    if b[j] == b'\\' && c == b'\'' {
                        j += 1;
                    }
                    j += 1;
                }
                i = j + 1;
            }
            b'[' => {
                i = sql[i..].find(']').map(|p| i + p + 1).unwrap_or(b.len());
            }
            b'-' if b.get(i + 1) == Some(&b'-') => {
                i = sql[i..].find('\n').map(|p| i + p).unwrap_or(b.len());
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i = sql[i + 2..]
                    .find("*/")
                    .map(|p| i + 2 + p + 2)
                    .unwrap_or(b.len());
            }
            b'(' => {
                depth += 1;
                i += 1;
            }
            b')' => {
                depth -= 1;
                i += 1;
            }
            _ if c.is_ascii_alphabetic() || c == b'_' => {
                let s = i;
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] == b'$')
                {
                    i += 1;
                }
                // part of a dotted name (`o.where`) isn't a keyword
                let dotted = s > 0 && b[s - 1] == b'.';
                if depth == 0 && !dotted {
                    out.push((s, sql[s..i].to_lowercase()));
                }
            }
            _ => i += 1,
        }
    }
    out
}

/// `"public"."orders" o` → ["public", "orders"]
fn table_parts(target: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    for ch in target.trim().chars() {
        match (quote, ch) {
            // the alias starts after the first unquoted space
            (None, c) if c.is_whitespace() => break,
            (None, '"' | '`' | '[') => quote = Some(if ch == '[' { ']' } else { ch }),
            (Some(q), c) if c == q => quote = None,
            (None, '.') => parts.push(std::mem::take(&mut cur)),
            (_, c) => cur.push(c),
        }
    }
    parts.push(cur);
    parts.retain(|p| !p.is_empty());
    parts
}

/// Parse one single-table UPDATE / DELETE. `None` for anything else.
pub fn parse(sql: &str) -> Result<Option<Dml>> {
    let stmts = crate::guard::split_statements(sql);
    if stmts.len() != 1 {
        return Ok(None);
    }
    let s = stmts[0].trim().trim_end_matches(';').trim();
    let w = top_words(s);
    let Some((_, first)) = w.first() else {
        return Ok(None);
    };
    let unsupported = |why: &str| {
        Err(Error::Other(format!(
            "preview works on single-table UPDATE / DELETE — {why}"
        )))
    };
    let pos = |word: &str, from: usize| {
        w.iter()
            .skip(from)
            .find(|(_, x)| x == word)
            .map(|(p, _)| *p)
    };
    let (kind, target, rest_at) = match first.as_str() {
        "update" => {
            let Some(set) = pos("set", 1) else {
                return Ok(None);
            };
            let target = s[w[0].0 + 6..set].trim().to_string();
            if target.contains(',') || w.iter().any(|(p, x)| *p < set && x == "join") {
                return unsupported("this one updates several tables");
            }
            if w.iter().any(|(p, x)| *p > set && x == "from") {
                return unsupported("UPDATE … FROM joins another table");
            }
            (DmlKind::Update, target, set)
        }
        "delete" => {
            let Some(from) = pos("from", 1) else {
                return unsupported("DELETE without FROM");
            };
            let end = w
                .iter()
                .find(|(p, x)| {
                    *p > from
                        && matches!(
                            x.as_str(),
                            "where" | "using" | "returning" | "order" | "limit"
                        )
                })
                .map(|(p, _)| *p)
                .unwrap_or(s.len());
            if w.iter().any(|(p, x)| *p > from && x == "using") {
                return unsupported("DELETE … USING joins another table");
            }
            (DmlKind::Delete, s[from + 4..end].trim().to_string(), from)
        }
        _ => return Ok(None),
    };
    if target.is_empty() || target.contains(',') {
        return unsupported("several tables");
    }
    let tail_start = w
        .iter()
        .find(|(p, x)| *p > rest_at && matches!(x.as_str(), "where" | "order" | "limit"))
        .map(|(p, _)| *p);
    let tail_end = w
        .iter()
        .find(|(p, x)| *p > rest_at && x == "returning")
        .map(|(p, _)| *p)
        .unwrap_or(s.len());
    let tail = tail_start
        .map(|t| s[t..tail_end].trim().to_string())
        .unwrap_or_default();
    let has_limit = w
        .iter()
        .any(|(p, x)| tail_start.is_some_and(|t| *p >= t) && *p < tail_end && x == "limit");
    Ok(Some(Dml {
        kind,
        table: table_parts(&target),
        target,
        tail,
        has_limit,
    }))
}

impl Dml {
    /// The rows the statement would touch.
    pub fn select_sql(&self) -> String {
        let mut q = format!("SELECT * FROM {}", self.target);
        if !self.tail.is_empty() {
            q.push(' ');
            q.push_str(&self.tail);
        }
        if !self.has_limit {
            q.push_str(&format!(" LIMIT {}", PREVIEW_ROWS + 1));
        }
        q
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_updates_and_deletes() {
        let d = parse("UPDATE public.orders SET status = 'paid', note = 'where?' WHERE id IN (SELECT id FROM x WHERE y) RETURNING *;")
            .unwrap()
            .unwrap();
        assert_eq!(d.kind, DmlKind::Update);
        assert_eq!(d.table, ["public", "orders"]);
        assert_eq!(d.tail, "WHERE id IN (SELECT id FROM x WHERE y)");
        assert_eq!(
            d.select_sql(),
            "SELECT * FROM public.orders WHERE id IN (SELECT id FROM x WHERE y) LIMIT 501"
        );

        let d = parse("delete from \"Sales\".\"Order Items\" where qty = 0 order by id limit 10")
            .unwrap()
            .unwrap();
        assert_eq!(d.kind, DmlKind::Delete);
        assert_eq!(d.table, ["Sales", "Order Items"]);
        assert_eq!(
            d.select_sql(),
            "SELECT * FROM \"Sales\".\"Order Items\" where qty = 0 order by id limit 10"
        );

        let d = parse("UPDATE users u SET u.active = 0").unwrap().unwrap();
        assert_eq!((d.target.as_str(), d.tail.as_str()), ("users u", ""));
        assert_eq!(d.table, ["users"]);

        assert!(parse("SELECT 1").unwrap().is_none());
        assert!(parse("UPDATE a SET x = 1; UPDATE b SET y = 2")
            .unwrap()
            .is_none());
        assert!(parse("UPDATE a SET x = b.x FROM b WHERE a.id = b.id").is_err());
        assert!(parse("DELETE FROM a USING b WHERE a.id = b.id").is_err());
        assert!(parse("UPDATE a JOIN b ON a.id = b.id SET a.x = 1").is_err());
    }
}
