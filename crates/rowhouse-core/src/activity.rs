//! Server activity: who is connected and what is running, with cancel / kill.

use serde::Serialize;
use serde_json::Value;

use crate::{config::Driver, Connection, Error, Result};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    /// pid / connection id / spid / query id
    pub id: String,
    pub user: String,
    pub database: String,
    pub client: String,
    pub state: String,
    /// seconds the current statement has been running
    pub seconds: Option<f64>,
    pub query: String,
    /// this is Rowhouse's own session
    pub own: bool,
}

fn s(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(x)) => x.clone(),
        Some(Value::Null) | None => String::new(),
        Some(o) => o.to_string(),
    }
}

fn f(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Number(n) => n.as_f64(),
        Value::String(x) => x.parse().ok(),
        _ => None,
    }
}

impl Connection {
    pub async fn activity(&self) -> Result<Vec<Session>> {
        let sql = match self.driver() {
            Driver::Postgres => {
                "SELECT pid::text, coalesce(usename, ''), coalesce(datname, ''), \
                   coalesce(host(client_addr), 'local'), coalesce(state, backend_type), \
                   extract(epoch FROM (now() - query_start))::float8, coalesce(query, ''), \
                   (pid = pg_backend_pid())::text \
                 FROM pg_stat_activity WHERE backend_type = 'client backend' \
                 ORDER BY (state = 'active') DESC, query_start"
            }
            Driver::Mysql => {
                "SELECT CAST(id AS CHAR), user, coalesce(db, ''), host, coalesce(command, ''), \
                   time, coalesce(info, ''), CAST(id = CONNECTION_ID() AS CHAR) \
                 FROM information_schema.processlist ORDER BY command = 'Sleep', time DESC"
            }
            Driver::Mssql => {
                "SELECT CAST(s.session_id AS varchar(12)), s.login_name, coalesce(DB_NAME(r.database_id), ''), \
                   coalesce(s.host_name, ''), coalesce(r.status, s.status), \
                   CAST(DATEDIFF(ms, r.start_time, GETDATE()) / 1000.0 AS float), \
                   coalesce(t.text, ''), CASE WHEN s.session_id = @@SPID THEN 'true' ELSE 'false' END \
                 FROM sys.dm_exec_sessions s \
                 LEFT JOIN sys.dm_exec_requests r ON r.session_id = s.session_id \
                 OUTER APPLY sys.dm_exec_sql_text(r.sql_handle) t \
                 WHERE s.is_user_process = 1"
            }
            Driver::Clickhouse => {
                "SELECT query_id, user, current_database, toString(address), 'running', \
                   elapsed, query, 'false' FROM system.processes"
            }
            _ => {
                return Err(Error::Other(format!(
                    "server activity isn't available for {:?}",
                    self.kind()
                )))
            }
        };
        let out = self.query_unguarded(sql).await?;
        Ok(out
            .rows
            .into_iter()
            .map(|r| Session {
                id: s(r.first()),
                user: s(r.get(1)),
                database: s(r.get(2)),
                client: s(r.get(3)),
                state: s(r.get(4)),
                seconds: f(r.get(5)),
                query: s(r.get(6)),
                own: s(r.get(7)) == "true" || s(r.get(7)) == "1",
            })
            .collect())
    }

    /// Stop a session's current statement (`terminate = false`) or close the session.
    pub async fn kill_session(&self, id: &str, terminate: bool) -> Result<()> {
        if self.config.read_only {
            return Err(Error::ReadOnly("connection is read-only".into()));
        }
        let sql = match self.driver() {
            Driver::Postgres => {
                let pid: i64 = id.parse().map_err(|_| Error::Other("bad pid".into()))?;
                if terminate {
                    format!("SELECT pg_terminate_backend({pid})")
                } else {
                    format!("SELECT pg_cancel_backend({pid})")
                }
            }
            Driver::Mysql => {
                let n: u64 = id.parse().map_err(|_| Error::Other("bad id".into()))?;
                if terminate {
                    format!("KILL {n}")
                } else {
                    format!("KILL QUERY {n}")
                }
            }
            Driver::Mssql => {
                let n: u32 = id.parse().map_err(|_| Error::Other("bad spid".into()))?;
                format!("KILL {n}")
            }
            Driver::Clickhouse => format!("KILL QUERY WHERE query_id = {} ASYNC", self.literal(id)),
            _ => return Err(Error::Other("not supported".into())),
        };
        self.query_unguarded(&sql).await.map(|_| ())
    }

    /// Run admin SQL on a read-only profile too (SELECT on catalog views).
    async fn query_unguarded(&self, sql: &str) -> Result<crate::QueryResult> {
        let out = self.query_admin(sql).await?;
        Ok(out.results.into_iter().next().unwrap_or_default())
    }
}
