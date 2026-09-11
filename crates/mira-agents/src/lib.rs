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
    /// Whether multiple instances of this type may run concurrently in
    /// the same parent turn. Read-only agents are safe to parallelize;
    /// write-capable agents (`coder`, `documenter`) need worktree
    /// isolation before parallel is safe, so they default to sequential.
    /// `None` here means the caller falls back to a conservative
    /// tool-inspection default at spawn time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallel_safe: Option<bool>,
    /// When a child of this type triggers a policy `Ask`, forward the
    /// decision to the parent's approver (the same modal the user sees
    /// for their own top-level commands). Read-only agents skip this by
    /// default (nothing dangerous to gate); write-capable types route
    /// to the parent so destructive commands never run silently.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_approvals_to_parent: Option<bool>,
    /// Spawn this type inside an ephemeral `git worktree` branched off
    /// HEAD, so the child's edits land in an isolated checkout. On
    /// return, the harness merges the child's changed/added/deleted
    /// files back into the parent's cwd and tears the worktree down.
    /// Read-only types leave this unset — nothing to isolate. Write-
    /// capable types flip it on to make parallel spawns safe.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<bool>,
    /// Name of another type this one inherits defaults from. During
    /// `resolve_inheritance` the parent's fields fill in any of the
    /// child's unset (`None` / empty) fields — the child's explicit
    /// settings always win. Chains are capped at `MAX_EXTENDS_DEPTH`;
    /// cycles are detected and logged rather than crashing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extends: Option<String>,
    /// Block the child's final summary on human approval before handing
    /// it back to the parent. The harness broadcasts a review request
    /// with the proposed summary; the user can approve, approve with a
    /// note, or deny (which returns an error to the parent). Meant for
    /// autonomous flows where an unattended parent shouldn't act on a
    /// subagent's word without a human in the loop.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_required: Option<bool>,
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

    /// Walk every type that has `extends` set and fold the parent's
    /// fields into the child. The child's explicit settings always win;
    /// the parent only fills in `None`/empty slots. Chains are capped
    /// at `MAX_EXTENDS_DEPTH`, and cycles are detected + logged.
    ///
    /// Idempotent: running twice is a no-op because the second pass sees
    /// already-filled fields and the "child wins" rule leaves them alone.
    pub fn resolve_inheritance(&mut self) {
        let names: Vec<String> = self.types.keys().cloned().collect();
        let mut resolved: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        for name in names {
            let mut path: Vec<String> = Vec::new();
            self.resolve_one(&name, &mut resolved, &mut path);
        }
    }

    fn resolve_one(
        &mut self,
        name: &str,
        resolved: &mut std::collections::HashSet<String>,
        path: &mut Vec<String>,
    ) {
        if resolved.contains(name) {
            return;
        }
        if path.iter().any(|n| n == name) {
            warn!(cycle = ?path, next = name, "extends cycle — leaving unchanged");
            return;
        }
        if path.len() >= MAX_EXTENDS_DEPTH {
            warn!(chain = ?path, "extends chain too deep — capping");
            return;
        }

        let parent_name = self.types.get(name).and_then(|t| t.extends.clone());
        if let Some(pn) = parent_name {
            path.push(name.to_owned());
            self.resolve_one(&pn, resolved, path);
            path.pop();

            if let Some(parent) = self.types.get(&pn).cloned() {
                if let Some(child) = self.types.get_mut(name) {
                    fill_from_parent(child, &parent);
                }
            } else {
                warn!(child = name, extends = %pn, "extends refers to unknown type");
            }
        }
        resolved.insert(name.to_owned());
    }
}

const MAX_EXTENDS_DEPTH: usize = 4;

