//! Conservative read-only classifier.
//!
//! This is the *first* line of defence. [`crate::Connection::query_read_only`]
//! additionally runs the statement inside a read-only transaction / `query_only`
//! session, so a statement that slips past this check is still refused by the DB.

/// Statement keywords that never modify data.
const READ_KEYWORDS: &[&str] = &[
    "select", "with", "show", "explain", "describe", "desc", "values", "table",
];

/// Keywords that turn an otherwise read-looking statement into a write
/// (`WITH x AS (DELETE ...)`, `SELECT ... INTO new_table`, etc).
const WRITE_KEYWORDS: &[&str] = &[
    "insert", "update", "delete", "merge", "upsert", "into", "create", "alter", "drop", "truncate",
    "grant", "revoke", "copy", "call", "lock", "vacuum", "attach", "detach", "reindex",
];

/// Split a script into statements, with comments and string literals blanked out.
/// Returns the *normalised* (lowercased, literal-free) text of each statement.
pub fn normalised_statements(sql: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let chars: Vec<char> = sql.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        match c {
            '-' if next == Some('-') => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
                cur.push(' ');
                continue;
            }
            '#' => {
                // MySQL line comment
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
                cur.push(' ');
                continue;
            }
            '/' if next == Some('*') => {
                i += 2;
                while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                    i += 1;
                }
                i += 2;
                cur.push(' ');
                continue;
            }
            '\'' | '"' | '`' => {
                let q = c;
                i += 1;
                while i < chars.len() {
                    if chars[i] == q {
                        if chars.get(i + 1) == Some(&q) {
                            i += 2; // escaped quote
                            continue;
                        }
                        break;
                    }
                    if chars[i] == '\\' && q == '\'' {
                        i += 1;
                    }
                    i += 1;
                }
                i += 1;
                // quoted identifiers / literals become a neutral token
                cur.push_str(" _ ");
                continue;
            }
            '$' => {
                // Postgres dollar-quoting: $tag$ ... $tag$
                let start = i;
                let mut j = i + 1;
                while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '_') {
                    j += 1;
                }
                if j < chars.len() && chars[j] == '$' {
                    let tag: String = chars[start..=j].iter().collect();
                    let rest: String = chars[j + 1..].iter().collect();
                    if let Some(end) = rest.find(&tag) {
                        i = j + 1 + rest[..end].chars().count() + tag.chars().count();
                        cur.push_str(" _ ");
                        continue;
                    }
                }
                cur.push(c);
            }
            ';' => {
                if !cur.trim().is_empty() {
                    out.push(cur.trim().to_lowercase());
                }
                cur.clear();
            }
            _ => cur.push(c),
        }
        i += 1;
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_lowercase());
    }
    out
}

/// Split a script into statements at top-level `;`, keeping the original text
/// (comments, literals) intact. Used by drivers without multi-statement support.
pub fn split_statements(sql: &str) -> Vec<String> {
    let chars: Vec<char> = sql.chars().collect();
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut i = 0usize;
    let push = |out: &mut Vec<String>, from: usize, to: usize| {
        let stmt: String = chars[from..to].iter().collect();
        // skip pieces that are only whitespace / comments
        if !normalised_statements(&stmt).is_empty() {
            out.push(stmt.trim().to_string());
        }
    };
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        match c {
            '-' if next == Some('-') => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '/' if next == Some('*') => {
                i += 2;
                while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                    i += 1;
                }
                i += 2;
            }
            '\'' | '"' | '`' | '[' => {
                let close = if c == '[' { ']' } else { c };
                i += 1;
                while i < chars.len() {
                    if chars[i] == close {
                        if chars.get(i + 1) == Some(&close) {
                            i += 2;
                            continue;
                        }
                        break;
                    }
                    if chars[i] == '\\' && c == '\'' {
                        i += 1;
                    }
                    i += 1;
                }
                i += 1;
            }
            '$' => {
                let mut j = i + 1;
                while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '_') {
                    j += 1;
                }
                if j < chars.len() && chars[j] == '$' {
                    let tag: String = chars[i..=j].iter().collect();
                    let rest: String = chars[j + 1..].iter().collect();
                    if let Some(end) = rest.find(&tag) {
                        i = j + 1 + rest[..end].chars().count() + tag.chars().count();
                        continue;
                    }
                }
                i += 1;
            }
            ';' => {
                push(&mut out, start, i);
                i += 1;
                start = i;
            }
            _ => i += 1,
        }
    }
    if start < chars.len() {
        push(&mut out, start, chars.len().min(i.max(start)));
    }
    out
}

