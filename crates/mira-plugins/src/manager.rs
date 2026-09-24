//! Marketplaces and installed plugins, kept under `~/.mira/plugins/`:
//!
//! ```text
//! known_marketplaces.json      marketplaces you added
//! installed_plugins.json       installed plugins and whether they're on
//! marketplaces/<name>/         each marketplace's files (a git clone,
//!                              or a downloaded marketplace.json)
//! cache/<marketplace>/<plugin>/<version>/   installed plugin files
//! ```
//!
//! The layout and file formats follow Claude Code, so the same
//! marketplaces and plugins work in both.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::components::{self, Components};
use crate::git;
use crate::manifest::{read_manifest_file, Marketplace, PluginEntry, PluginManifest, PluginSource};

/// Serializes changes to the state files within this process.
static LOCK: Mutex<()> = Mutex::const_new(());

/// Where a marketplace comes from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum MarketplaceSource {
    Github {
        repo: String,
        #[serde(default, rename = "ref", skip_serializing_if = "Option::is_none")]
        git_ref: Option<String>,
    },
    Git {
        url: String,
        #[serde(default, rename = "ref", skip_serializing_if = "Option::is_none")]
        git_ref: Option<String>,
    },
    /// A local directory, read in place.
    Directory { path: PathBuf },
    /// A `marketplace.json` served over HTTP(S).
    Url { url: String },
}

impl MarketplaceSource {
    /// Understand what someone typed: `owner/repo`, a GitHub or git URL,
    /// a URL to a `marketplace.json`, or a local path. `#ref` or `@ref`
    /// after a repo picks a branch or tag.
    pub fn parse(input: &str) -> Result<Self> {
        let input = input.trim();
        if input.is_empty() {
            bail!("enter a GitHub repo (owner/repo), a git URL, a URL to marketplace.json, or a local path");
        }
        let (base, git_ref) = match input.rsplit_once('#') {
            Some((b, r)) if !r.is_empty() => (b, Some(r.to_owned())),
            _ => (input, None),
        };
        if base.starts_with("http://") || base.starts_with("https://") {
            if base.ends_with(".json") {
                return Ok(MarketplaceSource::Url { url: base.into() });
            }
            if let Some(rest) = base
                .strip_prefix("https://github.com/")
                .or_else(|| base.strip_prefix("http://github.com/"))
            {
                let repo = rest.trim_end_matches('/').trim_end_matches(".git");
                if repo.split('/').count() == 2 {
                    return Ok(MarketplaceSource::Github {
                        repo: repo.into(),
                        git_ref,
                    });
                }
            }
            return Ok(MarketplaceSource::Git {
                url: base.into(),
                git_ref,
            });
        }
        if base.starts_with("git@")
            || base.starts_with("ssh://")
            || base.starts_with("file://")
            || base.ends_with(".git")
        {
            return Ok(MarketplaceSource::Git {
                url: base.into(),
                git_ref,
            });
        }
        let expanded = expand_home(input);
        if expanded.exists() {
            return Ok(MarketplaceSource::Directory {
                path: expanded.canonicalize().unwrap_or(expanded),
            });
        }
        let (repo, at_ref) = match base.split_once('@') {
            Some((r, v)) => (r, Some(v.to_owned())),
            None => (base, None),
        };
        let parts: Vec<&str> = repo.split('/').collect();
        if parts.len() == 2 && parts.iter().all(|p| valid_name(p)) {
            return Ok(MarketplaceSource::Github {
                repo: repo.into(),
                git_ref: git_ref.or(at_ref),
            });
        }
        bail!("`{input}` isn't a GitHub repo (owner/repo), git URL, marketplace.json URL, or existing path")
    }

    pub fn label(&self) -> String {
        match self {
            MarketplaceSource::Github { repo, git_ref } => match git_ref {
                Some(r) => format!("{repo}#{r}"),
                None => repo.clone(),
            },
            MarketplaceSource::Git { url, .. } => url.clone(),
            MarketplaceSource::Directory { path } => path.display().to_string(),
            MarketplaceSource::Url { url } => url.clone(),
        }
    }