/// Copy any of `parent`'s fields into `child` where the child hasn't
/// specified its own value. `extends` itself is never propagated — a
/// child that extends `A` doesn't automatically extend `A`'s parent
/// again (the resolver has already done that walk).
fn fill_from_parent(child: &mut AgentType, parent: &AgentType) {
    if child.description.is_empty() {
        child.description = parent.description.clone();
    }
    if child.category.is_none() {
        child.category = parent.category.clone();
    }
    if child.tools.is_none() {
        child.tools = parent.tools.clone();
    }
    if child.model.is_none() {
        child.model = parent.model.clone();
    }
    if child.max_rounds.is_none() {
        child.max_rounds = parent.max_rounds;
    }
    if child.system_prompt_addendum.is_none() {
        child.system_prompt_addendum = parent.system_prompt_addendum.clone();
    }
    if child.response_schema.is_none() {
        child.response_schema = parent.response_schema.clone();
    }
    if child.parallel_safe.is_none() {
        child.parallel_safe = parent.parallel_safe;
    }
    if child.route_approvals_to_parent.is_none() {
        child.route_approvals_to_parent = parent.route_approvals_to_parent;
    }
    if child.worktree.is_none() {
        child.worktree = parent.worktree;
    }
    if child.review_required.is_none() {
        child.review_required = parent.review_required;
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
    ("sentinel.md", include_str!("../agents/sentinel.md")),
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
                // Also emit to stderr during tests so bundle bugs surface
                // even when tracing subscribers aren't wired.
                #[cfg(test)]
                eprintln!("[builtin parse fail] {label}: {e:#}");
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

    reg.resolve_inheritance();
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
        parallel_safe: fm.parallel_safe,
        route_approvals_to_parent: fm.route_approvals_to_parent,
        worktree: fm.worktree,
        extends: fm.extends,
        review_required: fm.review_required,
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
    #[serde(default)]
    parallel_safe: Option<bool>,
    #[serde(default)]
    route_approvals_to_parent: Option<bool>,
    #[serde(default)]
    worktree: Option<bool>,
    #[serde(default)]
    extends: Option<String>,
    #[serde(default)]
    review_required: Option<bool>,
}

/* ---------- tests ---------- */

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_registers_expected_types() {
        let reg = builtin();
        for expected in [
            "explore",
            "reviewer",
            "cartographer",
            "coder",
            "documenter",
            "sentinel",
        ] {
            assert!(reg.get(expected).is_some(), "{expected} should be built in");
        }
    }

    #[test]
    fn builtin_schemas_parse_and_are_valid_json_objects() {
        let reg = builtin();
        for name in ["explore", "reviewer", "sentinel"] {
            let ty = reg.get(name).unwrap_or_else(|| panic!("missing {name}"));
            let schema = ty
                .response_schema
                .as_ref()
                .unwrap_or_else(|| panic!("{name} should have a response_schema"));
            assert!(
                schema.is_object(),
                "{name}'s schema should be a JSON object at the top level"
            );
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
    fn resolve_inheritance_fills_missing_fields() {
        let parent_src = "---\nname: base\ndescription: base type\ntools: [read_file, grep]\nmodel: parent-model\nmax_rounds: 20\n---\nParent addendum.\n";
        let child_src = "---\nname: derived\nextends: base\ndescription: derived type\n---\n";
        let mut reg = AgentRegistry::new();
        reg.types.insert("base".to_owned(), parse_agent_md(parent_src).unwrap());
        reg.types.insert(
            "derived".to_owned(),
            parse_agent_md(child_src).unwrap(),
        );
        reg.resolve_inheritance();

        let d = reg.get("derived").unwrap();
        // Child kept its own description
        assert_eq!(d.description, "derived type");
        // Child inherited unset fields
        assert_eq!(
            d.tools.as_deref(),
            Some(&["read_file".to_owned(), "grep".to_owned()][..])
        );
        assert_eq!(d.model.as_deref(), Some("parent-model"));
        assert_eq!(d.max_rounds, Some(20));
        assert_eq!(d.system_prompt_addendum.as_deref(), Some("Parent addendum."));
    }

    #[test]
    fn resolve_inheritance_child_overrides_win() {
        let parent_src = "---\nname: base\ntools: [read_file, grep]\nmax_rounds: 20\n---\n";
        let child_src = "---\nname: derived\nextends: base\ntools: [glob]\nmax_rounds: 5\n---\n";
        let mut reg = AgentRegistry::new();
        reg.types.insert("base".to_owned(), parse_agent_md(parent_src).unwrap());
        reg.types.insert(
            "derived".to_owned(),
            parse_agent_md(child_src).unwrap(),
        );
        reg.resolve_inheritance();

        let d = reg.get("derived").unwrap();
        assert_eq!(d.tools.as_deref(), Some(&["glob".to_owned()][..]));
        assert_eq!(d.max_rounds, Some(5));
    }

    #[test]
    fn resolve_inheritance_missing_parent_is_noop() {
        let child_src = "---\nname: orphan\nextends: does-not-exist\n---\n";
        let mut reg = AgentRegistry::new();
        reg.types.insert("orphan".to_owned(), parse_agent_md(child_src).unwrap());
        reg.resolve_inheritance();
        // Should not crash, and child stays unchanged.
        let o = reg.get("orphan").unwrap();
        assert!(o.tools.is_none());
    }

    #[test]
    fn resolve_inheritance_cycle_is_noop() {
        let a = "---\nname: a\nextends: b\n---\n";
        let b = "---\nname: b\nextends: a\n---\n";
        let mut reg = AgentRegistry::new();
        reg.types.insert("a".to_owned(), parse_agent_md(a).unwrap());
        reg.types.insert("b".to_owned(), parse_agent_md(b).unwrap());
        // Must terminate; both types remain in the registry.
        reg.resolve_inheritance();
        assert!(reg.get("a").is_some());
        assert!(reg.get("b").is_some());
    }

    #[test]
    fn resolve_inheritance_two_level_chain() {
        let base = "---\nname: base\ntools: [read_file]\nmodel: base-model\n---\n";
        let mid = "---\nname: mid\nextends: base\nmax_rounds: 42\n---\n";
        let leaf = "---\nname: leaf\nextends: mid\ndescription: leaf desc\n---\n";
        let mut reg = AgentRegistry::new();
        reg.types.insert("base".to_owned(), parse_agent_md(base).unwrap());
        reg.types.insert("mid".to_owned(), parse_agent_md(mid).unwrap());
        reg.types.insert("leaf".to_owned(), parse_agent_md(leaf).unwrap());
        reg.resolve_inheritance();

        let l = reg.get("leaf").unwrap();
        // Inherited from `mid`
        assert_eq!(l.max_rounds, Some(42));
        // Transitively inherited from `base` via `mid`
        assert_eq!(l.model.as_deref(), Some("base-model"));
        assert_eq!(l.tools.as_deref(), Some(&["read_file".to_owned()][..]));
        // Own field preserved
        assert_eq!(l.description, "leaf desc");
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
