//! Guards that every `Entry` kind is actually rendered by `EntryView`.
//!
//! # Why this is a Rust test and not a frontend one
//!
//! `EntryView` switches on `entry.kind` with no exhaustiveness check, so
//! adding an `Entry` variant type-checks, passes `tsc`, and renders nothing —
//! which in a transcript is indistinguishable from a wedged agent. The
//! `default:` arm added to `EntryView` makes that visible at runtime, but
//! nothing catches it at build time.
//!
//! A `never` assertion would catch it, but it is mutually exclusive with the
//! graceful `default:` we want: `never` requires every variant handled and
//! permits no fallback. So this test covers the build-time half and the
//! `default:` covers the runtime half.
//!
//! It lives here for the same reason as `acp_wire_drift.rs`: frontend test
//! infrastructure does not exist yet (#73), and a test nobody can run is not
//! a test.

use std::collections::BTreeSet;
use std::path::PathBuf;

fn frontend(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("frontend")
        .join(rel)
}

/// Every `kind` reachable from the `Entry` union.
///
/// The union is written as a list of type *aliases* (`| MsgEntry`,
/// `| ToolEntry`, …) rather than inline object types, so this resolves each
/// alias to its `kind: '...'` declaration rather than scanning the union
/// body. Scanning the union body finds nothing, which is why the first
/// version of this test reported zero kinds and would have passed vacuously.
fn entry_kinds() -> BTreeSet<String> {
    let src = std::fs::read_to_string(frontend("src/App.tsx")).expect("read App.tsx");
    let after = src
        .split_once("export type Entry =")
        .expect("Entry union")
        .1;
    let body = after.split_once("\ntype ").map(|(b, _)| b).unwrap_or(after);

    // Union members are `| AliasName`.
    let mut aliases: BTreeSet<&str> = BTreeSet::new();
    for line in body.lines() {
        let t = line.trim();
        if let Some(name) = t.strip_prefix("| ") {
            let name = name.trim();
            if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                aliases.insert(name);
            }
        }
    }

    let mut out = BTreeSet::new();
    for alias in aliases {
        // Find `type Alias = { kind: 'x'` anywhere in the file.
        let needle = format!("type {alias} =");
        let Some(pos) = src.find(&needle) else {
            continue;
        };
        let rest = &src[pos..];
        let Some(k) = rest.find("kind: '") else {
            continue;
        };
        let after_k = &rest[k + "kind: '".len()..];
        let end = after_k.find('\'').unwrap_or(0);
        out.insert(after_k[..end].to_string());
    }
    out
}

/// Every `kind` `EntryView` handles.
fn rendered_kinds() -> BTreeSet<String> {
    let src = std::fs::read_to_string(frontend("src/App.tsx")).expect("read App.tsx");
    let start = src.find("function EntryView").expect("EntryView");
    // The switch is the first `switch (entry.kind)` in the function.
    let sw = src[start..]
        .find("switch (entry.kind)")
        .map(|i| start + i)
        .expect("entry.kind switch");

    let mut out = BTreeSet::new();
    for cap in src[sw..].match_indices("case '") {
        let rest = &src[sw + cap.0 + "case '".len()..];
        let end = rest.find('\'').unwrap_or(0);
        out.insert(rest[..end].to_string());
    }
    out
}

#[test]
fn every_entry_kind_is_rendered() {
    let declared = entry_kinds();
    let rendered = rendered_kinds();

    assert!(
        !declared.is_empty(),
        "no Entry kinds parsed — the parser is probably wrong"
    );

    let missing: Vec<_> = declared.difference(&rendered).collect();
    assert!(
        missing.is_empty(),
        "Entry kinds declared but not rendered by EntryView: {missing:?}. \
         Each would compile cleanly and render nothing."
    );
}

#[test]
fn entry_view_has_a_fallback_arm() {
    // The runtime half of the same protection: a kind we cannot render
    // should say so rather than vanish. Without this, an unknown kind is
    // indistinguishable from a wedged agent.
    let src = std::fs::read_to_string(frontend("src/App.tsx")).expect("read App.tsx");
    let start = src.find("function EntryView").expect("EntryView");
    let tail = &src[start..];
    let sw = tail.find("switch (entry.kind)").expect("switch");
    assert!(
        tail[sw..].contains("default:"),
        "EntryView's switch has no `default:` arm, so an unrendered entry \
         disappears instead of reporting itself"
    );
}

#[test]
fn the_entry_view_parser_finds_a_plausible_number_of_kinds() {
    // Guards against the parser silently matching nothing, which would make
    // the test above vacuously pass.
    let declared = entry_kinds();
    assert!(
        declared.len() >= 5,
        "expected several Entry kinds, parsed {}: {declared:?}",
        declared.len()
    );
    assert!(declared.contains("msg"), "the base message kind is missing");
    assert!(declared.contains("tool"), "the tool kind is missing");
}