/// First keyword of a statement, lowercased (comments skipped).
pub fn first_keyword(stmt: &str) -> String {
    normalised_statements(stmt)
        .first()
        .and_then(|s| words(s).next().map(str::to_string))
        .unwrap_or_default()
}

/// A statement that destroys data or schema and deserves a typed confirmation.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Destructive {
    /// "drop" | "truncate" | "delete-all" | "update-all" | "drop-column"
    pub kind: String,
    /// Object name as written (unquoted, may be schema-qualified).
    pub object: String,
    pub statement: String,
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Word(String),  // bare word, original case
    Ident(String), // quoted identifier, unquoted
    Dot,
    Other,
}

fn tokenize(stmt: &str) -> Vec<Tok> {
    let c: Vec<char> = stmt.chars().collect();
    let mut i = 0;
    let mut out = Vec::new();
    while i < c.len() {
        let ch = c[i];
        if ch.is_whitespace() {
            i += 1;
        } else if ch == '-' && c.get(i + 1) == Some(&'-') {
            while i < c.len() && c[i] != '\n' {
                i += 1;
            }
        } else if ch == '/' && c.get(i + 1) == Some(&'*') {
            i += 2;
            while i + 1 < c.len() && !(c[i] == '*' && c[i + 1] == '/') {
                i += 1;
            }
            i += 2;
        } else if ch == '"' || ch == '`' || ch == '[' {
            let close = if ch == '[' { ']' } else { ch };
            let mut j = i + 1;
            let mut s = String::new();
            while j < c.len() {
                if c[j] == close {
                    if c.get(j + 1) == Some(&close) {
                        s.push(close);
                        j += 2;
                        continue;
                    }
                    break;
                }
                s.push(c[j]);
                j += 1;
            }
            out.push(Tok::Ident(s));
            i = j + 1;
        } else if ch == '\'' {
            let mut j = i + 1;
            while j < c.len() && !(c[j] == '\'' && c.get(j + 1) != Some(&'\'')) {
                if c[j] == '\'' {
                    j += 1;
                }
                j += 1;
            }
            out.push(Tok::Other);
            i = j + 1;
        } else if ch.is_alphanumeric() || ch == '_' || ch == '$' {
            let mut j = i;
            while j < c.len() && (c[j].is_alphanumeric() || c[j] == '_' || c[j] == '$') {
                j += 1;
            }
            out.push(Tok::Word(c[i..j].iter().collect()));
            i = j;
        } else {
            out.push(if ch == '.' { Tok::Dot } else { Tok::Other });
            i += 1;
        }
    }
    out
}

fn kw(t: Option<&Tok>, k: &str) -> bool {
    matches!(t, Some(Tok::Word(w)) if w.eq_ignore_ascii_case(k))
}

/// Read a (possibly dotted) object name starting at `i`.
fn object_at(toks: &[Tok], mut i: usize) -> Option<String> {
    let mut parts = Vec::new();
    loop {
        match toks.get(i) {
            Some(Tok::Word(w)) | Some(Tok::Ident(w)) => parts.push(w.clone()),
            _ => break,
        }
        if toks.get(i + 1) == Some(&Tok::Dot) {
            i += 2;
        } else {
            break;
        }
    }
    (!parts.is_empty()).then(|| parts.join("."))
}

fn has_kw(toks: &[Tok], k: &str) -> bool {
    toks.iter().any(|t| kw(Some(t), k))
}

/// Find statements that drop / truncate objects, or DELETE / UPDATE without WHERE.
pub fn destructive(sql: &str) -> Vec<Destructive> {
    let mut out = Vec::new();
    for stmt in split_statements(sql) {
        let t = tokenize(&stmt);
        let mut push = |kind: &str, object: Option<String>| {
            out.push(Destructive {
                kind: kind.into(),
                object: object.unwrap_or_else(|| "?".into()),
                statement: stmt.clone(),
            })
        };
        if kw(t.first(), "drop") {
            // DROP [TEMPORARY] TABLE|VIEW|SCHEMA|DATABASE|INDEX|… [IF EXISTS] name
            let mut i = 1;
            while matches!(t.get(i), Some(Tok::Word(w)) if ["temporary", "temp", "materialized", "unlogged"].contains(&w.to_lowercase().as_str()))
            {
                i += 1;
            }
            i += 1; // object type
            if kw(t.get(i), "if") && kw(t.get(i + 1), "exists") {
                i += 2;
            }
            push("drop", object_at(&t, i));
        } else if kw(t.first(), "truncate") {
            let i = if kw(t.get(1), "table") { 2 } else { 1 };
            push("truncate", object_at(&t, i));
        } else if kw(t.first(), "delete") && !has_kw(&t, "where") {
            let i = if kw(t.get(1), "from") { 2 } else { 1 };
            push("delete-all", object_at(&t, i));
        } else if kw(t.first(), "update") && !has_kw(&t, "where") {
            push("update-all", object_at(&t, 1));
        } else if kw(t.first(), "alter") && kw(t.get(1), "table") {
            let mut i = 2;
            if kw(t.get(i), "if") && kw(t.get(i + 1), "exists") {
                i += 2;
            }
            let obj = object_at(&t, i);
            let dropping = t.windows(2).any(|w| {
                kw(Some(&w[0]), "drop") && !kw(Some(&w[1]), "default") && !kw(Some(&w[1]), "not")
            });
            if dropping {
                push("drop-column", obj);
            }
        }
    }
    out
}

