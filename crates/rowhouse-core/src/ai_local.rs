//! Local AI: use what's already on the machine instead of an API key.
//!
//! * coding-agent CLIs you're signed in to — Claude Code, Codex, OpenCode,
//!   Gemini CLI, Cursor Agent, GitHub Copilot CLI — are run headless with a
//!   one-shot prompt (they use their own login / subscription);
//! * local model servers — Ollama, LM Studio — speak the OpenAI API.
//!
//! GUI apps on macOS don't inherit the shell's PATH, so the usual install
//! locations (Homebrew, npm/pnpm/bun globals, nvm, ~/.local/bin, …) are
//! searched too, and added to PATH when the agent runs (npm-installed CLIs
//! need `node`).

use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use serde::Serialize;
use tokio::io::AsyncWriteExt;

use crate::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Agent {
    pub id: &'static str,
    pub label: &'static str,
    /// executable names to look for
    pub bins: &'static [&'static str],
}

pub const AGENTS: &[Agent] = &[
    Agent {
        id: "claude",
        label: "Claude Code",
        bins: &["claude"],
    },
    Agent {
        id: "codex",
        label: "Codex CLI",
        bins: &["codex"],
    },
    Agent {
        id: "opencode",
        label: "OpenCode",
        bins: &["opencode"],
    },
    Agent {
        id: "gemini",
        label: "Gemini CLI",
        bins: &["gemini"],
    },
    Agent {
        id: "cursor",
        label: "Cursor Agent",
        bins: &["cursor-agent"],
    },
    Agent {
        id: "copilot",
        label: "GitHub Copilot CLI",
        bins: &["copilot"],
    },
];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Detected {
    /// agent id (`claude`, `codex`, …) or `ollama` / `lmstudio`
    pub id: String,
    pub label: String,
    /// "cli" | "server"
    pub kind: String,
    /// executable path (cli) or base URL (server)
    pub location: String,
    pub version: Option<String>,
    /// models a local server has pulled
    pub models: Vec<String>,
}

fn home() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|d| d.home_dir().to_path_buf())
}

/// Directories to search besides $PATH.
pub fn extra_dirs() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = vec![];
    if let Some(h) = home() {
        for rel in [
            ".local/bin",
            ".claude/local",
            ".npm-global/bin",
            ".bun/bin",
            ".opencode/bin",
            ".volta/bin",
            ".cargo/bin",
            ".deno/bin",
            "Library/pnpm",
            ".local/share/pnpm",
            "bin",
            "AppData/Roaming/npm",
        ] {
            v.push(h.join(rel));
        }
        // nvm / fnm node versions, newest first
        for base in [
            ".nvm/versions/node",
            ".local/share/fnm/node-versions",
            "Library/Application Support/fnm/node-versions",
        ] {
            if let Ok(rd) = std::fs::read_dir(h.join(base)) {
                let mut vs: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
                vs.sort();
                for p in vs.into_iter().rev() {
                    v.push(p.join("bin"));
                    v.push(p.join("installation/bin"));
                }
            }
        }
    }
    for p in [
        "/opt/homebrew/bin",
        "/usr/local/bin",
        "/usr/bin",
        "/snap/bin",
    ] {
        v.push(PathBuf::from(p));
    }
    v
}

fn search_path() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    for d in extra_dirs() {
        if !dirs.contains(&d) {
            dirs.push(d);
        }
    }
    dirs
}

