//! Named subagent types loaded from per-agent `.md` files with YAML
//! frontmatter — same shape Claude Code uses for its agent files.
//!
//! Each file looks like:
//!
//! ```markdown
//! ---
//! name: cartographer
//! description: Architecture mapper. Read-only.
//! category: recon
//! tools: [read_file, grep, glob]
//! max_rounds: 20
//! model: claude-sonnet-4-6
//! ---
//! You are Mira's Cartographer subagent...
//! (the markdown body becomes the child's system-prompt addendum)
//! ```
//!
//! Registry resolution has three tiers, each overriding the previous by
//! `name`:
//!
//! 1. **Built-ins** — bundled markdown files in `crates/mira-agents/agents/`
//!    via `include_str!`. Ships in the binary; users get `explore`,
//!    `reviewer`, and `cartographer` with zero config.
//! 2. **User** — every `*.md` in `~/.mira/agents/` (recursive one level
//!    deep — subdirs by category are fine).
//! 3. **Project** — every `*.md` in `<cwd>/.mira/agents/`. Perfect place
//!    for repo-specific specialists ("this repo has a Swift module").
//!
//! Loading is deliberately tolerant: a malformed file logs a warning and
//! gets skipped rather than aborting boot. Unknown tool names in the
//! `tools:` list are validated later at spawn time against the actual
//! tool registry.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use tracing::warn;

/// One named subagent type. Populated by the frontmatter loader — every
/// field except `name`/`description` is optional so a minimal file is
/// still valid.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentType {
    /// Machine name — used in `agent { type: "explore" }`. Keep short
    /// and identifier-ish.
    pub name: String,
    /// One-line summary of what this type is good for. Baked into the
    /// tool spec so the model picks the right type per task.
    pub description: String,
    /// Optional loose grouping (e.g. `recon`, `review`, `codegen`).
    /// Purely cosmetic today; UI can group by this later.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// Allowlist of tool names the child may see. `None` = child inherits
    /// the parent's full tool set. Unknown names log a warning at spawn
    /// time and are silently dropped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<String>>,
    /// Optional per-type model override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Optional per-type round budget.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_rounds: Option<usize>,
    /// The markdown body of the source file — appended to the shared
    /// subagent system prompt as this type's persona / rules.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt_addendum: Option<String>,
    /// Optional JSON Schema. When set, the child's final message must
    /// parse to JSON matching the schema. Not enforced yet — placeholder
    /// for Round 1 structured output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_schema: Option<JsonValue>,
}

/// Ordered map of type name → definition. `BTreeMap` for stable listing
/// order (matters for the tool-spec description we hand to the model).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AgentRegistry {
    #[serde(default)]
    pub types: BTreeMap<String, AgentType>,
}

impl AgentRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Merge `other` on top of `self` — same-name entries in `other` win.
    pub fn merge(&mut self, other: AgentRegistry) {
        for (name, ty) in other.types {
            self.types.insert(name, ty);
        }
    }

    pub fn get(&self, name: &str) -> Option<&AgentType> {
        self.types.get(name)
    }

    /// All type names in stable order.
    pub fn names(&self) -> Vec<String> {
        self.types.keys().cloned().collect()
    }
}

/* ---------- built-ins ---------- */

/// Baked-in agent files. `include_str!` pulls the markdown into the binary
/// so users get a working roster without ever creating a config file.
/// Add new builtins by dropping a file into `crates/mira-agents/agents/`
/// and referencing it here.
const BUILTIN_SOURCES: &[(&str, &str)] = &[
    ("explore.md", include_str!("../agents/explore.md")),
    ("reviewer.md", include_str!("../agents/reviewer.md")),
    ("cartographer.md", include_str!("../agents/cartographer.md")),
    ("coder.md", include_str!("../agents/coder.md")),
    ("documenter.md", include_str!("../agents/documenter.md")),
];