    fn git_url(&self) -> Option<(String, Option<String>)> {
        match self {
            MarketplaceSource::Github { repo, git_ref } => {
                Some((format!("https://github.com/{repo}.git"), git_ref.clone()))
            }
            MarketplaceSource::Git { url, git_ref } => Some((url.clone(), git_ref.clone())),
            _ => None,
        }
    }
}

fn expand_home(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(p)
}

/// Names used as directory names: no separators, no dot-dot.
fn valid_name(s: &str) -> bool {
    !s.is_empty()
        && s != "."
        && s != ".."
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KnownMarketplace {
    pub source: MarketplaceSource,
    /// Where its files are.
    pub path: PathBuf,
    pub added_at: u64,
    pub updated_at: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InstalledPlugin {
    pub name: String,
    pub marketplace: String,
    pub version: String,
    pub path: PathBuf,
    pub enabled: bool,
    pub installed_at: u64,
    pub updated_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha: Option<String>,
}

impl InstalledPlugin {
    pub fn id(&self) -> String {
        format!("{}@{}", self.name, self.marketplace)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct MarketplaceView {
    pub name: String,
    pub source: String,
    pub kind: &'static str,
    pub description: Option<String>,
    pub owner: Option<String>,
    pub plugin_count: usize,
    pub path: String,
    pub updated_at: u64,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct CatalogEntry {
    /// `plugin@marketplace`.
    pub id: String,
    pub name: String,
    pub display_name: Option<String>,
    pub marketplace: String,
    pub description: Option<String>,
    pub version: Option<String>,
    pub author: Option<String>,
    pub category: Option<String>,
    pub tags: Vec<String>,
    pub keywords: Vec<String>,
    pub homepage: Option<String>,
    pub source: String,
    /// False for sources Mira can't install (e.g. npm).
    pub installable: bool,
    pub installed: bool,
    pub enabled: bool,
    pub installed_version: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct PluginDetail {
    #[serde(flatten)]
    pub entry: CatalogEntry,
    pub license: Option<String>,
    pub repository: Option<String>,
    /// Known when the files are available locally (installed, or inside
    /// the marketplace).
    pub components: Option<Components>,
    pub command_names: Vec<String>,
    pub agent_names: Vec<String>,
    pub readme: Option<String>,
    pub path: Option<String>,
}

/// One enabled plugin's contributions.
#[derive(Clone, Debug)]
pub struct EnabledPlugin {
    pub id: String,
    pub name: String,
    pub root: PathBuf,
    pub components: Components,
}

#[derive(Clone, Debug, Default)]
pub struct Enabled {
    pub plugins: Vec<EnabledPlugin>,
}

impl Enabled {
    pub fn skill_dirs(&self) -> Vec<PathBuf> {
        self.plugins
            .iter()
            .flat_map(|p| p.components.skill_dirs.clone())
            .collect()
    }

    pub fn agent_files(&self) -> Vec<(String, PathBuf)> {
        self.plugins
            .iter()
            .flat_map(|p| {
                p.components
                    .agents
                    .iter()
                    .map(|a| (p.name.clone(), a.clone()))
            })
            .collect()
    }

    /// For [`crate::commands::load`].
    pub fn command_files(&self) -> Vec<(String, Vec<PathBuf>)> {
        self.plugins
            .iter()
            .map(|p| (p.name.clone(), p.components.commands.clone()))
            .collect()
    }

    /// MCP server definitions for mira-mcp, plus problems.
    pub fn mcp_specs(&self) -> (Vec<mira_mcp::ServerSpec>, Vec<String>) {
        let mut specs = Vec::new();
        let mut problems = Vec::new();
        for p in &self.plugins {
            let (s, pr) = components::mcp_specs(&p.name, &p.root, &p.components);
            specs.extend(s);
            problems.extend(pr.into_iter().map(|e| format!("{}: {e}", p.id)));
        }
        (specs, problems)
    }
}

#[derive(Clone, Debug)]
pub struct PluginManager {
    root: PathBuf,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

fn read_json<T: for<'de> Deserialize<'de> + Default>(p: &Path) -> Result<T> {
    match std::fs::read(p) {
        Ok(b) => serde_json::from_slice(&b).with_context(|| format!("parse {}", p.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(e).with_context(|| format!("read {}", p.display())),
    }
}

fn write_json<T: Serialize>(p: &Path, v: &T) -> Result<()> {
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = p.with_extension("tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(v)?)?;
    std::fs::rename(&tmp, p)?;
    Ok(())
}

impl PluginManager {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// `~/.mira/plugins`.
    pub fn default_root() -> PathBuf {
        mira_config::global_path()
            .parent()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(".mira"))
            .join("plugins")
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn known_path(&self) -> PathBuf {
        self.root.join("known_marketplaces.json")
    }

    fn installed_path(&self) -> PathBuf {
        self.root.join("installed_plugins.json")
    }

    pub fn known(&self) -> Result<BTreeMap<String, KnownMarketplace>> {
        read_json(&self.known_path())
    }

    pub fn installed_map(&self) -> Result<BTreeMap<String, InstalledPlugin>> {
        read_json(&self.installed_path())
    }

    fn load_marketplace(dir: &Path) -> Result<Marketplace> {
        let (path, text) = read_manifest_file(dir, "marketplace.json")
            .ok_or_else(|| anyhow!("no .claude-plugin/marketplace.json in {}", dir.display()))?;
        serde_json::from_str(&text).with_context(|| format!("parse {}", path.display()))
    }

    pub fn marketplaces(&self) -> Result<Vec<MarketplaceView>> {
        Ok(self
            .known()?
            .into_iter()
            .map(|(name, k)| {
                let loaded = Self::load_marketplace(&k.path);
                let (description, owner, count, error) = match &loaded {
                    Ok(m) => (
                        m.description
                            .clone()
                            .or_else(|| m.metadata.description.clone()),
                        m.owner.as_ref().map(|o| o.name.clone()),
                        m.plugins.len(),
                        None,
                    ),
                    Err(e) => (None, None, 0, Some(format!("{e:#}"))),
                };
                MarketplaceView {
                    name,
                    source: k.source.label(),
                    kind: match k.source {
                        MarketplaceSource::Github { .. } => "github",
                        MarketplaceSource::Git { .. } => "git",
                        MarketplaceSource::Directory { .. } => "directory",
                        MarketplaceSource::Url { .. } => "url",
                    },
                    description,
                    owner,
                    plugin_count: count,
                    path: k.path.display().to_string(),
                    updated_at: k.updated_at,
                    error,
                }
            })
            .collect())
    }

    /// Fetch a marketplace's files into `dest`.
    async fn fetch_marketplace(source: &MarketplaceSource, dest: &Path) -> Result<()> {
        if let Some((url, git_ref)) = source.git_url() {
            return git::clone(&url, dest, git_ref.as_deref(), None).await;
        }
        if let MarketplaceSource::Url { url } = source {
            let text = reqwest::get(url)
                .await
                .and_then(|r| r.error_for_status())
                .with_context(|| format!("download {url}"))?
                .text()
                .await?;
            serde_json::from_str::<Marketplace>(&text)
                .with_context(|| format!("{url} isn't a marketplace.json"))?;
            let manifest = dest.join(".claude-plugin").join("marketplace.json");
            std::fs::create_dir_all(manifest.parent().unwrap())?;
            std::fs::write(manifest, text)?;
            return Ok(());
        }
        Ok(())
    }

    /// Add a marketplace. Returns its name (from its `marketplace.json`).
    pub async fn add_marketplace(&self, input: &str) -> Result<String> {
        let source = MarketplaceSource::parse(input)?;
        let _g = LOCK.lock().await;
        let mut known = self.known()?;
        if let Some((name, _)) = known.iter().find(|(_, k)| k.source == source) {
            bail!("already added as `{name}`");
        }
        let dir_root = self.root.join("marketplaces");
        std::fs::create_dir_all(&dir_root)?;
        let (path, temp) = match &source {
            MarketplaceSource::Directory { path } => (path.clone(), None),
            _ => {
                let tmp = dir_root.join(format!(".adding-{}", now_nanos()));
                let _ = std::fs::remove_dir_all(&tmp);
                if let Err(e) = Self::fetch_marketplace(&source, &tmp).await {
                    let _ = std::fs::remove_dir_all(&tmp);
                    return Err(e);
                }
                (tmp.clone(), Some(tmp))
            }
        };
        let market = match Self::load_marketplace(&path) {
            Ok(m) => m,
            Err(e) => {
                if let Some(t) = &temp {
                    let _ = std::fs::remove_dir_all(t);
                }
                return Err(e);
            }
        };
        let name = market.name.trim().to_owned();
        if !valid_name(&name) {
            if let Some(t) = &temp {
                let _ = std::fs::remove_dir_all(t);
            }
            bail!("marketplace name `{name}` must be letters, digits, `-`, `_` or `.`");
        }
        if known.contains_key(&name) {
            if let Some(t) = &temp {
                let _ = std::fs::remove_dir_all(t);
            }
            bail!("a marketplace named `{name}` is already added; remove it first");
        }
        let final_path = match temp {
            Some(t) => {
                let dest = dir_root.join(&name);
                let _ = std::fs::remove_dir_all(&dest);
                std::fs::rename(&t, &dest)?;
                dest
            }
            None => path,
        };
        let t = now();
        known.insert(
            name.clone(),
            KnownMarketplace {
                source,
                path: final_path,
                added_at: t,
                updated_at: t,
            },
        );
        write_json(&self.known_path(), &known)?;
        Ok(name)
    }

    pub async fn update_marketplace(&self, name: &str) -> Result<()> {
        let _g = LOCK.lock().await;
        let mut known = self.known()?;
        let k = known
            .get_mut(name)
            .ok_or_else(|| anyhow!("no marketplace `{name}`"))?;
        match &k.source {
            MarketplaceSource::Directory { .. } => {}
            MarketplaceSource::Url { .. } => {
                let tmp = k.path.with_extension("updating");
                let _ = std::fs::remove_dir_all(&tmp);
                Self::fetch_marketplace(&k.source, &tmp).await?;
                let _ = std::fs::remove_dir_all(&k.path);
                std::fs::rename(&tmp, &k.path)?;
            }
            _ => {
                if k.path.join(".git").exists() {
                    git::pull(&k.path).await?;
                } else {
                    let tmp = k.path.with_extension("updating");
                    let _ = std::fs::remove_dir_all(&tmp);
                    Self::fetch_marketplace(&k.source, &tmp).await?;
                    let _ = std::fs::remove_dir_all(&k.path);
                    std::fs::rename(&tmp, &k.path)?;
                }
            }
        }
        Self::load_marketplace(&k.path)?;
        k.updated_at = now();
        write_json(&self.known_path(), &known)
    }

    /// Remove a marketplace and uninstall its plugins.
    pub async fn remove_marketplace(&self, name: &str) -> Result<()> {
        let _g = LOCK.lock().await;
        let mut known = self.known()?;
        let k = known
            .remove(name)
            .ok_or_else(|| anyhow!("no marketplace `{name}`"))?;
        let mut installed = self.installed_map()?;
        installed.retain(|_, p| p.marketplace != name);
        write_json(&self.installed_path(), &installed)?;
        let _ = std::fs::remove_dir_all(self.root.join("cache").join(name));
        if !matches!(k.source, MarketplaceSource::Directory { .. })
            && k.path.starts_with(&self.root)
        {
            let _ = std::fs::remove_dir_all(&k.path);
        }
        write_json(&self.known_path(), &known)
    }

    /// Every plugin in every marketplace.
    pub fn catalog(&self) -> Result<Vec<CatalogEntry>> {
        let installed = self.installed_map()?;
        let mut out = Vec::new();
        for (mname, k) in self.known()? {
            let Ok(m) = Self::load_marketplace(&k.path) else {
                continue;
            };
            for e in &m.plugins {
                out.push(catalog_entry(&mname, e, &installed));
            }
        }
        Ok(out)
    }

    fn find_entry(&self, id: &str) -> Result<(String, KnownMarketplace, Marketplace, PluginEntry)> {
        let (plugin, market) = split_id(id)?;
        let known = self.known()?;
        let candidates: Vec<(&String, &KnownMarketplace)> = match market {
            Some(m) => known.get_key_value(m).into_iter().collect(),
            None => known.iter().collect(),
        };
        let mut hits = Vec::new();
        for (mname, k) in candidates {
            let Ok(m) = Self::load_marketplace(&k.path) else {
                continue;
            };
            let wanted = m.renames.get(plugin).map(String::as_str).unwrap_or(plugin);
            if let Some(e) = m.plugins.iter().find(|e| e.name == wanted).cloned() {
                hits.push((mname.clone(), k.clone(), m, e));
            }
        }
        match hits.len() {
            0 => bail!("no plugin `{id}` in your marketplaces"),
            1 => Ok(hits.remove(0)),
            _ => bail!(
                "`{plugin}` is in several marketplaces ({}); use {plugin}@<marketplace>",
                hits.iter()
                    .map(|h| h.0.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }

    pub fn detail(&self, id: &str) -> Result<PluginDetail> {
        let (mname, k, market, entry) = self.find_entry(id)?;
        let installed = self.installed_map()?;
        let ce = catalog_entry(&mname, &entry, &installed);
        let local = match installed.get(&ce.id) {
            Some(p) => Some(p.path.clone()),
            None => match &entry.source {
                PluginSource::Relative(rel) => relative_dir(&k.path, &market, rel).ok(),
                _ => None,
            },
        };
        let (components, readme, manifest) = match &local {
            Some(dir) if dir.is_dir() => {
                let manifest = read_plugin_manifest(dir).merge_entry(&entry);
                let c = components::discover(dir, &manifest);
                let readme = std::fs::read_to_string(dir.join("README.md"))
                    .ok()
                    .map(|t| t.chars().take(20_000).collect());
                (Some(c), readme, Some(manifest))
            }
            _ => (None, None, None),
        };
        let stems = |files: &[PathBuf]| {
            files
                .iter()
                .filter_map(|f| f.file_stem().map(|s| s.to_string_lossy().into_owned()))
                .collect::<Vec<_>>()
        };
        Ok(PluginDetail {
            license: manifest
                .as_ref()
                .and_then(|m| m.license.clone())
                .or(entry.license.clone()),
            repository: entry.repository.clone(),
            command_names: components
                .as_ref()
                .map(|c| stems(&c.commands))
                .unwrap_or_default(),
            agent_names: components
                .as_ref()
                .map(|c| stems(&c.agents))
                .unwrap_or_default(),
            components,
            readme,
            path: local.map(|p| p.display().to_string()),
            entry: ce,
        })
    }

    /// Install (or reinstall) a plugin: `name` or `name@marketplace`.
    pub async fn install(&self, id: &str) -> Result<InstalledPlugin> {
        let (mname, k, market, entry) = self.find_entry(id)?;
        let _g = LOCK.lock().await;
        let cache = self.root.join("cache");
        std::fs::create_dir_all(&cache)?;
        let scratch = cache.join(format!(".fetch-{}", now_nanos()));
        let result = async {
            let (src, sha) = match &entry.source {
                PluginSource::Relative(rel) => {
                    // Versioned by the marketplace's commit, when it has one.
                    let sha = if k.path.join(".git").exists() {
                        git::head(&k.path).await.ok()
                    } else {
                        None
                    };
                    (relative_dir(&k.path, &market, rel)?, sha)
                }
                PluginSource::Github {
                    repo,
                    git_ref,
                    sha,
                    path,
                } => {
                    let url = format!("https://github.com/{repo}.git");
                    git::clone(&url, &scratch, git_ref.as_deref(), sha.as_deref()).await?;
                    let head = git::head(&scratch).await.ok();
                    (subdir(&scratch, path.as_deref())?, head)
                }
                PluginSource::Git {
                    url,
                    git_ref,
                    sha,
                    path,
                } => {
                    git::clone(url, &scratch, git_ref.as_deref(), sha.as_deref()).await?;
                    let head = git::head(&scratch).await.ok();
                    (subdir(&scratch, path.as_deref())?, head)
                }
                PluginSource::Unknown => {
                    bail!("`{}` uses a source Mira can't install yet", entry.name)
                }
            };
            if !src.is_dir() {
                bail!("plugin directory {} doesn't exist", src.display());
            }
            let manifest = read_plugin_manifest(&src).merge_entry(&entry);
            let version = manifest
                .version
                .clone()
                .or_else(|| sha.as_ref().map(|s| s.chars().take(12).collect()))
                .unwrap_or_else(|| "local".into());
            let safe_version: String = version
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                        c
                    } else {
                        '_'
                    }
                })
                .collect();
            let plugin_dir = cache.join(&mname).join(&entry.name);
            let dest = plugin_dir.join(&safe_version);
            let _ = std::fs::remove_dir_all(&plugin_dir);
            copy_dir(&src, &dest)?;
            Ok::<_, anyhow::Error>((dest, version, sha))
        }
        .await;
        let _ = std::fs::remove_dir_all(&scratch);
        let (dest, version, sha) = result?;

        let mut installed = self.installed_map()?;
        let key = format!("{}@{mname}", entry.name);
        let t = now();
        let record = InstalledPlugin {
            name: entry.name.clone(),
            marketplace: mname,
            version,
            path: dest,
            enabled: installed.get(&key).map(|p| p.enabled).unwrap_or(true),
            installed_at: installed.get(&key).map(|p| p.installed_at).unwrap_or(t),
            updated_at: t,
            sha,
        };
        installed.insert(key, record.clone());
        write_json(&self.installed_path(), &installed)?;
        Ok(record)
    }

    pub async fn uninstall(&self, id: &str) -> Result<()> {
        let _g = LOCK.lock().await;
        let mut installed = self.installed_map()?;
        let key = self.installed_key(&installed, id)?;
        let p = installed.remove(&key).expect("key exists");
        if let Some(dir) = p.path.parent() {
            if dir.starts_with(self.root.join("cache")) {
                let _ = std::fs::remove_dir_all(dir);
            }
        }
        write_json(&self.installed_path(), &installed)
    }

    pub async fn set_enabled(&self, id: &str, enabled: bool) -> Result<()> {
        let _g = LOCK.lock().await;
        let mut installed = self.installed_map()?;
        let key = self.installed_key(&installed, id)?;
        installed.get_mut(&key).expect("key exists").enabled = enabled;
        write_json(&self.installed_path(), &installed)
    }

    fn installed_key(
        &self,
        installed: &BTreeMap<String, InstalledPlugin>,
        id: &str,
    ) -> Result<String> {
        if installed.contains_key(id) {
            return Ok(id.to_owned());
        }
        let hits: Vec<&String> = installed
            .iter()
            .filter(|(_, p)| p.name == id)
            .map(|(k, _)| k)
            .collect();
        match hits.as_slice() {
            [one] => Ok((*one).clone()),
            [] => bail!("`{id}` isn't installed"),
            _ => bail!("`{id}` is installed from several marketplaces; use name@marketplace"),
        }
    }

    /// Installed plugins with their components.
    pub fn installed(&self) -> Result<Vec<(InstalledPlugin, Option<PluginManifest>, Components)>> {
        Ok(self
            .installed_map()?
            .into_values()
            .map(|p| {
                let manifest = p.path.is_dir().then(|| read_plugin_manifest(&p.path));
                let entry = self.find_entry(&p.id()).ok().map(|(_, _, _, e)| e);
                let merged = match (&manifest, &entry) {
                    (Some(m), Some(e)) => Some(m.clone().merge_entry(e)),
                    (m, _) => m.clone(),
                };
                let mut c = merged
                    .as_ref()
                    .map(|m| components::discover(&p.path, m))
                    .unwrap_or_default();
                if !p.path.is_dir() {
                    c.problems
                        .push(format!("files missing at {}; reinstall", p.path.display()));
                }
                (p, merged, c)
            })
            .collect())
    }

    /// What enabled plugins contribute.
    pub fn enabled(&self) -> Enabled {
        let list = self.installed().unwrap_or_default();
        Enabled {
            plugins: list
                .into_iter()
                .filter(|(p, _, _)| p.enabled && p.path.is_dir())
                .map(|(p, _, components)| EnabledPlugin {
                    id: p.id(),
                    name: p.name.clone(),
                    root: p.path.clone(),
                    components,
                })
                .collect(),
        }
    }
}

fn now_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default()
}

fn split_id(id: &str) -> Result<(&str, Option<&str>)> {
    let (p, m) = match id.split_once('@') {
        Some((p, m)) => (p, Some(m)),
        None => (id, None),
    };
    if !valid_name(p) || m.is_some_and(|m| !valid_name(m)) {
        bail!("`{id}` isn't a plugin name (plugin or plugin@marketplace)");
    }
    Ok((p, m))
}

fn catalog_entry(
    market: &str,
    e: &PluginEntry,
    installed: &BTreeMap<String, InstalledPlugin>,
) -> CatalogEntry {
    let id = format!("{}@{market}", e.name);
    let inst = installed.get(&id);
    CatalogEntry {
        name: e.name.clone(),
        display_name: e.display_name.clone(),
        marketplace: market.to_owned(),
        description: e.description.clone(),
        version: e.version.clone(),
        author: e.author.as_ref().map(|a| a.name.clone()),
        category: e.category.clone(),
        tags: e.tags.clone(),
        keywords: e.keywords.clone(),
        homepage: e.homepage.clone(),
        source: e.source.label(),
        installable: e.source != PluginSource::Unknown,
        installed: inst.is_some(),
        enabled: inst.is_some_and(|p| p.enabled),
        installed_version: inst.map(|p| p.version.clone()),
        id,
    }
}

fn read_plugin_manifest(dir: &Path) -> PluginManifest {
    read_manifest_file(dir, "plugin.json")
        .and_then(|(_, t)| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// A path inside `base`, refusing `..` and absolute paths.
fn contained(base: &Path, rel: &str) -> Result<PathBuf> {
    let p = Path::new(rel);
    if p.is_absolute()
        || p.components()
            .any(|c| matches!(c, Component::ParentDir | Component::Prefix(_)))
    {
        bail!("`{rel}` points outside the marketplace");
    }
    Ok(base.join(p))
}

fn relative_dir(market_dir: &Path, m: &Marketplace, rel: &str) -> Result<PathBuf> {
    let explicit = rel.starts_with("./") || rel.starts_with('.');
    let base = match (&m.metadata.plugin_root, explicit) {
        (Some(root), false) => contained(market_dir, root)?,
        _ => market_dir.to_path_buf(),
    };
    contained(&base, rel)
}

fn subdir(root: &Path, path: Option<&str>) -> Result<PathBuf> {
    match path {
        Some(p) => contained(root, p),
        None => Ok(root.to_path_buf()),
    }
}

/// Copy a plugin's files, leaving out `.git`.
fn copy_dir(src: &Path, dest: &Path) -> Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        if name == ".git" {
            continue;
        }
        let from = entry.path();
        let to = dest.join(&name);
        let ft = entry.file_type()?;
        if ft.is_dir() {
            copy_dir(&from, &to)?;
        } else if ft.is_symlink() {
            #[cfg(unix)]
            {
                let target = std::fs::read_link(&from)?;
                std::os::unix::fs::symlink(target, &to)?;
            }
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_marketplace_inputs() {
        use MarketplaceSource as S;
        assert_eq!(
            S::parse("anthropics/claude-code").unwrap(),
            S::Github {
                repo: "anthropics/claude-code".into(),
                git_ref: None
            }
        );
        assert_eq!(
            S::parse("https://github.com/o/r.git").unwrap(),
            S::Github {
                repo: "o/r".into(),
                git_ref: None
            }
        );
        assert_eq!(
            S::parse("o/r#v2").unwrap(),
            S::Github {
                repo: "o/r".into(),
                git_ref: Some("v2".into())
            }
        );
        assert!(matches!(
            S::parse("https://x.dev/marketplace.json").unwrap(),
            S::Url { .. }
        ));
        assert!(matches!(
            S::parse("git@github.com:o/r.git").unwrap(),
            S::Git { .. }
        ));
        assert!(matches!(
            S::parse("https://gitlab.com/o/r").unwrap(),
            S::Git { .. }
        ));
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            S::parse(&dir.path().display().to_string()).unwrap(),
            S::Directory { .. }
        ));
        assert!(S::parse("not a thing").is_err());
        assert!(S::parse("../x").is_err());
    }

    #[test]
    fn refuses_escaping_paths() {
        let base = Path::new("/m");
        assert!(contained(base, "../x").is_err());
        assert!(contained(base, "/etc").is_err());
        assert_eq!(contained(base, "./a/b").unwrap(), Path::new("/m/a/b"));
        assert!(split_id("../x").is_err());
        assert_eq!(split_id("a@b").unwrap(), ("a", Some("b")));
    }
}
