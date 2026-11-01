//! Row-level diff between two results of the same query (the app's
//! `lib/diff.ts` follows the same rules).
//!
//! Rows are matched by a key: the column(s) you pick, or an `id` / `*_id`
//! column whose values are unique in both results. Without a key, whole rows
//! are compared as a multiset (so there are only additions and removals).

use std::collections::HashMap;

use serde::Serialize;
use serde_json::Value;

use crate::QueryResult;

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Changed {
    pub before: Vec<Value>,
    pub after: Vec<Value>,
    /// indexes (in `columns`) of the cells that differ
    pub cells: Vec<usize>,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RowDiff {
    /// columns compared (present in both results)
    pub columns: Vec<String>,
    pub key: Vec<String>,
    pub columns_added: Vec<String>,
    pub columns_removed: Vec<String>,
    pub added: Vec<Vec<Value>>,
    pub removed: Vec<Vec<Value>>,
    pub changed: Vec<Changed>,
    pub unchanged: usize,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub added: usize,
    pub removed: usize,
    pub changed: usize,
}

use serde::Deserialize;

impl RowDiff {
    pub fn summary(&self) -> Summary {
        Summary {
            added: self.added.len(),
            removed: self.removed.len(),
            changed: self.changed.len(),
        }
    }
    pub fn is_empty(&self) -> bool {
        self.added.is_empty()
            && self.removed.is_empty()
            && self.changed.is_empty()
            && self.columns_added.is_empty()
            && self.columns_removed.is_empty()
    }
}

impl Summary {
    /// "3 added · 1 removed · 2 changed"
    pub fn text(&self) -> String {
        let mut parts = Vec::new();
        if self.added > 0 {
            parts.push(format!("{} added", self.added));
        }
        if self.removed > 0 {
            parts.push(format!("{} removed", self.removed));
        }
        if self.changed > 0 {
            parts.push(format!("{} changed", self.changed));
        }
        if parts.is_empty() {
            "no row changes".into()
        } else {
            parts.join(" · ")
        }
    }
}

fn unique(r: &QueryResult, idx: &[usize]) -> bool {
    let mut seen = std::collections::HashSet::new();
    r.rows.iter().all(|row| {
        let k: Vec<&Value> = idx.iter().map(|&i| &row[i]).collect();
        !k.iter().any(|v| v.is_null()) && seen.insert(serde_json::to_string(&k).unwrap_or_default())
    })
}

/// The key the diff would use when none is given.
pub fn auto_key(prev: &QueryResult, next: &QueryResult) -> Vec<String> {
    let names: Vec<&str> = next.columns.iter().map(|c| c.name.as_str()).collect();
    let mut candidates: Vec<&str> = names
        .iter()
        .copied()
        .filter(|n| n.eq_ignore_ascii_case("id"))
        .collect();
    candidates.extend(names.iter().copied().filter(|n| {
        let l = n.to_lowercase();
        l != "id" && (l.ends_with("_id") || l == "uuid" || l == "key" || l == "_id")
    }));
    for c in candidates {
        let (Some(pi), Some(ni)) = (
            prev.columns.iter().position(|x| x.name == c),
            next.columns.iter().position(|x| x.name == c),
        ) else {
            continue;
        };
        if unique(prev, &[pi]) && unique(next, &[ni]) {
            return vec![c.to_string()];
        }
    }
    vec![]
}

pub fn diff(prev: &QueryResult, next: &QueryResult, key: Option<&[String]>) -> RowDiff {
    let pn: Vec<&str> = prev.columns.iter().map(|c| c.name.as_str()).collect();
    let nn: Vec<&str> = next.columns.iter().map(|c| c.name.as_str()).collect();
    let common: Vec<String> = nn
        .iter()
        .filter(|n| pn.contains(n))
        .map(|s| s.to_string())
        .collect();
    let pi: Vec<usize> = common
        .iter()
        .map(|c| pn.iter().position(|x| x == c).unwrap())
        .collect();
    let ni: Vec<usize> = common
        .iter()
        .map(|c| nn.iter().position(|x| x == c).unwrap())
        .collect();
    let project = |row: &Vec<Value>, idx: &[usize]| -> Vec<Value> {
        idx.iter()
            .map(|&i| row.get(i).cloned().unwrap_or(Value::Null))
            .collect()
    };
    let mut out = RowDiff {
        columns: common.clone(),
        columns_added: nn
            .iter()
            .filter(|n| !pn.contains(n))
            .map(|s| s.to_string())
            .collect(),
        columns_removed: pn
            .iter()
            .filter(|n| !nn.contains(n))
            .map(|s| s.to_string())
            .collect(),
        ..Default::default()
    };
    let key: Vec<String> = match key {
        Some(k) if !k.is_empty() => k.iter().filter(|c| common.contains(c)).cloned().collect(),
        _ => auto_key(prev, next),
    };
    let prow: Vec<Vec<Value>> = prev.rows.iter().map(|r| project(r, &pi)).collect();
    let nrow: Vec<Vec<Value>> = next.rows.iter().map(|r| project(r, &ni)).collect();
    if key.is_empty() {
        // multiset of whole rows
        let mut counts: HashMap<String, (usize, Vec<Value>)> = HashMap::new();
        for r in &prow {
            counts
                .entry(serde_json::to_string(r).unwrap_or_default())
                .or_insert((0, r.clone()))
                .0 += 1;
        }
        for r in &nrow {
            let k = serde_json::to_string(r).unwrap_or_default();
            match counts.get_mut(&k) {
                Some((n, _)) if *n > 0 => {
                    *n -= 1;
                    out.unchanged += 1;
                }
                _ => out.added.push(r.clone()),
            }
        }
        for r in &prow {
            let k = serde_json::to_string(r).unwrap_or_default();
            if let Some((n, _)) = counts.get_mut(&k) {
                if *n > 0 {
                    *n -= 1;
                    out.removed.push(r.clone());
                }
            }
        }
        return out;
    }
    out.key = key.clone();
    let kidx: Vec<usize> = key
        .iter()
        .map(|k| common.iter().position(|c| c == k).unwrap())
        .collect();
    let kof = |r: &Vec<Value>| {
        serde_json::to_string(&kidx.iter().map(|&i| &r[i]).collect::<Vec<_>>()).unwrap_or_default()
    };
    let mut before: HashMap<String, &Vec<Value>> = prow.iter().map(|r| (kof(r), r)).collect();
    for r in &nrow {
        match before.remove(&kof(r)) {
            None => out.added.push(r.clone()),
            Some(b) => {
                let cells: Vec<usize> = (0..common.len()).filter(|&i| b[i] != r[i]).collect();
                if cells.is_empty() {
                    out.unchanged += 1;
                } else {
                    out.changed.push(Changed {
                        before: b.clone(),
                        after: r.clone(),
                        cells,
                    });
                }
            }
        }
    }
    // keep the previous order for removals
    for r in &prow {
        if before.contains_key(&kof(r)) {
            out.removed.push(r.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ResultColumn;
    use serde_json::json;

    fn res(cols: &[&str], rows: Vec<Value>) -> QueryResult {
        QueryResult {
            columns: cols
                .iter()
                .map(|c| ResultColumn {
                    name: c.to_string(),
                    data_type: String::new(),
                })
                .collect(),
            rows: rows
                .into_iter()
                .map(|r| r.as_array().unwrap().clone())
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn keyed_diff() {
        let a = res(
            &["id", "name", "total"],
            vec![
                json!([1, "a", 10]),
                json!([2, "b", 20]),
                json!([3, "c", 30]),
            ],
        );
        let b = res(
            &["id", "name", "total", "new_col"],
            vec![
                json!([1, "a", 10, 0]),
                json!([2, "b", 25, 0]),
                json!([4, "d", 40, 0]),
            ],
        );
        let d = diff(&a, &b, None);
        assert_eq!(d.key, ["id"]);
        assert_eq!(d.columns_added, ["new_col"]);
        assert_eq!(d.added, vec![vec![json!(4), json!("d"), json!(40)]]);
        assert_eq!(d.removed, vec![vec![json!(3), json!("c"), json!(30)]]);
        assert_eq!(d.changed.len(), 1);
        assert_eq!(d.changed[0].cells, vec![2]);
        assert_eq!(d.unchanged, 1);
        assert_eq!(d.summary().text(), "1 added · 1 removed · 1 changed");
    }

    #[test]
    fn keyless_multiset_diff() {
        let a = res(
            &["day", "n"],
            vec![json!(["mon", 1]), json!(["mon", 1]), json!(["tue", 2])],
        );
        let b = res(&["day", "n"], vec![json!(["mon", 1]), json!(["tue", 3])]);
        let d = diff(&a, &b, None);
        assert!(d.key.is_empty());
        assert_eq!(d.added, vec![vec![json!("tue"), json!(3)]]);
        assert_eq!(
            d.removed.len(),
            2,
            "one duplicate mon row and the old tue row"
        );
        assert_eq!(d.unchanged, 1);
        // an explicit key turns removal+addition into a change
        let d = diff(&a, &b, Some(&["day".to_string()]));
        assert!(d.is_empty() || !d.changed.is_empty());
    }
}
