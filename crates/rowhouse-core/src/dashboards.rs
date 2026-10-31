//! Dashboards: a grid of tiles, each a read-only query shown as a chart, a
//! table or a single number. Stored next to the connections
//! (`dashboards.json`); tiles can follow a saved query so editing the query
//! updates the dashboard.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{config::ConnectionStore, Error, Result};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Tile {
    pub id: String,
    pub title: String,
    pub sql: String,
    /// follow this saved query's SQL when it exists
    #[serde(default)]
    pub snippet_id: Option<i64>,
    /// "chart" | "table" | "number"
    pub view: String,
    /// the chart settings from the results chart (type, x, y)
    #[serde(default)]
    pub chart: Option<Value>,
    /// width in grid columns (of 12) and height in rows
    #[serde(default = "six")]
    pub w: u8,
    #[serde(default = "one")]
    pub h: u8,
}

fn six() -> u8 {
    6
}
fn one() -> u8 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Dashboard {
    pub id: String,
    pub name: String,
    pub connection_id: String,
    #[serde(default)]
    pub tiles: Vec<Tile>,
    /// auto refresh, seconds (0 = off)
    #[serde(default)]
    pub refresh_secs: u32,
    #[serde(default)]
    pub updated_at: i64,
}

#[derive(Default, Serialize, Deserialize)]
struct File {
    #[serde(default)]
    dashboards: Vec<Dashboard>,
}

pub struct DashboardStore {
    path: PathBuf,
}

impl Default for DashboardStore {
    fn default() -> Self {
        Self {
            path: ConnectionStore::default_dir().join("dashboards.json"),
        }
    }
}

impl DashboardStore {
    pub fn at(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn list(&self) -> Result<Vec<Dashboard>> {
        match std::fs::read_to_string(&self.path) {
            Ok(t) if t.trim().is_empty() => Ok(vec![]),
            Ok(t) => Ok(serde_json::from_str::<File>(&t)?.dashboards),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
            Err(e) => Err(e.into()),
        }
    }

    fn write(&self, dashboards: Vec<Dashboard>) -> Result<()> {
        if let Some(d) = self.path.parent() {
            std::fs::create_dir_all(d)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(&File { dashboards })?)?;
        std::fs::rename(tmp, &self.path)?;
        Ok(())
    }

    /// Insert or replace (by id).
    pub fn save(&self, mut d: Dashboard) -> Result<Dashboard> {
        if d.name.trim().is_empty() {
            return Err(Error::Other("give the dashboard a name".into()));
        }
        if d.tiles.len() > 60 {
            return Err(Error::Other("a dashboard holds up to 60 tiles".into()));
        }
        for t in &mut d.tiles {
            t.w = t.w.clamp(2, 12);
            t.h = t.h.clamp(1, 4);
            if !["chart", "table", "number"].contains(&t.view.as_str()) {
                t.view = "table".into();
            }
        }
        d.updated_at = crate::schedules::now();
        let mut all = self.list()?;
        match all.iter_mut().find(|x| x.id == d.id) {
            Some(x) => *x = d.clone(),
            None => all.push(d.clone()),
        }
        self.write(all)?;
        Ok(d)
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        let all = self.list()?.into_iter().filter(|d| d.id != id).collect();
        self.write(all)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_list_delete() {
        let dir = tempfile::tempdir().unwrap();
        let s = DashboardStore::at(dir.path().join("d.json"));
        assert!(s.list().unwrap().is_empty());
        let d = Dashboard {
            id: "a".into(),
            name: "Sales".into(),
            connection_id: "c".into(),
            tiles: vec![Tile {
                id: "t".into(),
                title: "Revenue".into(),
                sql: "SELECT 1".into(),
                snippet_id: None,
                view: "weird".into(),
                chart: None,
                w: 40,
                h: 0,
            }],
            refresh_secs: 300,
            updated_at: 0,
        };
        let saved = s.save(d).unwrap();
        assert_eq!(
            (
                saved.tiles[0].w,
                saved.tiles[0].h,
                saved.tiles[0].view.as_str()
            ),
            (12, 1, "table")
        );
        assert_eq!(s.list().unwrap().len(), 1);
        assert!(s
            .save(Dashboard {
                name: " ".into(),
                ..saved.clone()
            })
            .is_err());
        s.delete("a").unwrap();
        assert!(s.list().unwrap().is_empty());
    }
}
