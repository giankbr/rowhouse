//! One-click setup of the Rowhouse MCP server in AI clients: Claude Desktop,
//! Claude Code, Cursor, Windsurf, VS Code, Codex, OpenCode, Gemini CLI.
//!
//! Each client keeps its MCP servers in a config file; we add (or update) a
//! `rowhouse` entry pointing at the bundled `rowhouse-mcp`, keep everything
//! else as it was, and leave a `.bak` copy of the previous file.

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{json, Map, Value};

use crate::{Error, Result};

const NAME: &str = "rowhouse";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    /// `{ "<key>": { "rowhouse": { "command": … } } }`
    Json(&'static str),
    /// VS Code: `{ "servers": { "rowhouse": { "type": "stdio", "command": … } } }`
    VsCode,
    /// OpenCode: `{ "mcp": { "rowhouse": { "type": "local", "command": [ … ] } } }`
    OpenCode,
    /// Codex: `[mcp_servers.rowhouse]` in config.toml
    CodexToml,
}

struct Client {
    id: &'static str,
    label: &'static str,
    format: Format,
    /// config file, relative to home (first match per OS)
    paths: &'static [&'static str],
}

const CLIENTS: &[Client] = &[
    Client {
        id: "claude-desktop",
        label: "Claude Desktop",
        format: Format::Json("mcpServers"),
        paths: &[
            "Library/Application Support/Claude/claude_desktop_config.json",
            "AppData/Roaming/Claude/claude_desktop_config.json",
            ".config/Claude/claude_desktop_config.json",
        ],
    },
    Client {
        id: "claude-code",
        label: "Claude Code",
        format: Format::Json("mcpServers"),
        paths: &[".claude.json"],
    },
    Client {
        id: "cursor",
        label: "Cursor",
        format: Format::Json("mcpServers"),
        paths: &[".cursor/mcp.json"],
    },
    Client {
        id: "windsurf",
        label: "Windsurf",
        format: Format::Json("mcpServers"),
        paths: &[".codeium/windsurf/mcp_config.json"],
    },
    Client {
        id: "vscode",
        label: "VS Code",
        format: Format::VsCode,
        paths: &[
            "Library/Application Support/Code/User/mcp.json",
            "AppData/Roaming/Code/User/mcp.json",
            ".config/Code/User/mcp.json",
        ],
    },
    Client {
        id: "codex",
        label: "Codex",
        format: Format::CodexToml,
        paths: &[".codex/config.toml"],
    },
    Client {
        id: "opencode",
        label: "OpenCode",
        format: Format::OpenCode,
        paths: &[".config/opencode/opencode.json"],
    },
    Client {
        id: "gemini",
        label: "Gemini CLI",
        format: Format::Json("mcpServers"),
        paths: &[".gemini/settings.json"],
    },
];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpClient {
    pub id: String,
    pub label: String,
    pub config_path: String,
    /// the client (its config folder) exists on this machine
    pub detected: bool,
    /// "missing" | "installed" | "outdated" (points at another binary) | "unreadable"
    pub status: String,
}

fn pick_path(home: &Path, c: &Client) -> PathBuf {
    // prefer a path whose folder exists (the client is installed), else the first for this OS
    let os_first: Vec<&&str> = c
        .paths
        .iter()
        .filter(|p| {
            if cfg!(target_os = "macos") {
                !p.starts_with("AppData")
                    && !p.starts_with(".config/Code")
                    && !p.starts_with(".config/Claude")
            } else if cfg!(windows) {
                !p.starts_with("Library")
                    && !p.starts_with(".config/Code")
                    && !p.starts_with(".config/Claude")
            } else {
                !p.starts_with("Library") && !p.starts_with("AppData")
            }
        })
        .collect();
    os_first
        .iter()
        .map(|p| home.join(p))
        .find(|p| p.parent().is_some_and(|d| d.is_dir()) || p.is_file())
        .unwrap_or_else(|| home.join(os_first.first().copied().unwrap_or(&c.paths[0])))
}

fn detected(home: &Path, c: &Client, path: &Path) -> bool {
    if path.is_file() {
        return true;
    }
    let dir_ok = path.parent().is_some_and(|d| d.is_dir() && d != home);
    // ~/.claude.json lives in home itself: Claude Code is there if ~/.claude exists
    dir_ok || (c.id == "claude-code" && home.join(".claude").is_dir())
}