fn find_bin(name: &str) -> Option<PathBuf> {
    let exts: &[&str] = if cfg!(windows) {
        &[".exe", ".cmd", ".bat", ""]
    } else {
        &[""]
    };
    for d in search_path() {
        for e in exts {
            let p = d.join(format!("{name}{e}"));
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// PATH for running an agent: its own directory first, then everything we know.
fn run_path(bin: &Path) -> std::ffi::OsString {
    let mut dirs = vec![];
    if let Some(d) = bin.parent() {
        dirs.push(d.to_path_buf());
    }
    dirs.extend(search_path());
    std::env::join_paths(dirs).unwrap_or_default()
}

async fn version_of(bin: &Path) -> Option<String> {
    let out = tokio::time::timeout(
        Duration::from_secs(4),
        tokio::process::Command::new(bin)
            .arg("--version")
            .env("PATH", run_path(bin))
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    let s = String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    (!s.is_empty()).then_some(s.chars().take(60).collect())
}

async fn server_models(base: &str) -> Option<Vec<String>> {
    let c = reqwest::Client::builder()
        .timeout(Duration::from_millis(700))
        .build()
        .ok()?;
    let v: serde_json::Value = c
        .get(format!("{base}/models"))
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;
    Some(
        v["data"]
            .as_array()?
            .iter()
            .filter_map(|m| m["id"].as_str().map(str::to_string))
            .collect(),
    )
}

/// Everything usable on this machine, CLIs first.
pub async fn detect() -> Vec<Detected> {
    crate::init_crypto();
    let mut out = vec![];
    for a in AGENTS {
        if let Some(p) = a.bins.iter().find_map(|b| find_bin(b)) {
            out.push(Detected {
                id: a.id.into(),
                label: a.label.into(),
                kind: "cli".into(),
                version: version_of(&p).await,
                location: p.to_string_lossy().into(),
                models: vec![],
            });
        }
    }
    for (id, label, base) in [
        ("ollama", "Ollama", "http://localhost:11434/v1"),
        ("lmstudio", "LM Studio", "http://localhost:1234/v1"),
    ] {
        if let Some(models) = server_models(base).await {
            out.push(Detected {
                id: id.into(),
                label: label.into(),
                kind: "server".into(),
                location: base.into(),
                version: None,
                models,
            });
        }
    }
    out
}

fn agent(id: &str) -> Result<&'static Agent> {
    AGENTS
        .iter()
        .find(|a| a.id == id)
        .ok_or_else(|| Error::Other(format!("unknown local agent `{id}`")))
}

/// Arguments for a one-shot, no-tools answer. The prompt goes in on stdin
/// where the CLI supports it (keeps long schemas off the command line).
fn args_for(id: &str, model: &str, prompt: &str) -> (Vec<String>, bool) {
    let m = model.trim();
    let mut a: Vec<String> = vec![];
    let stdin = match id {
        "claude" => {
            a.extend(
                [
                    "-p",
                    "--output-format",
                    "text",
                    "--max-turns",
                    "1",
                    "--disallowedTools",
                    "Bash,Edit,Write,WebFetch,WebSearch",
                ]
                .map(String::from),
            );
            if !m.is_empty() {
                a.extend(["--model".into(), m.into()]);
            }
            true
        }
        "codex" => {
            a.extend(
                [
                    "exec",
                    "--skip-git-repo-check",
                    "--sandbox",
                    "read-only",
                    "--color",
                    "never",
                ]
                .map(String::from),
            );
            if !m.is_empty() {
                a.extend(["-m".into(), m.into()]);
            }
            a.push("-".into()); // read the prompt from stdin
            true
        }
        "opencode" => {
            a.push("run".into());
            if !m.is_empty() {
                a.extend(["-m".into(), m.into()]);
            }
            a.push(prompt.into());
            false
        }
        "gemini" => {
            if !m.is_empty() {
                a.extend(["-m".into(), m.into()]);
            }
            a.extend(["-p".into(), prompt.into()]);
            false
        }
        "cursor" => {
            a.extend(["-p", "--output-format", "text"].map(String::from));
            if !m.is_empty() {
                a.extend(["--model".into(), m.into()]);
            }
            a.push(prompt.into());
            false
        }
        _ => {
            // copilot
            if !m.is_empty() {
                a.extend(["--model".into(), m.into()]);
            }
            a.extend(["-p".into(), prompt.into()]);
            false
        }
    };
    (a, stdin)
}

/// Codex prints a transcript; keep only the final answer.
fn clean_output(id: &str, s: &str) -> String {
    let s = s.trim();
    if id == "codex" {
        if let Some(i) = s.rfind("\ncodex\n") {
            let rest = &s[i + 7..];
            let end = rest.find("\ntokens used").unwrap_or(rest.len());
            return rest[..end].trim().to_string();
        }
    }
    s.to_string()
}

/// Ask a local agent CLI. Runs in a scratch directory so it can't read or touch
/// your projects.
pub async fn run(id: &str, model: &str, system: &str, user: &str) -> Result<String> {
    let a = agent(id)?;
    let bin =
        a.bins.iter().find_map(|b| find_bin(b)).ok_or_else(|| {
            Error::Other(format!("{} isn't installed (or isn't on PATH)", a.label))
        })?;
    let prompt = format!(
        "{system}\n\nAnswer directly in this reply. Do not run tools, read files or ask questions.\n\n---\n\n{user}"
    );
    let (args, use_stdin) = args_for(id, model, &prompt);
    let dir = std::env::temp_dir().join("rowhouse-agent");
    std::fs::create_dir_all(&dir)?;
    let mut cmd = tokio::process::Command::new(&bin);
    cmd.args(&args)
        .current_dir(&dir)
        .env("PATH", run_path(&bin))
        .env("NO_COLOR", "1")
        .stdin(if use_stdin {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = cmd
        .spawn()
        .map_err(|e| Error::Other(format!("couldn't start {}: {e}", a.label)))?;
    if use_stdin {
        if let Some(mut si) = child.stdin.take() {
            si.write_all(prompt.as_bytes()).await?;
            drop(si);
        }
    }
    let out = tokio::time::timeout(Duration::from_secs(180), child.wait_with_output())
        .await
        .map_err(|_| Error::Other(format!("{} took longer than 3 minutes", a.label)))??;
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    if !out.status.success() || stdout.trim().is_empty() {
        let e = String::from_utf8_lossy(&out.stderr);
        let hint = if e.to_lowercase().contains("login") || e.to_lowercase().contains("auth") {
            format!(" — sign in once in a terminal (`{}`)", a.bins[0])
        } else {
            String::new()
        };
        return Err(Error::Other(format!(
            "{} failed{hint}: {}",
            a.label,
            e.lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("no output")
                .trim()
        )));
    }
    Ok(clean_output(id, &stdout))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_args_and_cleans_codex() {
        let (a, stdin) = args_for("claude", "sonnet", "hi");
        assert!(stdin && a.contains(&"-p".to_string()) && a.contains(&"sonnet".to_string()));
        let (a, stdin) = args_for("opencode", "", "hi");
        assert!(!stdin && a == ["run", "hi"]);
        let t =
            "[2026] OpenAI Codex\n--------\nuser\nwrite sql\n\ncodex\nSELECT 1;\ntokens used: 120";
        assert_eq!(clean_output("codex", t), "SELECT 1;");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn detects_and_runs_a_fake_agent() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("opencode");
        std::fs::write(&bin, "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo 'opencode 9.9'; exit 0; fi\necho \"SELECT 42; -- $2\" | head -c 40\n").unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let old = std::env::var_os("PATH").unwrap_or_default();
        std::env::set_var(
            "PATH",
            std::env::join_paths(
                std::iter::once(dir.path().to_path_buf()).chain(std::env::split_paths(&old)),
            )
            .unwrap(),
        );
        let found = detect().await;
        let oc = found.iter().find(|d| d.id == "opencode").expect("detected");
        assert_eq!(oc.version.as_deref(), Some("opencode 9.9"));
        let out = run("opencode", "", "You write SQL.", "count users")
            .await
            .unwrap();
        assert!(out.starts_with("SELECT 42;"), "{out}");
        std::env::set_var("PATH", old);
    }
}
