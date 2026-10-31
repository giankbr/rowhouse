//! Query variables, same rules as the app's editor (`apps/desktop/src/lib/vars.ts`):
//! `{{name}}` is pasted as written (anywhere, even inside strings); `:name`
//! (outside strings and comments) becomes a literal unless the value is a
//! number, NULL, TRUE or FALSE.

struct Hit {
    start: usize,
    end: usize,
    name: String,
    raw: bool,
}

fn ident_at(b: &[u8], i: usize) -> usize {
    let mut j = i;
    if j < b.len() && (b[j].is_ascii_alphabetic() || b[j] == b'_') {
        j += 1;
        while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
            j += 1;
        }
    }
    j
}

fn scan(sql: &str, colon: bool) -> Vec<Hit> {
    let b = sql.as_bytes();
    let n = b.len();
    let mut hits = Vec::new();
    // {{name}}
    let mut i = 0;
    while let Some(p) = sql[i..].find("{{") {
        let s = i + p;
        let mut j = s + 2;
        while j < n && b[j] == b' ' {
            j += 1;
        }
        let e = ident_at(b, j);
        let mut k = e;
        while k < n && b[k] == b' ' {
            k += 1;
        }
        if e > j && sql[k..].starts_with("}}") {
            hits.push(Hit {
                start: s,
                end: k + 2,
                name: sql[j..e].to_string(),
                raw: true,
            });
            i = k + 2;
        } else {
            i = s + 2;
        }
    }
    if colon {
        let mut i = 0;
        while i < n {
            let c = b[i];
            let next = b.get(i + 1).copied();
            if c == b'-' && next == Some(b'-') {
                i = sql[i..].find('\n').map(|p| i + p).unwrap_or(n);
            } else if c == b'/' && next == Some(b'*') {
                i = sql[i + 2..].find("*/").map(|p| i + 2 + p + 2).unwrap_or(n);
            } else if c == b'\'' || c == b'"' || c == b'`' {
                let mut j = i + 1;
                while j < n {
                    if b[j] == b'\\' && c == b'\'' {
                        j += 2;
                    } else if b[j] == c {
                        if b.get(j + 1) == Some(&c) {
                            j += 2;
                        } else {
                            break;
                        }
                    } else {
                        j += 1;
                    }
                }
                i = j + 1;
            } else if c == b'$' {
                // $$…$$ or $tag$…$tag$
                let e = ident_at(b, i + 1);
                if b.get(e) == Some(&b'$') {
                    let tag = &sql[i..=e];
                    i = sql[e + 1..]
                        .find(tag)
                        .map(|p| e + 1 + p + tag.len())
                        .unwrap_or(n);
                } else {
                    i += 1;
                }
            } else if c == b':'
                && (i == 0 || b[i - 1] != b':')
                && next != Some(b':')
                && next != Some(b'=')
            {
                let e = ident_at(b, i + 1);
                let prev_word = i > 0 && (b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_');
                if e > i + 1 && !prev_word {
                    hits.push(Hit {
                        start: i,
                        end: e,
                        name: sql[i + 1..e].to_string(),
                        raw: false,
                    });
                    i = e;
                } else {
                    i += 1;
                }
            } else {
                i += 1;
            }
        }
    }
    hits.sort_by_key(|h| h.start);
    hits
}

/// Distinct variable names in order of first use.
pub fn find(sql: &str, colon: bool) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for h in scan(sql, colon) {
        if !out.contains(&h.name) {
            out.push(h.name);
        }
    }
    out
}

/// Replace variables that have a value; `literal` quotes a string for the dialect.
pub fn substitute(
    sql: &str,
    values: &std::collections::BTreeMap<String, String>,
    colon: bool,
    literal: impl Fn(&str) -> String,
) -> String {
    let mut out = String::with_capacity(sql.len());
    let mut at = 0;
    for h in scan(sql, colon) {
        if h.start < at {
            continue;
        }
        let Some(v) = values.get(&h.name) else {
            continue;
        };
        out.push_str(&sql[at..h.start]);
        if h.raw {
            out.push_str(v);
        } else {
            let t = v.trim();
            let plain = t.parse::<f64>().is_ok() && !t.is_empty()
                || ["null", "true", "false"].contains(&t.to_lowercase().as_str());
            out.push_str(&if plain { t.to_string() } else { literal(v) });
        }
        at = h.end;
    }
    out.push_str(&sql[at..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_and_substitutes() {
        let sql = "SELECT * FROM t WHERE d >= :from AND r = :region AND x::int = 1 -- :nope\n AND s = ':skip' AND n = {{n}}";
        assert_eq!(find(sql, true), ["from", "region", "n"]);
        let mut v = std::collections::BTreeMap::new();
        v.insert("from".to_string(), "2026-10-01".to_string());
        v.insert("region".to_string(), "it's".to_string());
        v.insert("n".to_string(), "42".to_string());
        let out = substitute(sql, &v, true, |s| format!("'{}'", s.replace('\'', "''")));
        assert!(out.contains("d >= '2026-10-01'"), "{out}");
        assert!(out.contains("r = 'it''s'"), "{out}");
        assert!(
            out.contains("x::int") && out.contains("':skip'") && out.ends_with("n = 42"),
            "{out}"
        );
    }
}