pub fn builtin() -> AgentRegistry {
    let mut reg = AgentRegistry::new();
    for (label, src) in BUILTIN_SOURCES {
        match parse_agent_md(src) {
            Ok(ty) => {
                reg.types.insert(ty.name.clone(), ty);
            }
            Err(e) => {
                // Builtins are compiled in — a parse error here means the
                // shipped file is broken. Log loudly; don't panic (better
                // to boot with a smaller roster than not at all).
                tracing::error!(builtin = label, %e, "builtin agent failed to parse");
            }
        }
    }
    reg
}

/* ---------- disk loader ---------- */

/// Full resolution: builtins → user → project.
pub fn load(cwd: &Path) -> AgentRegistry {
    let mut reg = builtin();

    if let Some(user_dir) = user_agents_dir() {
        reg.merge(load_dir(&user_dir, "user"));
    }

    let project_dir = cwd.join(".mira").join("agents");
    reg.merge(load_dir(&project_dir, "project"));

    reg
}

fn user_agents_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".mira").join("agents"))
}

/// Load every `*.md` in `dir` into a fresh registry. Missing directory
/// is a silent no-op; a broken file logs and gets skipped.
pub fn load_dir(dir: &Path, scope: &str) -> AgentRegistry {
    let mut reg = AgentRegistry::new();
    if !dir.is_dir() {
        return reg;
    }
    let iter = match std::fs::read_dir(dir) {
        Ok(it) => it,
        Err(e) => {
            warn!(scope, dir = %dir.display(), %e, "read_dir failed");
            return reg;
        }
    };
    for entry in iter.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("md") {
            continue;
        }
        match load_file(&path) {
            Ok(ty) => {
                tracing::info!(scope, name = %ty.name, path = %path.display(), "loaded agent");
                reg.types.insert(ty.name.clone(), ty);
            }
            Err(e) => {
                warn!(scope, path = %path.display(), %e, "agent file failed to parse");
            }
        }
    }
    reg
}

fn load_file(path: &Path) -> Result<AgentType> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("read {}", path.display()))?;
    parse_agent_md(&raw)
        .with_context(|| format!("parse {}", path.display()))
}

/* ---------- parser ---------- */

/// Parse a single agent markdown file. Expects a `---` frontmatter block
/// at the top followed by the markdown body — the body becomes the
/// child's system-prompt addendum.
pub fn parse_agent_md(source: &str) -> Result<AgentType> {
    // Trim BOM + surrounding blank lines so files edited on Windows /
    // over the network don't fail with cryptic YAML errors.
    let source = source.trim_start_matches('\u{feff}').trim_start();

    let after_open = source
        .strip_prefix("---\n")
        .or_else(|| source.strip_prefix("---\r\n"))
        .ok_or_else(|| {
            anyhow!("missing frontmatter — expected file to start with `---`")
        })?;

    let end = after_open
        .find("\n---\n")
        .or_else(|| after_open.find("\n---\r\n"))
        // Some editors leave a trailing `---` without a newline after —
        // accept that shape too so `head -c $body` style writes work.
        .or_else(|| {
            if after_open.trim_end().ends_with("\n---") {
                Some(after_open.trim_end().len() - 3)
            } else {
                None
            }
        })
        .ok_or_else(|| anyhow!("unterminated frontmatter — missing closing `---`"))?;

    let fm_raw = &after_open[..end];
    let body_start = end
        + after_open[end..]
            .find('\n')
            .map(|i| i + 1)
            .unwrap_or(after_open[end..].len());
    // Skip the "---\n" closer itself.
    let body = after_open
        .get(body_start..)
        .unwrap_or("")
        .trim_start_matches("---\r\n")
        .trim_start_matches("---\n")
        .trim_start_matches("---")
        .trim_start_matches('\n');

    let fm: Frontmatter = serde_yaml::from_str(fm_raw)
        .context("frontmatter is not valid YAML")?;

    if fm.name.trim().is_empty() {
        return Err(anyhow!("frontmatter is missing required `name`"));
    }

    let body = body.trim().to_owned();
    let system_prompt_addendum = if body.is_empty() { None } else { Some(body) };

    Ok(AgentType {
        name: fm.name.trim().to_ascii_lowercase(),
        description: fm.description.unwrap_or_default(),
        category: fm.category,
        tools: fm.tools,
        model: fm.model,
        max_rounds: fm.max_rounds,
        system_prompt_addendum,
        response_schema: fm.response_schema,
    })
}

