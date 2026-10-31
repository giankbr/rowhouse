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
    /// current values of the filters the tiles use (`:from`, `{{region}}`, …)
    #[serde(default)]
    pub filters: std::collections::BTreeMap<String, String>,
    /// save a PNG of the dashboard on a schedule (while the app is open)
    #[serde(default)]
    pub snapshot: Option<Snapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub enabled: bool,
    pub every: crate::schedules::Every,
    /// folder the PNGs go to
    pub dir: String,
    #[serde(default)]
    pub notify: bool,
    #[serde(default)]
    pub last_run: Option<i64>,
    #[serde(default)]
    pub last_error: Option<String>,
}

impl Dashboard {
    /// Next snapshot time, if one is scheduled.
    pub fn next_snapshot(&self) -> Option<i64> {
        let s = self
            .snapshot
            .as_ref()
            .filter(|s| s.enabled && !s.dir.trim().is_empty())?;
        crate::schedules::next_after(&s.every, s.last_run.unwrap_or(self.updated_at))
    }

    /// Dashboards whose snapshot is due at `now`.
    pub fn due(list: &[Dashboard], now: i64) -> Vec<Dashboard> {
        list.iter()
            .filter(|d| d.next_snapshot().is_some_and(|t| t <= now))
            .cloned()
            .collect()
    }
}

impl DashboardStore {
    /// Record a snapshot run without touching `updated_at` (that's for edits).
    pub fn mark_snapshot(&self, id: &str, at: i64, error: Option<String>) -> Result<()> {
        let mut all = self.list()?;
        if let Some(d) = all.iter_mut().find(|d| d.id == id) {
            if let Some(s) = d.snapshot.as_mut() {
                s.last_run = Some(at);
                s.last_error = error;
            }
        }
        self.write(all)
    }
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
    pub fn default_path() -> PathBuf {
        ConnectionStore::default_dir().join("dashboards.json")
    }

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
        if let Some(s) = d.snapshot.as_mut().filter(|s| s.enabled) {
            if s.dir.trim().is_empty() {
                return Err(Error::Other("pick a folder for the snapshots".into()));
            }
            if let crate::schedules::Every::Minutes { n } = &mut s.every {
                *n = (*n).max(5);
            }
            if crate::schedules::next_after(&s.every, crate::schedules::now()).is_none() {
                return Err(Error::Other(
                    "that snapshot time isn't valid (use HH:MM)".into(),
                ));
            }
        }
        d.updated_at = crate::schedules::now();
        let mut all = self.list()?;
        match all.iter_mut().find(|x| x.id == d.id) {
            Some(x) => {
                // the editor may hold a stale copy; a snapshot that ran since wins
                if let (Some(new), Some(old)) = (d.snapshot.as_mut(), x.snapshot.as_ref()) {
                    if old.last_run > new.last_run {
                        new.last_run = old.last_run;
                        new.last_error = old.last_error.clone();
                    }
                }
                *x = d.clone()
            }
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
            filters: Default::default(),
            snapshot: Some(Snapshot {
                enabled: true,
                every: crate::schedules::Every::Minutes { n: 60 },
                dir: "/tmp".into(),
                notify: false,
                last_run: Some(1_000),
                last_error: None,
            }),
        };
        assert_eq!(d.next_snapshot(), Some(1_000 + 3600));
        assert_eq!(Dashboard::due(std::slice::from_ref(&d), 5_000).len(), 1);
        let saved = s.save(d).unwrap();
        s.mark_snapshot("a", 9_999, None).unwrap();
        assert_eq!(
            s.list().unwrap()[0].snapshot.as_ref().unwrap().last_run,
            Some(9_999)
        );
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
