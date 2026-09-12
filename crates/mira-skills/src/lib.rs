//! Named skills — reusable instruction bundles the agent invokes to
//! learn *how* to accomplish a task in this project.
//!
//! Skills are markdown files with YAML frontmatter, the same shape
//! subagents use, but simpler: only `name` and `description` are
//! required; the file body is the skill's instructions.
//!
//! ```markdown
//! ---
//! name: verify
//! description: Verify a code change works by running the app.
//! ---
//! When invoked, do the following:
//!
//! 1. Detect the project type (Cargo.toml, package.json, …).
//! 2. Boot the app via the appropriate command.
//! 3. Exercise the change you made.
//! 4. Report observed behavior back to the user.
//! ```
//!
//! Registry resolution has three tiers, each overriding the previous by
//! `name`:
//!
//! 1. **Built-ins** — bundled markdown files in `crates/mira-skills/skills/`
//!    baked into the binary via `include_str!`. Users get `verify`,
//!    `code-review`, and friends with zero config.
//! 2. **User** — every `*.md` in `~/.mira/skills/` (one level of subdirs
//!    is fine for categorisation).
//! 3. **Project** — every `*.md` in `<cwd>/.mira/skills/`, so a repo can
//!    ship its own "how we deploy" or "how we release" playbook.
//!
//! Loading is deliberately tolerant: a malformed file logs a warning and
//! is skipped rather than aborting boot.
//!
//! Delivery: the `Skill` tool returns the body wrapped in a
//! `<system-reminder>` block so the model reads the instructions as
//! authoritative (belt-and-suspenders — both the tool-result channel and
//! the reminder-tag effect land on the model in a single turn).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use tracing::warn;

/// One named skill. Loaded from a markdown file's YAML frontmatter
/// (`name`, `description`) + body. `source` is where on disk the file
/// came from, useful for the `/api/skills` response so a user can trace
/// which of the three tiers a given skill was picked up from.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Skill {
    /// Machine name — used in `Skill { name: "verify" }`. Keep short
    /// and identifier-ish (kebab-case is fine; the tool-spec enum
    /// preserves whatever the file used).
    pub name: String,
    /// One-line summary of when to use this skill. Baked into the tool
    /// spec so the model picks the right skill per task without having
    /// to read every body.
    pub description: String,
    /// Optional loose grouping (e.g. `review`, `release`, `deploy`).
    /// Purely cosmetic today; UI can group by this later.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// Optional kebab-case icon name (e.g. `shield-check`, `git-branch`,
    /// `bug`, `sparkle`). The frontend maps a curated set of these to
    /// Phosphor icon components; unknown names fall back to `sparkle`.
    /// Purely cosmetic — the tool spec doesn't include it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Optional Tailwind-flavored color name (e.g. `emerald`, `blue`,
    /// `amber`, `pink`). The frontend maps this to a preset badge
    /// palette; unknown names fall back to a stable hash-derived tint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// The markdown body — the actual step-by-step instructions the
    /// agent will follow when the skill is invoked. Wrapped in a
    /// `<system-reminder>` block on delivery so the model treats it as
    /// authoritative, not as generic tool output.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub body: String,
    /// Where this skill was loaded from. `None` for the bundled tier
    /// (they don't live on disk at runtime); `Some(path)` for user or
    /// project skills. Useful in error messages and in the settings UI.
    /// Points at the `SKILL.md` (dir shape) or the bare `foo.md` (flat
    /// shape) — whichever the skill came from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<PathBuf>,
    /// When the skill lives in its own directory alongside supporting
    /// files (helper scripts, prompt templates, reference material),
    /// this is the directory containing `SKILL.md`. The runtime tool
    /// enumerates the sibling files and includes their paths in the
    /// invocation reminder so the model knows to `read_file` them if
    /// the skill body references them.
    /// `None` for bundled builtins and for the flat-file (single-`*.md`)
    /// shape — those have no attached resources.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_dir: Option<PathBuf>,
}

