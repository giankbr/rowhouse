//! Running SQL files: split into statements (strings, comments, dollar quotes,
//! MySQL `DELIMITER`), run them one by one with progress, and stop or carry
//! on at errors.

use std::{
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::Instant,
};

use serde::{Deserialize, Serialize};

use crate::{config::Driver, Connection, Error, Result};

#[derive(Debug, Clone, PartialEq)]
pub struct Statement {
    pub sql: String,
    /// 1-based line where the statement starts
    pub line: usize,
}

/// Split a script. `brackets` treats `[…]` as quoted identifiers (SQL Server).
pub fn split(script: &str, brackets: bool) -> Vec<Statement> {
    let b: Vec<char> = script.chars().collect();
    let n = b.len();
    let mut out = Vec::new();
    let mut delim: Vec<char> = vec![';'];
    let mut start = 0usize;
    let mut start_line = 1usize;
    let mut line = 1usize;
    let mut i = 0usize;
    let mut at_line_start = true;

    let flush = |from: usize, to: usize, l: usize, out: &mut Vec<Statement>| {
        let s: String = b[from..to].iter().collect();
        let t = s.trim();
        if !t.is_empty() && !only_comments(t) {
            out.push(Statement {
                sql: t.to_string(),
                line: l,
            });
        }
    };
    let starts_with_ci = |at: usize, word: &str| -> bool {
        word.chars()
            .enumerate()
            .all(|(k, w)| b.get(at + k).is_some_and(|c| c.eq_ignore_ascii_case(&w)))
    };

    while i < n {
        let c = b[i];
        // MySQL client directive: DELIMITER xx (own line)
        if at_line_start && starts_with_ci(i, "delimiter ") {
            let mut j = i + 10;
            let mut d = String::new();
            while j < n && b[j] != '\n' {
                d.push(b[j]);
                j += 1;
            }
            let d = d.trim();
            if !d.is_empty() {
                flush(start, i, start_line, &mut out);
                delim = d.chars().collect();
                i = j;
                start = i;
                start_line = line;
                continue;
            }
        }
        if !c.is_whitespace() {
            at_line_start = false;
        }
        if c == '\n' {
            line += 1;
            at_line_start = true;
            i += 1;
            // a statement's line is where its first non-blank char is
            if b[start..i].iter().all(|c| c.is_whitespace()) {
                start_line = line;
            }
            continue;
        }
        let next = b.get(i + 1).copied();
        match c {
            '-' if next == Some('-') => {
                while i < n && b[i] != '\n' {
                    i += 1;
                }
                continue;
            }
            '/' if next == Some('*') => {
                i += 2;
                while i < n && !(b[i] == '*' && b.get(i + 1) == Some(&'/')) {
                    if b[i] == '\n' {
                        line += 1;
                    }
                    i += 1;
                }
                i += 2;
                continue;
            }
            '\'' | '"' | '`' => {
                i += 1;
                while i < n {
                    if b[i] == '\n' {
                        line += 1;
                    }
                    if b[i] == '\\' && c == '\'' && !brackets {
                        i += 2;
                        continue;
                    }
                    if b[i] == c {
                        if b.get(i + 1) == Some(&c) {
                            i += 2;
                            continue;
                        }
                        break;
                    }
                    i += 1;
                }
                i += 1;
                continue;
            }
            '[' if brackets => {
                while i < n && b[i] != ']' {
                    i += 1;
                }
                i += 1;
                continue;
            }
            '$' => {
                let mut j = i + 1;
                while j < n && (b[j].is_alphanumeric() || b[j] == '_') {
                    j += 1;
                }
                if j < n && b[j] == '$' && !(j > i + 1 && b[i + 1].is_ascii_digit()) {
                    let tag = &b[i..=j];
                    let mut k = j + 1;
                    while k + tag.len() <= n && &b[k..k + tag.len()] != tag {
                        if b[k] == '\n' {
                            line += 1;
                        }
                        k += 1;
                    }
                    i = (k + tag.len()).min(n);
                    continue;
                }
                i += 1;
                continue;
            }
            _ => {}
        }
        if b[i..].starts_with(&delim) {
            flush(start, i, start_line, &mut out);
            i += delim.len();
            start = i;
            start_line = line;
            continue;
        }
        i += 1;
    }
    flush(start, n, start_line, &mut out);
    out
}

