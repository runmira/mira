//! Claude Code's plugin file formats, read leniently.
//!
//! - `.claude-plugin/marketplace.json`: a catalog of plugins and where to
//!   get each one.
//! - `.claude-plugin/plugin.json`: a plugin's name, metadata, and
//!   optional custom component paths.
//!
//! Unknown fields are ignored and most fields are optional, so catalogs
//! written for newer Claude Code versions still load.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Where plugin manifests live inside a repo. Mira reads Claude Code's
/// directory first, then its own.
pub const MANIFEST_DIRS: [&str; 2] = [".claude-plugin", ".mira-plugin"];

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Marketplace {
    pub name: String,
    pub description: Option<String>,
    pub version: Option<String>,
    pub owner: Option<Person>,
    pub metadata: MarketplaceMetadata,
    pub plugins: Vec<PluginEntry>,
    /// Old plugin name → new name.
    #[serde(deserialize_with = "renames")]
    pub renames: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MarketplaceMetadata {
    pub description: Option<String>,
    pub version: Option<String>,
    /// Base directory prepended to relative plugin sources.
    pub plugin_root: Option<String>,
}

/// `renames` is a map in current catalogs; accept a list of pairs too.
fn renames<'de, D: serde::Deserializer<'de>>(d: D) -> Result<BTreeMap<String, String>, D::Error> {
    let v = Value::deserialize(d)?;
    Ok(match v {
        Value::Object(m) => m
            .into_iter()
            .filter_map(|(k, v)| v.as_str().map(|s| (k, s.to_owned())))
            .collect(),
        Value::Array(items) => items
            .into_iter()
            .filter_map(|i| {
                let from = i.get("from").or_else(|| i.get(0))?.as_str()?.to_owned();
                let to = i.get("to").or_else(|| i.get(1))?.as_str()?.to_owned();
                Some((from, to))
            })
            .collect(),
        _ => BTreeMap::new(),
    })
}

/// `author`/`owner`: an object, or just a name.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(from = "PersonRepr")]
pub struct Person {
    pub name: String,
    pub email: Option<String>,
    pub url: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum PersonRepr {
    Name(String),
    Full {
        #[serde(default)]
        name: String,
        #[serde(default)]
        email: Option<String>,
        #[serde(default)]
        url: Option<String>,
    },
}

impl From<PersonRepr> for Person {
    fn from(r: PersonRepr) -> Self {
        match r {
            PersonRepr::Name(name) => Person {
                name,
                ..Default::default()
            },
            PersonRepr::Full { name, email, url } => Person { name, email, url },
        }
    }
}

/// One plugin in a marketplace. Besides where to get it, an entry may
/// carry any `plugin.json` field; with `strict: false` the entry alone
/// describes the plugin.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PluginEntry {
    pub name: String,
    pub display_name: Option<String>,
    pub source: PluginSource,
    pub description: Option<String>,
    pub version: Option<String>,
    pub author: Option<Person>,
    pub homepage: Option<String>,
    pub repository: Option<String>,
    pub license: Option<String>,
    pub keywords: Vec<String>,
    pub category: Option<String>,
    pub tags: Vec<String>,
    /// Default true: the plugin's own `plugin.json` is authoritative.
    pub strict: Option<bool>,
    pub commands: Option<Value>,
    pub agents: Option<Value>,
    pub skills: Option<Value>,
    pub hooks: Option<Value>,
    pub mcp_servers: Option<Value>,
    pub lsp_servers: Option<Value>,
}

/// Where a plugin's files come from.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(from = "SourceRepr", into = "SourceRepr")]
pub enum PluginSource {
    /// A directory inside the marketplace (`"./plugins/x"`).
    Relative(String),
    /// `owner/repo` on GitHub.
    Github {
        repo: String,
        git_ref: Option<String>,
        sha: Option<String>,
        path: Option<String>,
    },
    /// Any git URL, optionally a subdirectory of it (`git-subdir`).
    Git {
        url: String,
        git_ref: Option<String>,
        sha: Option<String>,
        path: Option<String>,
    },
    #[default]
    Unknown,
}

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum SourceRepr {
    Path(String),
    Object(BTreeMap<String, Value>),
}

impl From<SourceRepr> for PluginSource {
    fn from(r: SourceRepr) -> Self {
        let m = match r {
            SourceRepr::Path(p) => return PluginSource::Relative(p),
            SourceRepr::Object(m) => m,
        };
        let s = |k: &str| m.get(k).and_then(Value::as_str).map(str::to_owned);
        let git_ref = s("ref").or_else(|| s("branch")).or_else(|| s("tag"));
        match s("source").as_deref() {
            Some("github") => match s("repo") {
                Some(repo) => PluginSource::Github {
                    repo,
                    git_ref,
                    sha: s("sha"),
                    path: s("path"),
                },
                None => PluginSource::Unknown,
            },
            Some("url" | "git" | "git-subdir") => match s("url") {
                Some(url) => PluginSource::Git {
                    url,
                    git_ref,
                    sha: s("sha"),
                    path: s("path"),
                },
                None => PluginSource::Unknown,
            },
            Some("relative" | "directory" | "local") => match s("path") {
                Some(p) => PluginSource::Relative(p),
                None => PluginSource::Unknown,
            },
            _ => PluginSource::Unknown,
        }
    }
}

impl From<PluginSource> for SourceRepr {
    fn from(s: PluginSource) -> Self {
        let mut m = BTreeMap::new();
        let mut put = |k: &str, v: Option<String>| {
            if let Some(v) = v {
                m.insert(k.to_owned(), Value::String(v));
            }
        };
        match s {
            PluginSource::Relative(p) => return SourceRepr::Path(p),
            PluginSource::Github {
                repo,
                git_ref,
                sha,
                path,
            } => {
                put("source", Some("github".into()));
                put("repo", Some(repo));
                put("ref", git_ref);
                put("sha", sha);
                put("path", path);
            }
            PluginSource::Git {
                url,
                git_ref,
                sha,
                path,
            } => {
                let kind = if path.is_some() { "git-subdir" } else { "url" };
                put("source", Some(kind.into()));
                put("url", Some(url));
                put("ref", git_ref);
                put("sha", sha);
                put("path", path);
            }
            PluginSource::Unknown => put("source", Some("unknown".into())),
        }
        SourceRepr::Object(m)
    }
}

impl PluginSource {
    /// Short description for lists.
    pub fn label(&self) -> String {
        match self {
            PluginSource::Relative(p) => p.clone(),
            PluginSource::Github { repo, path, .. } => match path {
                Some(p) => format!("github:{repo}/{p}"),
                None => format!("github:{repo}"),
            },
            PluginSource::Git { url, path, .. } => match path {
                Some(p) => format!("{url} ({p})"),
                None => url.clone(),
            },
            PluginSource::Unknown => "unsupported source".into(),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PluginManifest {
    pub name: String,
    pub display_name: Option<String>,
    pub version: Option<String>,
    pub description: Option<String>,
    pub author: Option<Person>,
    pub homepage: Option<String>,
    pub repository: Option<Value>,
    pub license: Option<String>,
    pub keywords: Vec<String>,
    pub commands: Option<Value>,
    pub agents: Option<Value>,
    pub skills: Option<Value>,
    pub hooks: Option<Value>,
    pub mcp_servers: Option<Value>,
    pub lsp_servers: Option<Value>,
}

impl PluginManifest {
    /// Fill gaps from the marketplace entry. With `strict: false` the
    /// entry's component fields win.
    pub fn merge_entry(mut self, e: &PluginEntry) -> Self {
        let strict = e.strict.unwrap_or(true);
        if self.name.is_empty() {
            self.name = e.name.clone();
        }
        self.display_name = self.display_name.or_else(|| e.display_name.clone());
        self.version = self.version.or_else(|| e.version.clone());
        self.description = self.description.or_else(|| e.description.clone());
        self.author = self.author.or_else(|| e.author.clone());
        self.homepage = self.homepage.or_else(|| e.homepage.clone());
        self.license = self.license.or_else(|| e.license.clone());
        if self.keywords.is_empty() {
            self.keywords = e.keywords.clone();
        }
        let pick = |own: Option<Value>, entry: &Option<Value>| {
            if strict {
                own.or_else(|| entry.clone())
            } else {
                entry.clone().or(own)
            }
        };
        self.commands = pick(self.commands, &e.commands);
        self.agents = pick(self.agents, &e.agents);
        self.skills = pick(self.skills, &e.skills);
        self.hooks = pick(self.hooks, &e.hooks);
        self.mcp_servers = pick(self.mcp_servers, &e.mcp_servers);
        self.lsp_servers = pick(self.lsp_servers, &e.lsp_servers);
        self
    }
}

/// Read `<dir>/.claude-plugin/<file>` (or `.mira-plugin`).
pub fn read_manifest_file(
    dir: &std::path::Path,
    file: &str,
) -> Option<(std::path::PathBuf, String)> {
    MANIFEST_DIRS.iter().find_map(|d| {
        let p = dir.join(d).join(file);
        std::fs::read_to_string(&p).ok().map(|t| (p, t))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_every_source_shape() {
        let text = r#"{
          "name": "m", "owner": "Someone",
          "renames": {"old": "new"},
          "plugins": [
            {"name": "a", "source": "./plugins/a", "author": {"name": "A", "email": "a@x"}},
            {"name": "b", "source": {"source": "github", "repo": "o/r", "ref": "v1"}},
            {"name": "c", "source": {"source": "url", "url": "https://x/c.git", "sha": "abc"}},
            {"name": "d", "source": {"source": "git-subdir", "url": "https://x/d.git", "path": "p/d"}},
            {"name": "e", "source": {"source": "npm", "package": "e"}, "future": true}
          ]
        }"#;
        let m: Marketplace = serde_json::from_str(text).unwrap();
        assert_eq!(m.owner.as_ref().unwrap().name, "Someone");
        assert_eq!(m.renames["old"], "new");
        assert_eq!(
            m.plugins[0].source,
            PluginSource::Relative("./plugins/a".into())
        );
        assert!(
            matches!(&m.plugins[1].source, PluginSource::Github { repo, git_ref: Some(r), .. } if repo == "o/r" && r == "v1")
        );
        assert!(
            matches!(&m.plugins[2].source, PluginSource::Git { sha: Some(s), path: None, .. } if s == "abc")
        );
        assert!(
            matches!(&m.plugins[3].source, PluginSource::Git { path: Some(p), .. } if p == "p/d")
        );
        assert_eq!(m.plugins[4].source, PluginSource::Unknown);
        // Round-trips.
        let back: Marketplace = serde_json::from_str(&serde_json::to_string(&m).unwrap()).unwrap();
        assert_eq!(back.plugins[3].source, m.plugins[3].source);
    }

    #[test]
    fn non_strict_entries_define_components() {
        let entry = PluginEntry {
            name: "x".into(),
            strict: Some(false),
            commands: Some(serde_json::json!(["./cmds"])),
            ..Default::default()
        };
        let own = PluginManifest {
            commands: Some(serde_json::json!("./own")),
            ..Default::default()
        };
        let merged = own.clone().merge_entry(&entry);
        assert_eq!(merged.name, "x");
        assert_eq!(merged.commands, Some(serde_json::json!(["./cmds"])));
        let strict_entry = PluginEntry {
            strict: None,
            ..entry
        };
        assert_eq!(
            own.merge_entry(&strict_entry).commands,
            Some(serde_json::json!("./own"))
        );
    }
}