impl Skill {
    /// Enumerate the skill's attached files (siblings of `SKILL.md` in
    /// its own directory). Empty when the skill has no `source_dir`
    /// (bundled builtins, flat-file skills). Filenames are returned as
    /// paths relative to `source_dir` so the tool can quote them back
    /// without leaking absolute paths.
    pub fn attached_files(&self) -> Vec<PathBuf> {
        let Some(dir) = self.source_dir.as_ref() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let Ok(rd) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        for entry in rd.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n,
                None => continue,
            };
            // SKILL.md is the definition, not an attachment.
            if name.eq_ignore_ascii_case("SKILL.md") {
                continue;
            }
            out.push(PathBuf::from(name));
        }
        out.sort();
        out
    }
}

/// Ordered map of skill name → definition. `BTreeMap` for stable
/// listing order (matters for the tool-spec description we hand to the
/// model — a stable list keeps prompt-caching hits stable too).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SkillRegistry {
    #[serde(default)]
    pub skills: BTreeMap<String, Skill>,
}

impl SkillRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Merge `other` on top of `self` — same-name entries in `other` win.
    /// Used to layer user + project tiers over the built-ins.
    pub fn merge(&mut self, other: SkillRegistry) {
        for (name, s) in other.skills {
            self.skills.insert(name, s);
        }
    }

    pub fn get(&self, name: &str) -> Option<&Skill> {
        self.skills.get(name)
    }

    /// All skill names in stable order.
    pub fn names(&self) -> Vec<String> {
        self.skills.keys().cloned().collect()
    }

    /// Full three-tier resolve: bundled → user (~/.mira/skills) →
    /// project (<cwd>/.mira/skills). Each tier overrides the previous
    /// by name. Missing directories are treated as empty.
    pub fn load_layered(user_dir: &Path, project_dir: &Path) -> Self {
        let mut reg = builtin();
        reg.merge(load_dir(user_dir));
        reg.merge(load_dir(project_dir));
        reg
    }
}

/* ---------- built-ins ---------- */

/// Baked-in skill files. `include_str!` pulls each SKILL.md into the
/// binary so users get a working roster without ever creating a config
/// file. Bundled skills follow the directory shape
/// (`skills/<name>/SKILL.md`) so future builtins can bundle helper
/// scripts + templates alongside their instructions, matching the
/// on-disk shape users write to under `~/.mira/skills/`.
///
/// Add new builtins by creating `crates/mira-skills/skills/<name>/SKILL.md`
/// and referencing it here.
const BUILTIN_SOURCES: &[(&str, &str)] = &[
    ("verify", include_str!("../skills/verify/SKILL.md")),
    ("code-review", include_str!("../skills/code-review/SKILL.md")),
    ("init", include_str!("../skills/init/SKILL.md")),
    ("commit", include_str!("../skills/commit/SKILL.md")),
    ("security-review", include_str!("../skills/security-review/SKILL.md")),
    ("skill-creator", include_str!("../skills/skill-creator/SKILL.md")),
    ("git-workflow", include_str!("../skills/git-workflow/SKILL.md")),
    ("debug-systematically", include_str!("../skills/debug-systematically/SKILL.md")),
];

pub fn builtin() -> SkillRegistry {
    let mut reg = SkillRegistry::new();
    for (label, src) in BUILTIN_SOURCES {
        match parse_skill_md(src) {
            Ok(mut s) => {
                // Builtins have no on-disk source; leave `source` = None
                // so callers can distinguish a bundled skill from an
                // override sitting under `~/.mira/skills/verify.md`.
                s.source = None;
                reg.skills.insert(s.name.clone(), s);
            }
            Err(e) => {
                // Builtins are compiled in — a parse error here means
                // the shipped file is broken. Log loudly; don't panic
                // (better to boot with a smaller roster than not at
                // all).
                tracing::error!(builtin = label, %e, "builtin skill failed to parse");
                #[cfg(test)]
                eprintln!("[builtin parse fail] {label}: {e:#}");
            }
        }
    }
    reg
}

/* ---------- disk loader ---------- */

