//! Notebooks: SQL and Markdown cells on one connection, kept in
//! `notebooks.json` next to the connections and synced like dashboards.
//! Results aren't stored — cells re-run when the notebook opens — except for
//! the Markdown export, which the app builds from the cells it just ran.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{config::ConnectionStore, Error, Result};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NbCell {
    pub id: String,
    /// "sql" | "markdown"
    pub kind: String,
    #[serde(default)]
    pub source: String,
    /// sql cells: "table" | "chart"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chart: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Notebook {
    pub id: String,
    pub name: String,
    pub connection_id: String,
    #[serde(default)]
    pub cells: Vec<NbCell>,
    /// values for `:name` / `{{name}}` used in the cells
    #[serde(default)]
    pub params: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub updated_at: i64,
}

#[derive(Default, Serialize, Deserialize)]
struct File {
    #[serde(default)]
    notebooks: Vec<Notebook>,
}

pub struct NotebookStore {
    path: PathBuf,
}

impl Default for NotebookStore {
    fn default() -> Self {
        Self::at(Self::default_path())
    }
}

impl NotebookStore {
    pub fn default_path() -> PathBuf {
        ConnectionStore::default_dir().join("notebooks.json")
    }

    pub fn at(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn list(&self) -> Result<Vec<Notebook>> {
        match std::fs::read_to_string(&self.path) {
            Ok(t) if t.trim().is_empty() => Ok(vec![]),
            Ok(t) => Ok(serde_json::from_str::<File>(&t)?.notebooks),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
            Err(e) => Err(e.into()),
        }
    }

    fn write(&self, notebooks: Vec<Notebook>) -> Result<()> {
        if let Some(d) = self.path.parent() {
            std::fs::create_dir_all(d)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(&File { notebooks })?)?;
        std::fs::rename(tmp, &self.path)?;
        Ok(())
    }

    /// Insert or replace (by id).
    pub fn save(&self, mut n: Notebook) -> Result<Notebook> {
        if n.name.trim().is_empty() {
            return Err(Error::Other("give the notebook a name".into()));
        }
        if n.cells.len() > 300 {
            return Err(Error::Other("a notebook holds up to 300 cells".into()));
        }
        for c in &mut n.cells {
            if c.kind != "markdown" {
                c.kind = "sql".into();
            }
        }
        n.updated_at = crate::schedules::now();
        let mut all = self.list()?;
        match all.iter_mut().find(|x| x.id == n.id) {
            Some(x) => *x = n.clone(),
            None => all.push(n.clone()),
        }
        self.write(all)?;
        Ok(n)
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        let all = self.list()?.into_iter().filter(|n| n.id != id).collect();
        self.write(all)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_list_delete() {
        let dir = tempfile::tempdir().unwrap();
        let s = NotebookStore::at(dir.path().join("n.json"));
        assert!(s.list().unwrap().is_empty());
        let n = Notebook {
            id: "n1".into(),
            name: "Churn".into(),
            connection_id: "c".into(),
            cells: vec![
                NbCell {
                    id: "a".into(),
                    kind: "markdown".into(),
                    source: "# Churn".into(),
                    view: None,
                    chart: None,
                },
                NbCell {
                    id: "b".into(),
                    kind: "weird".into(),
                    source: "SELECT 1".into(),
                    view: None,
                    chart: None,
                },
            ],
            params: Default::default(),
            updated_at: 0,
        };
        let saved = s.save(n).unwrap();
        assert!(saved.updated_at > 0);
        assert_eq!(saved.cells[1].kind, "sql");
        assert_eq!(s.list().unwrap().len(), 1);
        assert!(s
            .save(Notebook {
                name: " ".into(),
                ..saved.clone()
            })
            .is_err());
        s.delete("n1").unwrap();
        assert!(s.list().unwrap().is_empty());
    }
}