fn only_comments(s: &str) -> bool {
    s.lines().all(|l| {
        let l = l.trim();
        l.is_empty() || l.starts_with("--") || l.starts_with('#')
    }) || (s.starts_with("/*") && s.ends_with("*/") && !s[2..s.len() - 2].contains("*/"))
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunOptions {
    #[serde(default)]
    pub continue_on_error: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptError {
    pub index: usize,
    pub line: usize,
    pub statement: String,
    pub message: String,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptProgress {
    pub total: usize,
    pub done: usize,
    pub failed: usize,
    pub rows_affected: u64,
    pub elapsed_ms: u128,
    pub finished: bool,
    pub cancelled: bool,
    pub errors: Vec<ScriptError>,
    /// first line of the statement running now
    pub current: Option<String>,
}

fn preview(s: &str) -> String {
    let one: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() > 160 {
        format!("{}…", one.chars().take(160).collect::<String>())
    } else {
        one
    }
}

impl Connection {
    /// Run every statement in `path`. `progress` is called at most every ~100 ms
    /// and once at the end; set `cancel` to stop after the current statement.
    pub async fn run_sql_file(
        &self,
        path: &Path,
        opts: &RunOptions,
        cancel: &AtomicBool,
        mut progress: impl FnMut(&ScriptProgress),
    ) -> Result<ScriptProgress> {
        if matches!(self.driver(), Driver::Redis | Driver::Mongo) {
            return Err(Error::Other(
                "SQL files can't run on this database — paste the commands into a query tab".into(),
            ));
        }
        let text = std::fs::read_to_string(path)
            .map_err(|e| Error::Other(format!("can't read {}: {e}", path.display())))?;
        let stmts = if self.driver() == Driver::Cassandra {
            crate::backend::cassandra::statements(&text)
                .into_iter()
                .map(|sql| Statement { sql, line: 0 })
                .collect()
        } else {
            split(&text, self.driver() == Driver::Mssql)
        };
        let started = Instant::now();
        let mut p = ScriptProgress {
            total: stmts.len(),
            ..Default::default()
        };
        let mut last = Instant::now();
        let mut skip_copy = false;
        for (idx, st) in stmts.iter().enumerate() {
            if cancel.load(Ordering::Relaxed) {
                p.cancelled = true;
                break;
            }
            // `COPY … FROM stdin` data blocks from pg_dump can't go through a normal query
            let upper = st.sql.to_uppercase();
            let res = if skip_copy && st.sql.lines().any(|l| l.trim() == "\\.") {
                skip_copy = false;
                Err(Error::Other("COPY data block skipped".into()))
            } else if upper.starts_with("COPY ") && upper.contains("FROM STDIN") {
                skip_copy = true;
                Err(Error::Other("COPY … FROM stdin isn't supported here — use Tools → Restore (psql) for pg_dump files".into()))
            } else if self.driver() == Driver::Mssql && upper.trim() == "GO" {
                Ok(0)
            } else {
                p.current = Some(preview(&st.sql));
                self.query(&st.sql, 1)
                    .await
                    .map(|o| o.results.iter().map(|r| r.rows_affected).sum::<u64>())
            };
            p.done = idx + 1;
            match res {
                Ok(n) => p.rows_affected += n,
                Err(e) => {
                    p.failed += 1;
                    if p.errors.len() < 200 {
                        p.errors.push(ScriptError {
                            index: idx,
                            line: st.line,
                            statement: preview(&st.sql),
                            message: e.to_string(),
                        });
                    }
                    if !opts.continue_on_error {
                        break;
                    }
                }
            }
            if last.elapsed().as_millis() >= 100 {
                p.elapsed_ms = started.elapsed().as_millis();
                progress(&p);
                last = Instant::now();
            }
        }
        p.current = None;
        p.finished = true;
        p.elapsed_ms = started.elapsed().as_millis();
        progress(&p);
        Ok(p)
    }
}

#[cfg(test)]
mod tests {
    use super::split;

    fn sqls(s: &str) -> Vec<String> {
        split(s, false).into_iter().map(|s| s.sql).collect()
    }

    #[test]
    fn splits_scripts() {
        assert_eq!(
            sqls("select 1; select 'a;b' ; -- c;\nselect 2"),
            ["select 1", "select 'a;b'", "-- c;\nselect 2"]
        );
        let f = "CREATE FUNCTION f() RETURNS int AS $$ BEGIN RETURN 1; END $$ LANGUAGE plpgsql;\nSELECT f();";
        assert_eq!(sqls(f).len(), 2);
        let my = "DELIMITER ;;\nCREATE TRIGGER t BEFORE INSERT ON x FOR EACH ROW BEGIN SET NEW.a = 1; END;;\nDELIMITER ;\nINSERT INTO x VALUES (1);";
        let v = sqls(my);
        assert_eq!(v.len(), 2, "{v:?}");
        assert!(v[0].ends_with("END"));
        assert_eq!(sqls("/* only */ ; ;  -- x\n"), Vec::<String>::new());
        let lines = split("select 1;\n\n  select 2;\nselect\n3;", false);
        assert_eq!(lines.iter().map(|s| s.line).collect::<Vec<_>>(), [1, 3, 4]);
        assert_eq!(sqls("select $1, $2; select 2").len(), 2);
    }
}
