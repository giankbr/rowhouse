//! Extensions: small web apps that run in a sandboxed panel inside Rowhouse
//! and talk to it through a message API (`window.rowhouse`). Each one is a
//! folder (or a `.rhext` zip of it) with a `manifest.json`:
//!
//! ```json
//! { "id": "table-stats", "name": "Table stats", "version": "1.0.0",
//!   "entry": "index.html",
//!   "contributes": { "panels": [{ "id": "main", "title": "Table stats" }] },
//!   "permissions": ["schema", "query:read"] }
//! ```
//!
//! Permissions are shown when an extension is enabled and enforced by the
//! host: `schema`, `query:read` (read-only statements), `query:write`,
//! `result` (receive the current result grid), `network:<host>`.

use std::{
    collections::HashMap,
    io::Read,
    path::{Component, Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{config::ConnectionStore, Error, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    #[serde(default = "default_entry")]
    pub entry: String,
    #[serde(default)]
    pub contributes: Contributes,
    #[serde(default)]
    pub permissions: Vec<String>,
    /// lowest Rowhouse version it works with
    #[serde(default)]
    pub engines: Option<String>,
}

fn default_entry() -> String {
    "index.html".into()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Contributes {
    #[serde(default)]
    pub panels: Vec<Contribution>,
    #[serde(default)]
    pub commands: Vec<Contribution>,
    /// actions offered on a query result ("Send to …")
    #[serde(default)]
    pub result_actions: Vec<Contribution>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contribution {
    pub id: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Installed {
    pub manifest: Manifest,
    pub dir: String,
    pub enabled: bool,
    /// permissions the user accepted when enabling
    pub granted: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct StateFile {
    #[serde(default)]
    enabled: HashMap<String, Vec<String>>,
}

pub const KNOWN_PERMISSIONS: &[&str] =
    &["schema", "query:read", "query:write", "result", "clipboard"];

fn valid_permission(p: &str) -> bool {
    KNOWN_PERMISSIONS.contains(&p)
        || p.strip_prefix("network:").is_some_and(|h| {
            !h.is_empty()
                && h.chars().all(|c| {
                    c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '*' || c == ':'
                })
        })
}

pub fn validate(m: &Manifest) -> Result<()> {
    if m.id.is_empty()
        || m.id.len() > 64
        || !m
            .id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(Error::Other(
            "manifest: `id` must be lowercase letters, digits and dashes".into(),
        ));
    }
    if m.name.trim().is_empty() {
        return Err(Error::Other("manifest: `name` is required".into()));
    }
    if let Some(p) = m.permissions.iter().find(|p| !valid_permission(p)) {
        return Err(Error::Other(format!("manifest: unknown permission `{p}`")));
    }
    if safe_join(Path::new("/x"), &m.entry).is_none() {
        return Err(Error::Other(
            "manifest: `entry` must be a file inside the extension".into(),
        ));
    }
    if m.contributes.panels.is_empty()
        && m.contributes.commands.is_empty()
        && m.contributes.result_actions.is_empty()
    {
        return Err(Error::Other(
            "manifest: contributes nothing (add a panel, command or result action)".into(),
        ));
    }
    Ok(())
}

/// `base/rel` if `rel` stays inside `base`.
fn safe_join(base: &Path, rel: &str) -> Option<PathBuf> {
    let p = Path::new(rel);
    if p.components()
        .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
    {
        return None;
    }
    Some(base.join(p))
}

pub struct Registry {
    pub root: PathBuf,
}

impl Default for Registry {
    fn default() -> Self {
        Registry {
            root: ConnectionStore::default_dir().join("extensions"),
        }
    }
}

impl Registry {
    fn state_path(&self) -> PathBuf {
        self.root.join("state.json")
    }
    fn state(&self) -> StateFile {
        std::fs::read_to_string(self.state_path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }
    fn save_state(&self, s: &StateFile) -> Result<()> {
        std::fs::create_dir_all(&self.root)?;
        std::fs::write(self.state_path(), serde_json::to_vec_pretty(s)?)?;
        Ok(())
    }

    fn read_manifest(dir: &Path) -> Result<Manifest> {
        let text = std::fs::read_to_string(dir.join("manifest.json"))
            .map_err(|_| Error::Other(format!("no manifest.json in {}", dir.display())))?;
        let m: Manifest =
            serde_json::from_str(&text).map_err(|e| Error::Other(format!("manifest.json: {e}")))?;
        validate(&m)?;
        if !dir.join(&m.entry).is_file() {
            return Err(Error::Other(format!("entry file `{}` is missing", m.entry)));
        }
        Ok(m)
    }

    pub fn list(&self) -> Result<Vec<Installed>> {
        let st = self.state();
        let mut out = vec![];
        let Ok(rd) = std::fs::read_dir(&self.root) else {
            return Ok(out);
        };
        for e in rd.flatten() {
            if !e.path().is_dir() {
                continue;
            }
            if let Ok(manifest) = Self::read_manifest(&e.path()) {
                let granted = st.enabled.get(&manifest.id).cloned();
                out.push(Installed {
                    dir: e.path().to_string_lossy().into(),
                    enabled: granted.is_some(),
                    granted: granted.unwrap_or_default(),
                    manifest,
                });
            }
        }
        out.sort_by(|a, b| {
            a.manifest
                .name
                .to_lowercase()
                .cmp(&b.manifest.name.to_lowercase())
        });
        Ok(out)
    }

    pub fn get(&self, id: &str) -> Result<Installed> {
        self.list()?
            .into_iter()
            .find(|x| x.manifest.id == id)
            .ok_or_else(|| Error::Other(format!("extension `{id}` is not installed")))
    }

    /// Install (or update) from a folder or a `.rhext` / `.zip` file. Starts disabled.
    pub fn install(&self, from: &Path) -> Result<Installed> {
        std::fs::create_dir_all(&self.root)?;
        let staging = self.root.join(format!(".incoming-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&staging);
        std::fs::create_dir_all(&staging)?;
        let res = (|| {
            if from.is_dir() {
                copy_dir(from, &staging)?;
            } else {
                unzip(from, &staging)?;
            }
            // a zip may wrap everything in one top folder
            let src = if staging.join("manifest.json").is_file() {
                staging.clone()
            } else {
                std::fs::read_dir(&staging)?
                    .flatten()
                    .map(|e| e.path())
                    .find(|p| p.join("manifest.json").is_file())
                    .ok_or_else(|| Error::Other("no manifest.json found".into()))?
            };
            let m = Self::read_manifest(&src)?;
            let dest = self.root.join(&m.id);
            let _ = std::fs::remove_dir_all(&dest);
            std::fs::rename(&src, &dest).or_else(|_| copy_dir(&src, &dest))?;
            // updated permissions need a fresh approval
            let mut st = self.state();
            if let Some(g) = st.enabled.get(&m.id) {
                if m.permissions.iter().any(|p| !g.contains(p)) {
                    st.enabled.remove(&m.id);
                    self.save_state(&st)?;
                }
            }
            self.get(&m.id)
        })();
        let _ = std::fs::remove_dir_all(&staging);
        res
    }

    pub fn uninstall(&self, id: &str) -> Result<()> {
        let x = self.get(id)?;
        std::fs::remove_dir_all(&x.dir)?;
        let mut st = self.state();
        st.enabled.remove(id);
        self.save_state(&st)
    }

    /// Enabling grants the manifest's permissions as they are now.
    pub fn set_enabled(&self, id: &str, enabled: bool) -> Result<Installed> {
        let x = self.get(id)?;
        let mut st = self.state();
        if enabled {
            st.enabled
                .insert(id.to_string(), x.manifest.permissions.clone());
        } else {
            st.enabled.remove(id);
        }
        self.save_state(&st)?;
        self.get(id)
    }

    /// The entry page with local `<script src>` / `<link href>` files inlined
    /// (the panel runs from an opaque origin and can't load them itself).
    pub fn entry_html(&self, id: &str) -> Result<String> {
        let x = self.get(id)?;
        if !x.enabled {
            return Err(Error::Other(format!("{} is disabled", x.manifest.name)));
        }
        let dir = PathBuf::from(&x.dir);
        let html = std::fs::read_to_string(dir.join(&x.manifest.entry))?;
        Ok(inline_assets(&html, &dir))
    }
}

fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let i = tag.find(&format!("{name}="))? + name.len() + 1;
    let rest = &tag[i..];
    let q = rest.chars().next()?;
    if q == '"' || q == '\'' {
        rest[1..].split(q).next()
    } else {
        rest.split(|c: char| c.is_whitespace() || c == '>').next()
    }
}

fn local(src: &str) -> bool {
    !src.contains("://") && !src.starts_with("//") && !src.starts_with("data:")
}

fn inline_assets(html: &str, dir: &Path) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(i) = rest.find('<') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let end = rest.find('>').map(|e| e + 1).unwrap_or(rest.len());
        let tag = &rest[..end];
        let lower = tag.to_ascii_lowercase();
        if lower.starts_with("<script") && lower.contains("src=") {
            if let Some(src) = attr(tag, "src").filter(|s| local(s)) {
                if let Some(code) =
                    safe_join(dir, src).and_then(|p| std::fs::read_to_string(p).ok())
                {
                    let close = rest[end..]
                        .to_ascii_lowercase()
                        .find("</script>")
                        .map(|c| end + c + 9)
                        .unwrap_or(end);
                    let module = if lower.contains("type=\"module\"") {
                        " type=\"module\""
                    } else {
                        ""
                    };
                    out.push_str(&format!(
                        "<script{module}>\n{}\n</script>",
                        code.replace("</script", "<\\/script")
                    ));
                    rest = &rest[close..];
                    continue;
                }
            }
        }
        if lower.starts_with("<link") && lower.contains("stylesheet") {
            if let Some(href) = attr(tag, "href").filter(|s| local(s)) {
                if let Some(css) =
                    safe_join(dir, href).and_then(|p| std::fs::read_to_string(p).ok())
                {
                    out.push_str(&format!("<style>\n{css}\n</style>"));
                    rest = &rest[end..];
                    continue;
                }
            }
        }
        out.push_str(tag);
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)?.flatten() {
        let name = e.file_name();
        if name.to_string_lossy().starts_with('.') || name == "node_modules" {
            continue;
        }
        let p = e.path();
        if p.is_dir() {
            copy_dir(&p, &to.join(&name))?;
        } else {
            std::fs::copy(&p, to.join(&name))?;
        }
    }
    Ok(())
}

fn unzip(file: &Path, to: &Path) -> Result<()> {
    let f = std::fs::File::open(file)?;
    let mut z = zip::ZipArchive::new(f)
        .map_err(|e| Error::Other(format!("not a valid extension package: {e}")))?;
    let mut total = 0u64;
    for i in 0..z.len() {
        let mut entry = z.by_index(i).map_err(|e| Error::Other(e.to_string()))?;
        let Some(rel) = entry.enclosed_name() else {
            continue;
        };
        let dest = to.join(rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&dest)?;
            continue;
        }
        if let Some(d) = dest.parent() {
            std::fs::create_dir_all(d)?;
        }
        let mut buf = vec![];
        entry
            .by_ref()
            .take(50 * 1024 * 1024)
            .read_to_end(&mut buf)?;
        total += buf.len() as u64;
        if total > 100 * 1024 * 1024 {
            return Err(Error::Other(
                "extension package is too large (100 MB max)".into(),
            ));
        }
        std::fs::write(dest, buf)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_ext(dir: &Path, id: &str, perms: &[&str]) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(
            dir.join("manifest.json"),
            serde_json::json!({
                "id": id, "name": "Demo", "version": "1.0.0",
                "contributes": { "panels": [{ "id": "main", "title": "Demo" }] },
                "permissions": perms,
            })
            .to_string(),
        )
        .unwrap();
        std::fs::write(dir.join("index.html"), "<html><head><link rel=\"stylesheet\" href=\"app.css\"></head><body><script src=\"app.js\"></script><script src=\"https://cdn.example/x.js\"></script></body></html>").unwrap();
        std::fs::write(dir.join("app.js"), "console.log('hi')").unwrap();
        std::fs::write(dir.join("app.css"), "body{color:red}").unwrap();
    }

    #[test]
    fn install_enable_inline() {
        let tmp = tempfile::tempdir().unwrap();
        let reg = Registry {
            root: tmp.path().join("ext"),
        };
        let src = tmp.path().join("src");
        write_ext(&src, "demo", &["schema", "query:read"]);
        let x = reg.install(&src).unwrap();
        assert!(!x.enabled);
        assert!(
            reg.entry_html("demo").is_err(),
            "disabled extensions don't load"
        );
        let x = reg.set_enabled("demo", true).unwrap();
        assert_eq!(x.granted, ["schema", "query:read"]);
        let html = reg.entry_html("demo").unwrap();
        assert!(
            html.contains("console.log('hi')")
                && html.contains("body{color:red}")
                && html.contains("https://cdn.example/x.js")
        );

        // an update asking for more permissions is disabled until approved again
        write_ext(&src, "demo", &["schema", "query:write"]);
        assert!(!reg.install(&src).unwrap().enabled);

        // bad manifests are refused
        write_ext(&tmp.path().join("bad"), "Bad Id", &[]);
        assert!(reg.install(&tmp.path().join("bad")).is_err());
        write_ext(&tmp.path().join("bad2"), "bad2", &["root"]);
        assert!(reg.install(&tmp.path().join("bad2")).is_err());

        reg.uninstall("demo").unwrap();
        assert!(reg.list().unwrap().is_empty());
        assert!(safe_join(Path::new("/a"), "../etc/passwd").is_none());
    }

    #[test]
    fn installs_zip() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("pkg");
        write_ext(&src, "zipped", &[]);
        let zpath = tmp.path().join("zipped.rhext");
        let mut w = zip::ZipWriter::new(std::fs::File::create(&zpath).unwrap());
        for name in ["manifest.json", "index.html", "app.js", "app.css"] {
            w.start_file(
                format!("zipped/{name}"),
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
            std::io::Write::write_all(&mut w, &std::fs::read(src.join(name)).unwrap()).unwrap();
        }
        w.finish().unwrap();
        let reg = Registry {
            root: tmp.path().join("ext"),
        };
        assert_eq!(reg.install(&zpath).unwrap().manifest.id, "zipped");
    }
}
