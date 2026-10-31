//! Scheduled queries: saved SQL that runs on an interval or at a time of day
//! while the app is open. Each run is recorded (last [`KEEP_RUNS`]), can export
//! its result to a folder, and notices when the result changed.
//!
//! Stored in `schedules.json` next to `connections.json`.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Datelike, Local, NaiveTime, TimeZone};
use serde::{Deserialize, Serialize};

use crate::{config::ConnectionStore, guard, io::ExportFormat, Connection, Error, Result};

pub const KEEP_RUNS: usize = 20;
/// Rows kept in memory per run when the result isn't exported.
const ROW_LIMIT: usize = 10_000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Every {
    /// every N minutes (≥ 1)
    Minutes { n: u32 },
    /// every day at `at` (local "HH:MM")
    Daily { at: String },
    /// on `weekday` (0 = Monday) at `at` (local "HH:MM")
    Weekly { weekday: u8, at: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Notify {
    Never,
    #[default]
    OnFailure,
    /// on failure, or when the result differs from the previous run
    OnChange,
    Always,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportTarget {
    pub dir: String,
    pub format: ExportFormat,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    /// unix seconds
    pub at: i64,
    pub ok: bool,
    pub rows: u64,
    pub elapsed_ms: u64,
    #[serde(default)]
    pub error: Option<String>,
    /// result differs from the previous successful run
    #[serde(default)]
    pub changed: bool,
    /// exported file, if any
    #[serde(default)]
    pub file: Option<String>,
    #[serde(default)]
    pub hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Schedule {
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub connection_id: String,
    pub sql: String,
    pub every: Every,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub export: Option<ExportTarget>,
    #[serde(default)]
    pub notify: Notify,
    /// unix seconds; also reset whenever the timing changes or the schedule is re-enabled,
    /// so the next run counts from then
    #[serde(default)]
    pub created_at: i64,
    /// newest first
    #[serde(default)]
    pub runs: Vec<Run>,
}

fn yes() -> bool {
    true
}

/// The first run of `every` strictly after `since` (unix seconds).
pub fn next_after(every: &Every, since: i64) -> Option<i64> {
    match every {
        Every::Minutes { n } => Some(since + i64::from((*n).max(1)) * 60),
        Every::Daily { at } => {
            let t = parse_hm(at).ok()?;
            let base = local(since);
            let mut cand = base.date_naive().and_time(t);
            if Local.from_local_datetime(&cand).earliest()? <= base {
                cand += chrono::Duration::days(1);
            }
            Local
                .from_local_datetime(&cand)
                .earliest()
                .map(|d| d.timestamp())
        }
        Every::Weekly { weekday, at } => {
            let t = parse_hm(at).ok()?;
            let base = local(since);
            let today = base.weekday().num_days_from_monday() as i64;
            let mut days = (i64::from(*weekday) - today).rem_euclid(7);
            let mut cand = (base.date_naive() + chrono::Duration::days(days)).and_time(t);
            if Local.from_local_datetime(&cand).earliest()? <= base {
                days += 7;
                cand = (base.date_naive() + chrono::Duration::days(days)).and_time(t);
            }
            Local
                .from_local_datetime(&cand)
                .earliest()
                .map(|d| d.timestamp())
        }
    }
}

fn parse_hm(s: &str) -> Result<NaiveTime> {
    NaiveTime::parse_from_str(s.trim(), "%H:%M")
        .map_err(|_| Error::Other(format!("time must be HH:MM (got `{s}`)")))
}

fn local(ts: i64) -> DateTime<Local> {
    Local
        .timestamp_opt(ts, 0)
        .single()
        .unwrap_or_else(Local::now)
}

impl Schedule {
    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() {
            return Err(Error::Other("give the schedule a name".into()));
        }
        if self.sql.trim().is_empty() {
            return Err(Error::Other("the schedule has no SQL".into()));
        }
        match &self.every {
            Every::Minutes { n } if *n == 0 => {
                return Err(Error::Other("interval must be at least 1 minute".into()))
            }
            Every::Daily { at } => {
                parse_hm(at)?;
            }
            Every::Weekly { weekday, at } => {
                if *weekday > 6 {
                    return Err(Error::Other("weekday must be 0 (Mon) … 6 (Sun)".into()));
                }
                parse_hm(at)?;
            }
            _ => {}
        }
        if let Some(d) = guard::destructive(&self.sql).first() {
            return Err(Error::Other(format!(
                "scheduled queries can't run destructive statements ({} {})",
                d.kind, d.object
            )));
        }
        Ok(())
    }

    pub fn last_run_at(&self) -> Option<i64> {
        self.runs.first().map(|r| r.at)
    }

    /// When the schedule should run next (unix seconds), after its last run
    /// (or creation). `None` when disabled or invalid.
    pub fn next_run(&self) -> Option<i64> {
        if !self.enabled {
            return None;
        }
        let since = self.last_run_at().unwrap_or(0).max(self.created_at);
        next_after(&self.every, since)
    }

    pub fn is_due(&self, now: i64) -> bool {
        self.next_run().is_some_and(|t| t <= now)
    }

    /// Should this run produce a notification?
    pub fn should_notify(&self, run: &Run) -> bool {
        match self.notify {
            Notify::Never => false,
            Notify::OnFailure => !run.ok,
            Notify::OnChange => !run.ok || run.changed,
            Notify::Always => true,
        }
    }
}

/// `schedules.json`
pub struct ScheduleStore {
    path: PathBuf,
}

impl ScheduleStore {
    pub fn open_default() -> Self {
        Self::at(ConnectionStore::default_dir().join("schedules.json"))
    }

    pub fn at(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn list(&self) -> Result<Vec<Schedule>> {
        match std::fs::read_to_string(&self.path) {
            Ok(s) if s.trim().is_empty() => Ok(vec![]),
            Ok(s) => Ok(serde_json::from_str(&s)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
            Err(e) => Err(e.into()),
        }
    }

    fn write(&self, all: &[Schedule]) -> Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(all)?)?;
        std::fs::rename(tmp, &self.path)?;
        Ok(())
    }

    /// Insert or update (keeps the run log of an existing schedule).
    pub fn save(&self, mut s: Schedule) -> Result<Schedule> {
        s.validate()?;
        let mut all = self.list()?;
        if s.id.is_empty() {
            s.id = uuid::Uuid::new_v4().to_string();
        }
        match all.iter_mut().find(|x| x.id == s.id) {
            Some(cur) => {
                s.runs = std::mem::take(&mut cur.runs);
                s.created_at = cur.created_at;
                // a changed timing starts counting from now
                if cur.every != s.every || (!cur.enabled && s.enabled) {
                    s.created_at = now();
                }
                *cur = s.clone();
            }
            None => {
                if s.created_at == 0 {
                    s.created_at = now();
                }
                all.push(s.clone());
            }
        }
        self.write(&all)?;
        Ok(s)
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        let mut all = self.list()?;
        all.retain(|s| s.id != id);
        self.write(&all)
    }

    /// Append a run (newest first, capped).
    pub fn record(&self, id: &str, run: Run) -> Result<Option<Schedule>> {
        let mut all = self.list()?;
        let Some(s) = all.iter_mut().find(|s| s.id == id) else {
            return Ok(None);
        };
        s.runs.insert(0, run);
        s.runs.truncate(KEEP_RUNS);
        let out = s.clone();
        self.write(&all)?;
        Ok(Some(out))
    }
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// FNV-1a, stable across builds (unlike `DefaultHasher`).
fn fnv(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

fn file_name(s: &Schedule, at: i64, format: ExportFormat) -> String {
    let safe: String = s
        .name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let ext = match format {
        ExportFormat::Csv => "csv",
        ExportFormat::Json => "json",
        ExportFormat::Sql => "sql",
    };
    format!(
        "{}-{}.{ext}",
        safe.trim_matches('-'),
        local(at).format("%Y%m%d-%H%M%S")
    )
}

/// Run one schedule on an open connection. Never fails: errors are recorded in the run.
pub async fn run_on(conn: &Connection, s: &Schedule) -> Run {
    let at = now();
    let started = std::time::Instant::now();
    let prev_hash = s.runs.iter().find(|r| r.ok).and_then(|r| r.hash.clone());
    let result: Result<(u64, String, Option<String>)> = async {
        s.validate()?;
        match &s.export {
            Some(t) => {
                let dir = crate::tunnel::expand_home(&t.dir);
                std::fs::create_dir_all(&dir)?;
                let path = Path::new(&dir).join(file_name(s, at, t.format));
                let n = conn.export(&s.sql, t.format, &path, None).await?;
                // the exported file is the result; hash it as written
                let hash = fnv(&std::fs::read(&path)?);
                Ok((n, hash, Some(path.to_string_lossy().into_owned())))
            }
            None => {
                let out = conn.query(&s.sql, ROW_LIMIT).await?;
                let rows: u64 = out
                    .results
                    .iter()
                    .map(|r| (r.rows.len() as u64).max(r.rows_affected))
                    .sum();
                let body = serde_json::to_vec(
                    &out.results
                        .iter()
                        .map(|r| (&r.columns, &r.rows))
                        .collect::<Vec<_>>(),
                )?;
                Ok((rows, fnv(&body), None))
            }
        }
    }
    .await;
    let elapsed_ms = started.elapsed().as_millis() as u64;
    match result {
        Ok((rows, hash, file)) => Run {
            at,
            ok: true,
            rows,
            elapsed_ms,
            error: None,
            changed: prev_hash.is_some_and(|p| p != hash),
            file,
            hash: Some(hash),
        },
        Err(e) => Run {
            at,
            ok: false,
            rows: 0,
            elapsed_ms,
            error: Some(e.to_string()),
            changed: false,
            file: None,
            hash: None,
        },
    }
}

/// Connect with the saved profile and run.
pub async fn run_with_store(store: &ConnectionStore, s: &Schedule) -> Run {
    let conn = async {
        let cfg = store.get(&s.connection_id)?;
        Connection::connect(&cfg).await
    }
    .await;
    match conn {
        Ok(c) => {
            let r = run_on(&c, s).await;
            c.close().await;
            r
        }
        Err(e) => Run {
            at: now(),
            ok: false,
            rows: 0,
            elapsed_ms: 0,
            error: Some(e.to_string()),
            changed: false,
            file: None,
            hash: None,
        },
    }
}

/// Weekday/hour helpers for the UI preview text.
pub fn describe(e: &Every) -> String {
    const DAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    match e {
        Every::Minutes { n: 1 } => "every minute".into(),
        Every::Minutes { n } if n % 60 == 0 => format!("every {} h", n / 60),
        Every::Minutes { n } => format!("every {n} min"),
        Every::Daily { at } => format!("daily at {at}"),
        Every::Weekly { weekday, at } => {
            format!("{} at {at}", DAYS.get(*weekday as usize).unwrap_or(&"?"))
        }
    }
}