/// Load every skill under `dir` and return them as a registry.
///
/// Two shapes are supported side-by-side:
///
/// * **Flat** — a bare `foo.md` at the top level of `dir`. Skill is
///   `foo` (or whatever the frontmatter's `name:` says).
/// * **Directory** — `foo/SKILL.md`. The skill's directory holds
///   supporting resources (scripts, templates, examples) alongside the
///   SKILL.md; the runtime tool enumerates them and includes their
///   paths in the invocation reminder.
///
/// When a subdirectory contains a `SKILL.md`, it's treated as a skill
/// directory (no recursion into it). When a subdirectory doesn't
/// contain a `SKILL.md`, it's treated as a category folder and we
/// recurse ONE level for either flat `*.md` files or nested skill
/// directories — deep enough for the common `internal/refactor-safely/`
/// pattern, shallow enough that a wayward `node_modules/` in
/// `.mira/skills` can't blow boot time.
///
/// Missing directory → empty registry. Malformed files log a warning
/// and are skipped rather than aborting.
pub fn load_dir(dir: &Path) -> SkillRegistry {
    let mut reg = SkillRegistry::new();
    walk_skills(dir, 0, &mut reg);
    reg
}

const MAX_CATEGORY_DEPTH: usize = 2;

fn walk_skills(dir: &Path, depth: usize, reg: &mut SkillRegistry) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return,
        Err(err) => {
            warn!(?dir, %err, "skills: failed to read directory");
            return;
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        // Flat shape: a `*.md` file at any depth.
        if path.is_file() && has_md_ext(&path) {
            match load_flat_file(&path) {
                Ok(s) => {
                    reg.skills.insert(s.name.clone(), s);
                }
                Err(e) => warn!(?path, %e, "skills: skipped malformed file"),
            }
            continue;
        }
        if !path.is_dir() {
            continue;
        }
        // Directory shape: a folder containing `SKILL.md` is a skill
        // dir. Anything else is a category folder we recurse into.
        let skill_md = path.join("SKILL.md");
        if skill_md.is_file() {
            match load_skill_dir(&path, &skill_md) {
                Ok(s) => {
                    reg.skills.insert(s.name.clone(), s);
                }
                Err(e) => warn!(?path, %e, "skills: skipped malformed skill directory"),
            }
            continue;
        }
        if depth < MAX_CATEGORY_DEPTH {
            walk_skills(&path, depth + 1, reg);
        }
    }
}

fn load_flat_file(path: &Path) -> Result<Skill> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("read skill file {}", path.display()))?;
    let mut s = parse_skill_md(&raw)
        .with_context(|| format!("parse skill file {}", path.display()))?;
    s.source = Some(path.to_path_buf());
    s.source_dir = None;
    Ok(s)
}

fn load_skill_dir(dir: &Path, skill_md: &Path) -> Result<Skill> {
    let raw = std::fs::read_to_string(skill_md)
        .with_context(|| format!("read {}", skill_md.display()))?;
    let mut s = parse_skill_md(&raw)
        .with_context(|| format!("parse {}", skill_md.display()))?;
    s.source = Some(skill_md.to_path_buf());
    s.source_dir = Some(dir.to_path_buf());
    Ok(s)
}

fn has_md_ext(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("md"))
        .unwrap_or(false)
}

/* ---------- frontmatter parser ---------- */

/// Parse a skill file: YAML frontmatter between `---` fences, then the
/// markdown body. Same shape as `mira-agents::parse_agent_md`, kept
/// separate so their schemas can diverge (skills stay simpler on purpose).
pub fn parse_skill_md(raw: &str) -> Result<Skill> {
    let (frontmatter, body) = split_frontmatter(raw)?;
    #[derive(Deserialize)]
    struct Fm {
        name: String,
        description: String,
        #[serde(default)]
        category: Option<String>,
        #[serde(default)]
        icon: Option<String>,
        #[serde(default)]
        color: Option<String>,
    }
    let fm: Fm = serde_yaml::from_str(frontmatter)
        .with_context(|| "parse YAML frontmatter")?;
    if fm.name.trim().is_empty() {
        return Err(anyhow!("skill `name` must be non-empty"));
    }
    if fm.description.trim().is_empty() {
        return Err(anyhow!("skill `description` must be non-empty"));
    }
    let trim_or_none = |s: Option<String>| {
        s.and_then(|v| {
            let t = v.trim().to_owned();
            if t.is_empty() {
                None
            } else {
                Some(t)
            }
        })
    };
    Ok(Skill {
        name: fm.name.trim().to_owned(),
        description: fm.description.trim().to_owned(),
        category: trim_or_none(fm.category),
        icon: trim_or_none(fm.icon),
        color: trim_or_none(fm.color),
        body: body.trim().to_owned(),
        source: None,
        source_dir: None,
    })
}

