//! Write approvals between `rowhouse-mcp` and the desktop app.
//!
//! The MCP server never writes on its own. For a write it drops a request
//! file into `<config dir>/approvals/`, the desktop app shows it to the user,
//! and the user's answer comes back as `<id>.answer.json`. Files are plain
//! JSON in the user's own config dir, so no network port or token is needed.
//!
//! The app touches `approvals/app.alive` every few seconds; when that file is
//! stale the MCP server fails fast with "open Rowhouse to approve".

use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use crate::{ConnectionStore, Error, Result};

const ALIVE: &str = "app.alive";
/// The app counts as running if it touched `app.alive` this recently.
const ALIVE_WINDOW: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRequest {
    pub id: String,
    pub connection_id: String,
    pub connection_name: String,
    pub sql: String,
    /// MCP client name, e.g. "claude-ai", "cursor"
    pub client: String,
    /// unix seconds
    pub created_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalAnswer {
    pub approved: bool,
    #[serde(default)]
    pub reason: Option<String>,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub struct Approvals {
    dir: PathBuf,
}

impl Approvals {
    pub fn open_default() -> Result<Self> {
        Self::at(ConnectionStore::default_dir().join("approvals"))
    }

    pub fn at(dir: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&dir)?;
        Ok(Self { dir })
    }

    fn req_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.request.json"))
    }

    fn ans_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.answer.json"))
    }

    fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(tmp, path)?;
        Ok(())
    }

    // ---- app side ------------------------------------------------------------

    /// Called periodically by the desktop app.
    pub fn heartbeat(&self) -> Result<()> {
        Self::write_atomic(&self.dir.join(ALIVE), now().to_string().as_bytes())
    }

    /// Requests that have no answer yet, oldest first.
    pub fn pending(&self) -> Result<Vec<ApprovalRequest>> {
        let mut out = Vec::new();
        for e in std::fs::read_dir(&self.dir)? {
            let p = e?.path();
            let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let Some(id) = name.strip_suffix(".request.json") else {
                continue;
            };
            if self.ans_path(id).exists() {
                continue;
            }
            if let Ok(r) = serde_json::from_slice::<ApprovalRequest>(&std::fs::read(&p)?) {
                out.push(r);
            }
        }
        out.sort_by_key(|r| r.created_at);
        Ok(out)
    }

    pub fn answer(&self, id: &str, answer: &ApprovalAnswer) -> Result<()> {
        if !self.req_path(id).exists() {
            return Err(Error::Other(
                "approval request not found (it may have timed out)".into(),
            ));
        }
        Self::write_atomic(&self.ans_path(id), &serde_json::to_vec(answer)?)
    }

    // ---- MCP side ------------------------------------------------------------

    pub fn app_alive(&self) -> bool {
        std::fs::read_to_string(self.dir.join(ALIVE))
            .ok()
            .and_then(|s| s.trim().parse::<u64>().ok())
            .is_some_and(|t| now().saturating_sub(t) <= ALIVE_WINDOW.as_secs())
    }

    pub fn submit(
        &self,
        connection_id: &str,
        connection_name: &str,
        sql: &str,
        client: &str,
    ) -> Result<ApprovalRequest> {
        let req = ApprovalRequest {
            id: uuid::Uuid::new_v4().to_string(),
            connection_id: connection_id.into(),
            connection_name: connection_name.into(),
            sql: sql.into(),
            client: client.into(),
            created_at: now(),
        };
        Self::write_atomic(&self.req_path(&req.id), &serde_json::to_vec_pretty(&req)?)?;
        Ok(req)
    }

    /// Wait for the user's answer. The request is removed afterwards either way.
    pub async fn wait(&self, id: &str, timeout: Duration) -> Result<ApprovalAnswer> {
        let deadline = tokio::time::Instant::now() + timeout;
        let result = loop {
            if let Ok(bytes) = std::fs::read(self.ans_path(id)) {
                break serde_json::from_slice::<ApprovalAnswer>(&bytes).map_err(Error::from);
            }
            if tokio::time::Instant::now() >= deadline {
                break Ok(ApprovalAnswer {
                    approved: false,
                    reason: Some("no answer in time".into()),
                });
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        };
        let _ = std::fs::remove_file(self.req_path(id));
        let _ = std::fs::remove_file(self.ans_path(id));
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn request_answer_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let a = Approvals::at(dir.path().into()).unwrap();
        assert!(!a.app_alive());
        a.heartbeat().unwrap();
        assert!(a.app_alive());

        let r = a
            .submit("c1", "Prod", "UPDATE t SET x = 1 WHERE id = 2", "claude-ai")
            .unwrap();
        assert_eq!(a.pending().unwrap(), vec![r.clone()]);

        let b = Approvals::at(dir.path().into()).unwrap();
        let id = r.id.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            b.answer(
                &id,
                &ApprovalAnswer {
                    approved: true,
                    reason: None,
                },
            )
            .unwrap();
        });
        let ans = a.wait(&r.id, Duration::from_secs(5)).await.unwrap();
        assert!(ans.approved);
        assert!(a.pending().unwrap().is_empty());
        assert!(!a.req_path(&r.id).exists(), "cleaned up");

        let r2 = a
            .submit("c1", "Prod", "DELETE FROM t WHERE id = 1", "x")
            .unwrap();
        let ans = a.wait(&r2.id, Duration::from_millis(300)).await.unwrap();
        assert!(!ans.approved && ans.reason.as_deref() == Some("no answer in time"));
    }
}