/// Wire shape for frontmatter YAML. Every field bar `name` is optional
/// so a minimal file (`--- name: X ---`) still parses.
#[derive(Debug, Deserialize)]
struct Frontmatter {
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    tools: Option<Vec<String>>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    max_rounds: Option<usize>,
    #[serde(default)]
    response_schema: Option<JsonValue>,
}

/* ---------- tests ---------- */

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_registers_expected_types() {
        let reg = builtin();
        for expected in ["explore", "reviewer", "cartographer", "coder", "documenter"] {
            assert!(reg.get(expected).is_some(), "{expected} should be built in");
        }
    }

    #[test]
    fn parse_full_frontmatter() {
        let src = "---\nname: draco\ndescription: Swift specialist\ncategory: codegen\ntools: [read_file, grep]\nmax_rounds: 20\nmodel: claude-sonnet\n---\nYou are Draco.\nYou speak Swift.\n";
        let ty = parse_agent_md(src).unwrap();
        assert_eq!(ty.name, "draco");
        assert_eq!(ty.description, "Swift specialist");
        assert_eq!(ty.category.as_deref(), Some("codegen"));
        assert_eq!(
            ty.tools.as_deref(),
            Some(&["read_file".to_owned(), "grep".to_owned()][..]),
        );
        assert_eq!(ty.max_rounds, Some(20));
        assert_eq!(ty.model.as_deref(), Some("claude-sonnet"));
        assert_eq!(
            ty.system_prompt_addendum.as_deref(),
            Some("You are Draco.\nYou speak Swift."),
        );
    }

    #[test]
    fn parse_minimal_frontmatter() {
        let src = "---\nname: bare\n---\n";
        let ty = parse_agent_md(src).unwrap();
        assert_eq!(ty.name, "bare");
        assert_eq!(ty.description, "");
        assert!(ty.tools.is_none());
        assert!(ty.system_prompt_addendum.is_none());
    }

    #[test]
    fn parse_lowercases_name() {
        let src = "---\nname: DRACO\n---\n";
        let ty = parse_agent_md(src).unwrap();
        assert_eq!(ty.name, "draco");
    }

    #[test]
    fn missing_frontmatter_errors() {
        let src = "no frontmatter here\n";
        assert!(parse_agent_md(src).is_err());
    }

    #[test]
    fn merge_overrides_by_name() {
        let mut a = builtin();
        let src = "---\nname: explore\nmodel: my-fast-model\n---\n";
        let b_reg = {
            let mut r = AgentRegistry::new();
            let t = parse_agent_md(src).unwrap();
            r.types.insert(t.name.clone(), t);
            r
        };
        a.merge(b_reg);
        assert_eq!(
            a.get("explore").unwrap().model.as_deref(),
            Some("my-fast-model"),
        );
        // description came from the override which didn't set one → empty
        assert_eq!(a.get("explore").unwrap().description, "");
    }

    #[test]
    fn load_dir_reads_md_only() {
        let tmp = tempdir();
        std::fs::write(tmp.path().join("scout.md"), "---\nname: scout\n---\n").unwrap();
        std::fs::write(tmp.path().join("ignored.txt"), "not an agent").unwrap();
        let reg = load_dir(tmp.path(), "test");
        assert_eq!(reg.names(), vec!["scout".to_owned()]);
    }

    /// tempfile-free scratch dir helper — keeps this crate dep-light in
    /// dev-deps. Best-effort cleanup via Drop.
    struct TmpDir(PathBuf);
    impl TmpDir {
        fn path(&self) -> &Path { &self.0 }
    }
    impl Drop for TmpDir {
        fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
    }
    fn tempdir() -> TmpDir {
        let mut p = std::env::temp_dir();
        p.push(format!("mira-agents-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        TmpDir(p)
    }
}