fn words(stmt: &str) -> impl Iterator<Item = &str> {
    stmt.split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty())
}

/// Returns `Ok(())` when every statement in `sql` is read-only, otherwise an
/// explanation of the first offending statement.
pub fn check_read_only(sql: &str) -> Result<(), String> {
    let stmts = normalised_statements(sql);
    if stmts.is_empty() {
        return Err("empty query".into());
    }
    for stmt in &stmts {
        let first = words(stmt).next().unwrap_or_default();
        if !READ_KEYWORDS.contains(&first) {
            return Err(format!(
                "`{first}` statements are not allowed in read-only mode"
            ));
        }
        // EXPLAIN ANALYZE actually executes the statement.
        if first == "explain" && words(stmt).nth(1) == Some("analyze") {
            if let Some(w) = words(stmt).skip(2).find(|w| WRITE_KEYWORDS.contains(w)) {
                return Err(format!(
                    "EXPLAIN ANALYZE of a `{w}` statement is not allowed"
                ));
            }
            continue;
        }
        if let Some(w) = words(stmt).skip(1).find(|w| WRITE_KEYWORDS.contains(w)) {
            return Err(format!("`{w}` is not allowed in read-only mode"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_reads() {
        assert!(check_read_only("select * from users").is_ok());
        assert!(check_read_only("  -- hi\nSELECT 1; select 'delete from x'").is_ok());
        assert!(check_read_only("with a as (select 1) select * from a").is_ok());
        assert!(check_read_only("explain select 1").is_ok());
        assert!(check_read_only("show tables").is_ok());
        assert!(check_read_only("select replace(name, 'a', 'b') from t").is_ok());
        assert!(check_read_only("select $$drop table x$$").is_ok());
    }

    #[test]
    fn splits_keeping_text() {
        let s = split_statements("select 'a;b' -- c;\n; /* x; */ select 2;; ");
        assert_eq!(s, vec!["select 'a;b' -- c;", "/* x; */ select 2"]);
        assert_eq!(
            split_statements("select [a;b] from t"),
            vec!["select [a;b] from t"]
        );
        assert_eq!(first_keyword("  -- hi\n  SELECT 1"), "select");
    }

    #[test]
    fn finds_destructive_statements() {
        let d = destructive(
            "select 1; DROP TABLE IF EXISTS \"public\".\"Users\"; truncate orders; \
             delete from logs; delete from logs where id = 1; UPDATE t SET a = 1; \
             alter table t drop column c; alter table t alter column c drop default; \
             DROP SCHEMA app CASCADE; -- drop table nope\n select 'drop table x'",
        );
        let got: Vec<(&str, &str)> = d
            .iter()
            .map(|x| (x.kind.as_str(), x.object.as_str()))
            .collect();
        assert_eq!(
            got,
            vec![
                ("drop", "public.Users"),
                ("truncate", "orders"),
                ("delete-all", "logs"),
                ("update-all", "t"),
                ("drop-column", "t"),
                ("drop", "app"),
            ]
        );
        assert!(destructive("update t set a = 'where' ").len() == 1);
        assert!(destructive("DELETE FROM [dbo].[x] WHERE 1=1").is_empty());
    }

    #[test]
    fn blocks_writes() {
        assert!(check_read_only("delete from users").is_err());
        assert!(check_read_only("select 1; drop table users").is_err());
        assert!(check_read_only("with d as (delete from t returning *) select * from d").is_err());
        assert!(check_read_only("select * into t2 from t").is_err());
        assert!(check_read_only("explain analyze delete from t").is_err());
        assert!(check_read_only("/* select */ update t set a=1").is_err());
        assert!(check_read_only("").is_err());
    }
}
