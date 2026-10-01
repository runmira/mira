//! Guards the hand-maintained boundary between the Rust `ServerMsg` enum and
//! its TypeScript mirror.
//!
//! Nothing links these two at compile time, so a variant added on one side
//! and forgotten on the other produces no error anywhere — the frame is
//! simply ignored by the client, which looks exactly like a hung agent. This
//! test parses both files and fails if they disagree.

use std::collections::BTreeSet;
use std::path::PathBuf;

fn src_path(rel: &str) -> PathBuf {
    // CARGO_MANIFEST_DIR is `crates/mira-server`.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join(rel)
}

/// Every `Acp*` arm declared in the `ServerMsg` enum, as its wire `type`.
fn rust_acp_arms() -> BTreeSet<String> {
    let src = std::fs::read_to_string(src_path("crates/mira-server/src/protocol.rs"))
        .expect("read protocol.rs");

    // Only the enum body: stop at the closing brace before the impl block.
    let after_enum = src
        .split_once("pub enum ServerMsg {")
        .expect("ServerMsg enum")
        .1;
    let body = after_enum.split_once("\n}\n").expect("enum body").0;

    let mut out = BTreeSet::new();
    for line in body.lines() {
        let t = line.trim();
        if !t.starts_with("Acp") {
            continue;
        }
        let name: String = t
            .trim_start_matches("Acp")
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect();
        out.insert(format!("acp_{}", snake(&name)));
    }
    out
}

/// Every `type: 'acp_*'` member of the `ServerMsg` union in `types.ts`.
fn ts_acp_arms() -> BTreeSet<String> {
    let src = std::fs::read_to_string(src_path("crates/mira-server/frontend/src/types.ts"))
        .expect("read types.ts");

    // Scoped to the union, so helper types elsewhere in the file (which have
    // `Acp*` names of their own) cannot pollute the result.
    let after_union = src
        .split_once("export type ServerMsg =")
        .expect("ServerMsg union")
        .1;
    let body = after_union
        .split_once("export type ScratchpadEntry")
        .expect("union body")
        .0;

    // Scan for any `type: 'acp_*'` rather than requiring the member to sit
    // on one line. The first version of this test only matched
    // `| { type: '...' }`, which silently skipped every multi-line member —
    // including `subagent_scratchpad_note`, which predates this test. A
    // guard with a blind spot is worse than none, because it reads as
    // coverage.
    let mut out = BTreeSet::new();
    for cap in body.match_indices("type: 'acp_") {
        let rest = &body[cap.0 + "type: '".len()..];
        let end = rest.find('\'').unwrap_or(0);
        out.insert(rest[..end].to_string());
    }
    out
}

fn snake(camel: &str) -> String {
    let mut out = String::new();
    for (i, c) in camel.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

#[test]
fn the_two_sides_declare_the_same_acp_frames() {
    let rust = rust_acp_arms();
    let ts = ts_acp_arms();

    assert!(
        !rust.is_empty(),
        "no Acp* variants found in protocol.rs — the parser is probably wrong"
    );
    assert!(
        !ts.is_empty(),
        "no acp_* members found in types.ts — the parser is probably wrong"
    );

    let missing_in_ts: Vec<_> = rust.difference(&ts).collect();
    let missing_in_rust: Vec<_> = ts.difference(&rust).collect();

    assert!(
        missing_in_ts.is_empty() && missing_in_rust.is_empty(),
        "ACP wire frames have drifted.\n  \
         in Rust but not types.ts: {missing_in_ts:?}\n  \
         in types.ts but not Rust: {missing_in_rust:?}"
    );
}

#[test]
fn every_frame_name_is_snake_case() {
    // `#[serde(tag = "type", rename_all = "snake_case")]` means a variant that
    // doesn't snake_case simply won't arrive under the name the client
    // expects, and the client would drop it in silence.
    for arm in rust_acp_arms() {
        assert!(
            arm.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
            "{arm} is not snake_case"
        );
    }
}

/// Field names on the agent-status row, compared across the boundary.
///
/// The variant guard above cannot see inside a variant, and `AcpAgentStatus`
/// is the one payload the settings panel reads. A field added in Rust and
/// forgotten in TypeScript is silently `undefined` in the client — which is
/// exactly how the version and auth summary went missing without any error.
#[test]
fn agent_status_fields_match_across_the_boundary() {
    // Rust: the field names of `AcpAgentStatus` as serialized.
    let rust = std::fs::read_to_string(src_path("crates/mira-server/../mira-acp/src/status.rs"))
        .expect("read mira-acp status.rs");
    let after_struct = rust
        .split_once("pub struct AgentStatus {")
        .expect("AgentStatus struct")
        .1;
    let body = after_struct.split_once('}').expect("AgentStatus body").0;
    let rust_fields: BTreeSet<String> = body
        .lines()
        .filter_map(|l| {
            let t = l.trim();
            if !t.starts_with("pub ") {
                return None;
            }
            t.trim_start_matches("pub ")
                .split(':')
                .next()
                .map(|n| n.trim().to_string())
        })
        .collect();

    // TypeScript: the same names on `AcpAgentStatus`.
    let ts = std::fs::read_to_string(src_path("crates/mira-server/frontend/src/types.ts"))
        .expect("read types.ts");
    let after_type = ts
        .split_once("export type AcpAgentStatus = {")
        .expect("AcpAgentStatus type")
        .1;
    let body = after_type
        .split_once("\n};")
        .expect("AcpAgentStatus body")
        .0;
    let ts_fields: BTreeSet<String> = body
        .lines()
        .filter_map(|l| {
            let t = l.trim();
            // Doc-comment lines are not fields.
            if t.is_empty() || t.starts_with("/*") || t.starts_with('*') || t.starts_with("//") {
                return None;
            }
            // `name: T;` or `name?: T;`
            let name = t.split([':', '?']).next()?.trim();
            (!name.is_empty() && name != "}").then(|| name.to_string())
        })
        .collect();

    // Serialization renames nothing on this struct, so the sets must match.
    let only_rust: Vec<_> = rust_fields.difference(&ts_fields).collect();
    let only_ts: Vec<_> = ts_fields.difference(&rust_fields).collect();
    assert!(
        only_rust.is_empty() && only_ts.is_empty(),
        "AgentStatus drifted.\n  only in Rust: {only_rust:?}\n  only in TS:   {only_ts:?}"
    );
}