fn read_json(path: &Path) -> Result<Value> {
    match std::fs::read_to_string(path) {
        Ok(t) if t.trim().is_empty() => Ok(json!({})),
        Ok(t) => serde_json::from_str(&t).map_err(|e| {
            Error::Other(format!(
                "{} isn't plain JSON ({e}) — add Rowhouse by hand",
                path.display()
            ))
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
        Err(e) => Err(e.into()),
    }
}

fn entry_command(c: &Client, v: &Value) -> Option<String> {
    match c.format {
        Format::Json(key) => v
            .get(key)?
            .get(NAME)?
            .get("command")?
            .as_str()
            .map(String::from),
        Format::VsCode => v
            .get("servers")?
            .get(NAME)?
            .get("command")?
            .as_str()
            .map(String::from),
        Format::OpenCode => v
            .get("mcp")?
            .get(NAME)?
            .get("command")?
            .get(0)?
            .as_str()
            .map(String::from),
        Format::CodexToml => None,
    }
}

fn toml_block(cmd: &str) -> String {
    format!(
        "[mcp_servers.{NAME}]\ncommand = {}\n",
        serde_json::to_string(cmd).unwrap_or_default()
    )
}

/// `[mcp_servers.rowhouse]` … up to the next table header.
fn toml_find(text: &str) -> Option<(usize, usize)> {
    let start = text.find(&format!("[mcp_servers.{NAME}]"))?;
    let after = start + 1;
    let end = text[after..]
        .find("\n[")
        .map(|i| after + i + 1)
        .unwrap_or(text.len());
    Some((start, end))
}

fn status_of(c: &Client, path: &Path, cmd: &str) -> String {
    if c.format == Format::CodexToml {
        return match std::fs::read_to_string(path) {
            Ok(t) => match toml_find(&t) {
                Some((a, b))
                    if t[a..b].contains(&serde_json::to_string(cmd).unwrap_or_default()) =>
                {
                    "installed"
                }
                Some(_) => "outdated",
                None => "missing",
            },
            Err(_) => "missing",
        }
        .into();
    }
    match read_json(path) {
        Ok(v) => match entry_command(c, &v) {
            Some(x) if x == cmd => "installed",
            Some(_) => "outdated",
            None => "missing",
        }
        .into(),
        Err(_) => "unreadable".into(),
    }
}

pub fn list_in(home: &Path, cmd: &str) -> Vec<McpClient> {
    CLIENTS
        .iter()
        .map(|c| {
            let path = pick_path(home, c);
            McpClient {
                id: c.id.into(),
                label: c.label.into(),
                detected: detected(home, c, &path),
                status: status_of(c, &path, cmd),
                config_path: path.to_string_lossy().into(),
            }
        })
        .collect()
}

fn client(id: &str) -> Result<&'static Client> {
    CLIENTS
        .iter()
        .find(|c| c.id == id)
        .ok_or_else(|| Error::Other(format!("unknown client `{id}`")))
}

fn backup(path: &Path) {
    if path.is_file() {
        let _ = std::fs::copy(
            path,
            path.with_extension(format!(
                "{}.bak",
                path.extension().and_then(|e| e.to_str()).unwrap_or("cfg")
            )),
        );
    }
}

fn write(path: &Path, text: &str) -> Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    backup(path);
    let tmp = path.with_extension("rowhouse-tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Add or update the Rowhouse entry. Returns the config file written.
pub fn install_in(home: &Path, id: &str, cmd: &str) -> Result<String> {
    let c = client(id)?;
    let path = pick_path(home, c);
    if c.format == Format::CodexToml {
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let block = toml_block(cmd);
        let next = match toml_find(&text) {
            Some((a, b)) => format!("{}{}{}", &text[..a], block, &text[b..]),
            None => format!(
                "{}{}{}",
                text,
                if text.is_empty() || text.ends_with("\n\n") {
                    ""
                } else if text.ends_with('\n') {
                    "\n"
                } else {
                    "\n\n"
                },
                block
            ),
        };
        write(&path, &next)?;
        return Ok(path.to_string_lossy().into());
    }
    let mut v = read_json(&path)?;
    if !v.is_object() {
        return Err(Error::Other(format!(
            "{} has an unexpected shape — add Rowhouse by hand",
            path.display()
        )));
    }
    let (key, entry) = match c.format {
        Format::Json(k) => (k, json!({ "command": cmd, "args": [] })),
        Format::VsCode => (
            "servers",
            json!({ "type": "stdio", "command": cmd, "args": [] }),
        ),
        Format::OpenCode => (
            "mcp",
            json!({ "type": "local", "command": [cmd], "enabled": true }),
        ),
        Format::CodexToml => unreachable!(),
    };
    let obj = v.as_object_mut().unwrap();
    let servers = obj.entry(key).or_insert_with(|| Value::Object(Map::new()));
    if !servers.is_object() {
        *servers = Value::Object(Map::new());
    }
    servers.as_object_mut().unwrap().insert(NAME.into(), entry);
    write(&path, &(serde_json::to_string_pretty(&v)? + "\n"))?;
    Ok(path.to_string_lossy().into())
}

pub fn remove_in(home: &Path, id: &str) -> Result<()> {
    let c = client(id)?;
    let path = pick_path(home, c);
    if !path.is_file() {
        return Ok(());
    }
    if c.format == Format::CodexToml {
        let text = std::fs::read_to_string(&path)?;
        if let Some((a, b)) = toml_find(&text) {
            write(&path, &format!("{}{}", &text[..a], &text[b..]))?;
        }
        return Ok(());
    }
    let mut v = read_json(&path)?;
    let key = match c.format {
        Format::Json(k) => k,
        Format::VsCode => "servers",
        Format::OpenCode => "mcp",
        Format::CodexToml => unreachable!(),
    };
    if let Some(s) = v.get_mut(key).and_then(Value::as_object_mut) {
        if s.remove(NAME).is_some() {
            write(&path, &(serde_json::to_string_pretty(&v)? + "\n"))?;
        }
    }
    Ok(())
}

fn home() -> Result<PathBuf> {
    directories::BaseDirs::new()
        .map(|d| d.home_dir().to_path_buf())
        .ok_or_else(|| Error::Other("no home directory".into()))
}
pub fn list(cmd: &str) -> Result<Vec<McpClient>> {
    Ok(list_in(&home()?, cmd))
}
pub fn install(id: &str, cmd: &str) -> Result<String> {
    install_in(&home()?, id, cmd)
}
pub fn remove(id: &str) -> Result<()> {
    remove_in(&home()?, id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installs_updates_and_removes() {
        let h = tempfile::tempdir().unwrap();
        let home = h.path();
        std::fs::create_dir_all(home.join(".cursor")).unwrap();
        std::fs::write(
            home.join(".cursor/mcp.json"),
            r#"{"mcpServers":{"other":{"command":"x"}},"theme":"dark"}"#,
        )
        .unwrap();
        std::fs::create_dir_all(home.join(".codex")).unwrap();
        std::fs::write(
            home.join(".codex/config.toml"),
            "model = \"gpt-5\"\n\n[mcp_servers.other]\ncommand = \"y\"\n",
        )
        .unwrap();

        let l = list_in(home, "/app/rowhouse-mcp");
        let cur = l.iter().find(|c| c.id == "cursor").unwrap();
        assert!(cur.detected && cur.status == "missing");
        assert!(!l.iter().find(|c| c.id == "windsurf").unwrap().detected);

        install_in(home, "cursor", "/app/rowhouse-mcp").unwrap();
        let v: Value =
            serde_json::from_str(&std::fs::read_to_string(home.join(".cursor/mcp.json")).unwrap())
                .unwrap();
        assert_eq!(v["mcpServers"]["rowhouse"]["command"], "/app/rowhouse-mcp");
        assert_eq!(v["mcpServers"]["other"]["command"], "x");
        assert_eq!(v["theme"], "dark");
        assert!(home.join(".cursor/mcp.json.bak").is_file());
        assert_eq!(
            list_in(home, "/new/rowhouse-mcp")
                .iter()
                .find(|c| c.id == "cursor")
                .unwrap()
                .status,
            "outdated"
        );

        install_in(home, "codex", "/app/rowhouse-mcp").unwrap();
        install_in(home, "codex", "/app2/rowhouse-mcp").unwrap();
        let t = std::fs::read_to_string(home.join(".codex/config.toml")).unwrap();
        assert!(t.contains("[mcp_servers.other]") && t.contains("model = \"gpt-5\""));
        assert_eq!(t.matches("[mcp_servers.rowhouse]").count(), 1);
        assert!(t.contains("\"/app2/rowhouse-mcp\""));
        assert_eq!(
            list_in(home, "/app2/rowhouse-mcp")
                .iter()
                .find(|c| c.id == "codex")
                .unwrap()
                .status,
            "installed"
        );

        install_in(home, "opencode", "/app/rowhouse-mcp").unwrap();
        let v: Value = serde_json::from_str(
            &std::fs::read_to_string(home.join(".config/opencode/opencode.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(v["mcp"]["rowhouse"]["command"][0], "/app/rowhouse-mcp");

        remove_in(home, "cursor").unwrap();
        remove_in(home, "codex").unwrap();
        assert!(!std::fs::read_to_string(home.join(".codex/config.toml"))
            .unwrap()
            .contains("rowhouse"));
        assert_eq!(
            list_in(home, "/app/rowhouse-mcp")
                .iter()
                .find(|c| c.id == "cursor")
                .unwrap()
                .status,
            "missing"
        );

        std::fs::write(home.join(".cursor/mcp.json"), "{ // comment\n}").unwrap();
        assert!(
            install_in(home, "cursor", "/x").is_err(),
            "never clobber a file we can't parse"
        );
    }
}
