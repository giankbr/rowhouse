//! Backup & restore through the databases' own tools (`pg_dump` / `pg_restore` /
//! `psql`, `mysqldump` / `mysql`), plus SQLite's `VACUUM INTO`. Runs against the
//! connection's real endpoint, so SSH-tunnelled profiles work too.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tokio::process::Command;

use crate::{config::Driver, Connection, Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum DumpFormat {
    /// plain SQL script
    #[default]
    Sql,
    /// pg_dump custom archive (`-Fc`), restore with pg_restore
    Custom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum DumpContent {
    #[default]
    All,
    SchemaOnly,
    DataOnly,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DumpOptions {
    #[serde(default)]
    pub format: DumpFormat,
    #[serde(default)]
    pub content: DumpContent,
    /// only these tables (`schema.table` or `table`); empty = everything
    #[serde(default)]
    pub tables: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DumpReport {
    pub path: String,
    pub bytes: u64,
    pub tool: String,
    pub elapsed_ms: u128,
    /// warnings the tool printed (stderr), if any
    pub log: String,
}

/// Where the client tools usually live when they're not on PATH (GUI apps on
/// macOS don't inherit the shell's PATH).
fn search_dirs() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    for d in [
        "/opt/homebrew/bin",
        "/usr/local/bin",
        "/usr/bin",
        "/opt/homebrew/opt/libpq/bin",
        "/usr/local/opt/libpq/bin",
        "/opt/homebrew/opt/mysql-client/bin",
        "/usr/local/opt/mysql-client/bin",
        "/Applications/Postgres.app/Contents/Versions/latest/bin",
        "/Applications/DBngin.app/Contents/Resources",
    ] {
        v.push(PathBuf::from(d));
    }
    for base in ["/opt/homebrew/opt", "/usr/local/opt"] {
        if let Ok(rd) = std::fs::read_dir(base) {
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().to_string();
                if n.starts_with("postgresql@")
                    || n.starts_with("mysql-client@")
                    || n.starts_with("mysql@")
                {
                    v.push(e.path().join("bin"));
                }
            }
        }
    }
    for base in ["/Library/PostgreSQL", "C:\\Program Files\\PostgreSQL"] {
        if let Ok(rd) = std::fs::read_dir(base) {
            let mut vers: Vec<PathBuf> = rd.flatten().map(|e| e.path().join("bin")).collect();
            vers.sort();
            v.extend(vers.into_iter().rev());
        }
    }
    if let Ok(rd) = std::fs::read_dir("C:\\Program Files\\MySQL") {
        v.extend(rd.flatten().map(|e| e.path().join("bin")));
    }
    v
}

/// Full path of a client tool, if it's installed.
pub fn find_tool(name: &str) -> Option<PathBuf> {
    let exe = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    search_dirs()
        .into_iter()
        .map(|d| d.join(&exe))
        .find(|p| p.is_file())
}

fn need(name: &str, hint: &str) -> Result<PathBuf> {
    find_tool(name).ok_or_else(|| Error::Other(format!("`{name}` not found — {hint}")))
}

const PG_HINT: &str = "install the PostgreSQL client tools (macOS: `brew install libpq`, Windows: the PostgreSQL installer)";
const MY_HINT: &str = "install the MySQL client tools (macOS: `brew install mysql-client`)";

async fn run(mut cmd: Command, tool: &str) -> Result<String> {
    cmd.kill_on_drop(true);
    let out = cmd
        .output()
        .await
        .map_err(|e| Error::Other(format!("{tool}: {e}")))?;
    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
    if !out.status.success() {
        return Err(Error::Other(format!(
            "{tool} failed ({}): {}",
            out.status,
            if stderr.is_empty() {
                "no output".into()
            } else {
                stderr
            }
        )));
    }
    Ok(stderr)
}

fn pg_base(cmd: &mut Command, c: &Connection, password: Option<&str>) {
    let (host, port) = c.endpoint();
    cmd.arg("-h").arg(host).arg("-p").arg(port.to_string());
    if let Some(u) = c.config.user.as_deref().filter(|u| !u.is_empty()) {
        cmd.arg("-U").arg(u);
    }
    if let Some(p) = password.filter(|p| !p.is_empty()) {
        cmd.env("PGPASSWORD", p);
    }
    if let Some(t) = &c.config.tls {
        use crate::config::TlsMode as M;
        cmd.env(
            "PGSSLMODE",
            match t.mode {
                M::Disable => "disable",
                M::Prefer => "prefer",
                M::Require => "require",
                M::VerifyCa => "verify-ca",
                M::VerifyFull => "verify-full",
            },
        );
        if let Some(ca) = t.ca_cert.as_deref().filter(|s| !s.is_empty()) {
            cmd.env("PGSSLROOTCERT", crate::tunnel::expand_home(ca));
        }
    }
    cmd.arg("-w"); // never prompt
}

fn my_base(cmd: &mut Command, c: &Connection, password: Option<&str>) {
    let (host, port) = c.endpoint();
    // 127.0.0.1 instead of localhost so the client uses TCP (the tunnel), not a socket
    let host = if host == "localhost" {
        "127.0.0.1"
    } else {
        host
    };
    cmd.arg("-h").arg(host).arg("-P").arg(port.to_string());
    if let Some(u) = c.config.user.as_deref().filter(|u| !u.is_empty()) {
        cmd.arg("-u").arg(u);
    }
    if let Some(p) = password.filter(|p| !p.is_empty()) {
        cmd.env("MYSQL_PWD", p);
    }
}

fn database(c: &Connection) -> Result<String> {
    c.config
        .database
        .clone()
        .filter(|d| !d.trim().is_empty())
        .ok_or_else(|| {
            Error::Other(
                "the profile has no database set — pick one in the connection settings".into(),
            )
        })
}

/// Write a backup of the connection's database to `path`.
pub async fn backup(
    c: &Connection,
    password: Option<&str>,
    opts: &DumpOptions,
    path: &Path,
) -> Result<DumpReport> {
    let started = std::time::Instant::now();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let (tool, log) = match c.driver() {
        Driver::Postgres => {
            let exe = need("pg_dump", PG_HINT)?;
            let mut cmd = Command::new(&exe);
            pg_base(&mut cmd, c, password);
            cmd.arg("-d").arg(
                c.config
                    .database
                    .clone()
                    .unwrap_or_else(|| "postgres".into()),
            );
            cmd.arg("--no-owner").arg("--no-privileges");
            match opts.format {
                DumpFormat::Custom => cmd.arg("-Fc"),
                DumpFormat::Sql => cmd.arg("-Fp"),
            };
            match opts.content {
                DumpContent::SchemaOnly => {
                    cmd.arg("--schema-only");
                }
                DumpContent::DataOnly => {
                    cmd.arg("--data-only");
                }
                DumpContent::All => {}
            }
            for t in &opts.tables {
                cmd.arg("-t").arg(t);
            }
            cmd.arg("-f").arg(path);
            ("pg_dump", run(cmd, "pg_dump").await?)
        }
        Driver::Mysql => {
            let exe = need("mysqldump", MY_HINT)?;
            let mut cmd = Command::new(&exe);
            my_base(&mut cmd, c, password);
            cmd.args([
                "--single-transaction",
                "--routines",
                "--triggers",
                "--column-statistics=0",
            ]);
            // MariaDB's mysqldump doesn't know --column-statistics; retried below without it
            match opts.content {
                DumpContent::SchemaOnly => {
                    cmd.arg("--no-data");
                }
                DumpContent::DataOnly => {
                    cmd.arg("--no-create-info");
                }
                DumpContent::All => {}
            }
            cmd.arg(format!("--result-file={}", path.display()));
            cmd.arg(database(c)?);
            for t in &opts.tables {
                cmd.arg(t.rsplit('.').next().unwrap_or(t));
            }
            let log = match run(cmd, "mysqldump").await {
                Err(e) if e.to_string().contains("column-statistics") => {
                    let mut cmd = Command::new(&exe);
                    my_base(&mut cmd, c, password);
                    cmd.args(["--single-transaction", "--routines", "--triggers"]);
                    match opts.content {
                        DumpContent::SchemaOnly => {
                            cmd.arg("--no-data");
                        }
                        DumpContent::DataOnly => {
                            cmd.arg("--no-create-info");
                        }
                        DumpContent::All => {}
                    }
                    cmd.arg(format!("--result-file={}", path.display()));
                    cmd.arg(database(c)?);
                    for t in &opts.tables {
                        cmd.arg(t.rsplit('.').next().unwrap_or(t));
                    }
                    run(cmd, "mysqldump").await?
                }
                other => other?,
            };
            ("mysqldump", log)
        }
        Driver::Sqlite if c.kind() == crate::DbKind::Sqlite => {
            if path.exists() {
                std::fs::remove_file(path)?;
            }
            let target = path.to_string_lossy().replace('\'', "''");
            c.query(&format!("VACUUM INTO '{target}'"), 1).await?;
            ("sqlite VACUUM INTO", String::new())
        }
        Driver::Duckdb => {
            std::fs::create_dir_all(path)?;
            let target = path.to_string_lossy().replace('\'', "''");
            c.query(&format!("EXPORT DATABASE '{target}'"), 1).await?;
            ("duckdb EXPORT DATABASE", String::new())
        }
        _ => {
            return Err(Error::Other(format!(
                "backup isn't available for {:?} yet — use Export on a table instead",
                c.kind()
            )))
        }
    };
    let bytes = dir_or_file_size(path);
    Ok(DumpReport {
        path: path.to_string_lossy().into_owned(),
        bytes,
        tool: tool.into(),
        elapsed_ms: started.elapsed().as_millis(),
        log,
    })
}

fn dir_or_file_size(p: &Path) -> u64 {
    if p.is_dir() {
        std::fs::read_dir(p)
            .map(|rd| rd.flatten().map(|e| dir_or_file_size(&e.path())).sum())
            .unwrap_or(0)
    } else {
        std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
    }
}

/// Load a backup into the connection's database. Plain `.sql` files go through
/// `psql` / `mysql` (stops at the first error); pg custom archives through `pg_restore`.
pub async fn restore(c: &Connection, password: Option<&str>, path: &Path) -> Result<DumpReport> {
    if c.config.read_only {
        return Err(Error::ReadOnly("connection is read-only".into()));
    }
    let started = std::time::Instant::now();
    let head = {
        use std::io::Read;
        let mut buf = [0u8; 5];
        let n = std::fs::File::open(path)?.read(&mut buf)?;
        buf[..n].to_vec()
    };
    let (tool, log) = match c.driver() {
        Driver::Postgres if head.starts_with(b"PGDMP") => {
            let exe = need("pg_restore", PG_HINT)?;
            let mut cmd = Command::new(&exe);
            pg_base(&mut cmd, c, password);
            cmd.arg("-d").arg(
                c.config
                    .database
                    .clone()
                    .unwrap_or_else(|| "postgres".into()),
            );
            cmd.args(["--no-owner", "--no-privileges", "--exit-on-error"]);
            cmd.arg(path);
            ("pg_restore", run(cmd, "pg_restore").await?)
        }
        Driver::Postgres => {
            let exe = need("psql", PG_HINT)?;
            let mut cmd = Command::new(&exe);
            pg_base(&mut cmd, c, password);
            cmd.arg("-d").arg(
                c.config
                    .database
                    .clone()
                    .unwrap_or_else(|| "postgres".into()),
            );
            cmd.args(["-v", "ON_ERROR_STOP=1", "-q", "-f"]).arg(path);
            ("psql", run(cmd, "psql").await?)
        }
        Driver::Mysql => {
            let exe = need("mysql", MY_HINT)?;
            let mut cmd = Command::new(&exe);
            my_base(&mut cmd, c, password);
            cmd.arg(database(c)?);
            cmd.arg("-e").arg(format!("source {}", path.display()));
            ("mysql", run(cmd, "mysql").await?)
        }
        Driver::Sqlite | Driver::Duckdb => {
            let sql = std::fs::read_to_string(path)?;
            c.query(&sql, 1).await?;
            ("script", String::new())
        }
        _ => {
            return Err(Error::Other(format!(
                "restore isn't available for {:?} yet",
                c.kind()
            )))
        }
    };
    Ok(DumpReport {
        path: path.to_string_lossy().into_owned(),
        bytes: dir_or_file_size(path),
        tool: tool.into(),
        elapsed_ms: started.elapsed().as_millis(),
        log,
    })
}

/// Which tools this machine has, for the dialog to show a hint up front.
pub fn tools_for(c: &Connection) -> Vec<(String, bool)> {
    let names: &[&str] = match c.driver() {
        Driver::Postgres => &["pg_dump", "pg_restore", "psql"],
        Driver::Mysql => &["mysqldump", "mysql"],
        _ => &[],
    };
    names
        .iter()
        .map(|n| (n.to_string(), find_tool(n).is_some()))
        .collect()
}