/// Split `raw` into `(frontmatter, body)`. Frontmatter is delimited by
/// `---` on its own line, first at the very top of the file and again
/// at the end of the block. Files without frontmatter are rejected —
/// name + description are required, and there's no sensible default.
fn split_frontmatter(raw: &str) -> Result<(&str, &str)> {
    let raw = raw.trim_start_matches('\u{feff}'); // BOM
    let raw = raw.strip_prefix("---\n").or_else(|| raw.strip_prefix("---\r\n"))
        .ok_or_else(|| anyhow!("missing YAML frontmatter (must start with `---`)"))?;
    // Find the closing `---` line. Accept both `\n---\n` and `\n---\r\n`.
    let closer_idx = raw
        .find("\n---\n")
        .or_else(|| raw.find("\n---\r\n"))
        .ok_or_else(|| anyhow!("unterminated YAML frontmatter (missing closing `---`)"))?;
    let fm = &raw[..closer_idx];
    // Advance past the closing fence + its trailing newline(s).
    let after = &raw[closer_idx..];
    let body = after
        .strip_prefix("\n---\n")
        .or_else(|| after.strip_prefix("\n---\r\n"))
        .unwrap_or("");
    Ok((fm, body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn parse_minimal_ok() {
        let raw = "---\nname: verify\ndescription: verify a change\n---\nRun the tests.\n";
        let s = parse_skill_md(raw).unwrap();
        assert_eq!(s.name, "verify");
        assert_eq!(s.description, "verify a change");
        assert_eq!(s.body, "Run the tests.");
        assert!(s.category.is_none());
    }

    #[test]
    fn parse_with_category() {
        let raw = "---\nname: verify\ndescription: verify\ncategory: qa\n---\nBody.\n";
        let s = parse_skill_md(raw).unwrap();
        assert_eq!(s.category.as_deref(), Some("qa"));
    }

    #[test]
    fn parse_missing_frontmatter_errors() {
        let raw = "Just a body, no frontmatter.\n";
        assert!(parse_skill_md(raw).is_err());
    }

    #[test]
    fn parse_missing_name_errors() {
        let raw = "---\ndescription: no name\n---\nBody.\n";
        assert!(parse_skill_md(raw).is_err());
    }

    #[test]
    fn parse_empty_description_errors() {
        let raw = "---\nname: verify\ndescription: \"\"\n---\nBody.\n";
        assert!(parse_skill_md(raw).is_err());
    }

    #[test]
    fn parse_unterminated_frontmatter_errors() {
        let raw = "---\nname: verify\ndescription: v\nno closing fence\n";
        assert!(parse_skill_md(raw).is_err());
    }

    #[test]
    fn builtins_load() {
        let reg = builtin();
        assert!(reg.get("verify").is_some(), "expected verify builtin");
        assert!(reg.get("code-review").is_some(), "expected code-review builtin");
        assert!(reg.get("init").is_some(), "expected init builtin");
        // Every builtin should have a non-empty body.
        for name in reg.names() {
            let s = reg.get(&name).unwrap();
            assert!(!s.body.is_empty(), "builtin {name} has empty body");
        }
    }

    #[test]
    fn load_dir_reads_md_files() {
        let tmp = tempdir().unwrap();
        fs::write(
            tmp.path().join("mine.md"),
            "---\nname: mine\ndescription: user-defined\n---\nDo the thing.\n",
        )
        .unwrap();
        fs::write(tmp.path().join("not-a-skill.txt"), "ignored").unwrap();
        let reg = load_dir(tmp.path());
        assert_eq!(reg.names(), vec!["mine".to_string()]);
        assert!(reg.get("mine").unwrap().source.is_some());
    }

    #[test]
    fn load_dir_missing_is_empty_not_error() {
        let tmp = tempdir().unwrap();
        let reg = load_dir(&tmp.path().join("nonexistent"));
        assert!(reg.skills.is_empty());
    }

    #[test]
    fn load_dir_walks_one_subdir_level() {
        let tmp = tempdir().unwrap();
        let sub = tmp.path().join("release");
        fs::create_dir_all(&sub).unwrap();
        fs::write(
            sub.join("cut.md"),
            "---\nname: cut-release\ndescription: cut a release\n---\nSteps.\n",
        )
        .unwrap();
        let reg = load_dir(tmp.path());
        assert!(reg.get("cut-release").is_some());
    }

    #[test]
    fn merge_overrides_by_name() {
        let mut a = SkillRegistry::new();
        a.skills.insert(
            "verify".into(),
            Skill {
                name: "verify".into(),
                description: "old".into(),
                category: None,
                icon: None,
                color: None,
                body: "old body".into(),
                source: None,
                source_dir: None,
            },
        );
        let mut b = SkillRegistry::new();
        b.skills.insert(
            "verify".into(),
            Skill {
                name: "verify".into(),
                description: "new".into(),
                category: None,
                icon: None,
                color: None,
                body: "new body".into(),
                source: None,
                source_dir: None,
            },
        );
        a.merge(b);
        assert_eq!(a.get("verify").unwrap().description, "new");
        assert_eq!(a.get("verify").unwrap().body, "new body");
    }

    #[test]
    fn load_layered_respects_precedence() {
        let tmp = tempdir().unwrap();
        let user_dir = tmp.path().join("user");
        let project_dir = tmp.path().join("project");
        fs::create_dir_all(&user_dir).unwrap();
        fs::create_dir_all(&project_dir).unwrap();
        // Project override wins over user, which wins over builtin.
        fs::write(
            user_dir.join("verify.md"),
            "---\nname: verify\ndescription: user override\n---\nUser body.\n",
        )
        .unwrap();
        fs::write(
            project_dir.join("verify.md"),
            "---\nname: verify\ndescription: project override\n---\nProject body.\n",
        )
        .unwrap();
        let reg = SkillRegistry::load_layered(&user_dir, &project_dir);
        let v = reg.get("verify").unwrap();
        assert_eq!(v.description, "project override");
        assert_eq!(v.body, "Project body.");
    }

    #[test]
    fn load_dir_reads_skill_directory_with_SKILL_md() {
        let tmp = tempdir().unwrap();
        let dir = tmp.path().join("deploy-preview");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("SKILL.md"),
            "---\nname: deploy-preview\ndescription: preview\n---\nDo it.\n",
        )
        .unwrap();
        fs::write(dir.join("check.sh"), "#!/bin/sh\necho hi\n").unwrap();
        fs::write(dir.join("template.md"), "template body").unwrap();
        let reg = load_dir(tmp.path());
        let s = reg.get("deploy-preview").expect("skill loaded");
        assert_eq!(s.source_dir.as_deref(), Some(dir.as_path()));
        let files = s.attached_files();
        // SKILL.md is not listed as an attachment.
        assert!(files.contains(&PathBuf::from("check.sh")));
        assert!(files.contains(&PathBuf::from("template.md")));
        assert!(!files.contains(&PathBuf::from("SKILL.md")));
    }

    #[test]
    fn load_dir_prefers_skill_dir_over_recursion() {
        // A directory with SKILL.md is a SKILL, not a category. Nested
        // markdown files under it should be attachments, not additional
        // skills — otherwise a `references/*.md` explosion inside a
        // skill would flood the registry.
        let tmp = tempdir().unwrap();
        let dir = tmp.path().join("with-refs");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("SKILL.md"),
            "---\nname: with-refs\ndescription: has refs\n---\nBody.\n",
        )
        .unwrap();
        fs::create_dir_all(dir.join("references")).unwrap();
        fs::write(
            dir.join("references").join("fine-print.md"),
            "not a skill",
        )
        .unwrap();
        let reg = load_dir(tmp.path());
        assert!(reg.get("with-refs").is_some());
        // No spurious skill from the nested reference file.
        assert_eq!(reg.skills.len(), 1);
    }

    #[test]
    fn load_layered_falls_through_to_user_then_builtin() {
        let tmp = tempdir().unwrap();
        let user_dir = tmp.path().join("user");
        let project_dir = tmp.path().join("project");
        fs::create_dir_all(&user_dir).unwrap();
        // Only user has a `verify` override; project doesn't → user
        // wins over builtin.
        fs::write(
            user_dir.join("verify.md"),
            "---\nname: verify\ndescription: user override\n---\nUser body.\n",
        )
        .unwrap();
        let reg = SkillRegistry::load_layered(&user_dir, &project_dir);
        assert_eq!(reg.get("verify").unwrap().description, "user override");
        // `code-review` came only from the builtin tier.
        assert!(reg.get("code-review").is_some());
    }
}
