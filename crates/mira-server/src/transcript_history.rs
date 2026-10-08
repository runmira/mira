//! Turn-aligned pages over the provider transcript and native sidecar.
//! Cursors validate their prefix, so append-only live work remains compatible
//! while a rewind/rewrite cannot silently splice incompatible history.
use serde::Serialize;
use serde_json::{json, Value};
use std::hash::{Hash, Hasher};

#[derive(Clone, Debug, Serialize)]
pub struct TranscriptPage {
    pub items: Vec<Value>,
    pub next_cursor: Option<String>,
    pub total_turns: usize,
    pub first_turn: usize,
}

pub fn interleave(messages: &[mira_core::Message], lines: &[Value]) -> Vec<Value> {
    let mut out = Vec::new();
    let mut from = 0;
    let mut segment = Vec::new();
    let mut provider_turn = None;
    let mut push_messages = |out: &mut Vec<Value>, start: usize, end: usize| {
        for message in &messages[start..end] {
            if message.role == mira_core::Role::User
                && message.input_intent.as_deref() != Some("steer")
            {
                provider_turn = Some(provider_turn.map_or(0, |n| n + 1));
            }
            out.push(json!({"message":message,"provider_turn_index":provider_turn}));
        }
    };
    if !lines.iter().any(|l| l.get("switch").is_some()) {
        push_messages(&mut out, 0, messages.len());
        out.extend(lines.iter().map(|line| json!({"line":line})));
    } else {
        for line in lines {
            if let Some(marker) = line.get("switch") {
                out.append(&mut segment);
                let to = marker
                    .get("harness_len")
                    .and_then(Value::as_u64)
                    .unwrap_or(from as u64) as usize;
                let to = to.clamp(from, messages.len());
                push_messages(&mut out, from, to);
                from = to;
                out.push(json!({"line":line}));
            } else {
                segment.push(json!({"line":line}));
            }
        }
        out.append(&mut segment);
        push_messages(&mut out, from, messages.len());
    }
    let mut turn = 0usize;
    let mut seen_user = false;
    for item in &mut out {
        let user = item.pointer("/message/role").and_then(Value::as_str) == Some("user")
            || item.pointer("/line/user").is_some();
        let steer = item
            .pointer("/message/input_intent")
            .and_then(Value::as_str)
            == Some("steer")
            || item
                .pointer("/line/user/input_intent")
                .and_then(Value::as_str)
                == Some("steer");
        if user && !steer {
            if seen_user {
                turn += 1;
            }
            seen_user = true;
        }
        item["turn_index"] = json!(turn);
    }
    out
}
fn fingerprint(session: &str, items: &[Value]) -> String {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    session.hash(&mut hash);
    for item in items {
        item.to_string().hash(&mut hash);
    }
    format!("{:016x}", hash.finish())
}
/// Target encoded size of one history page (before compression). A single
/// turn larger than this is still sent whole.
const PAGE_BYTES: usize = 1024 * 1024;

fn turn_of(item: &Value) -> usize {
    item.get("turn_index").and_then(Value::as_u64).unwrap_or(0) as usize
}

pub fn page(
    session: &str,
    items: &[Value],
    cursor: Option<&str>,
    turns: usize,
) -> Result<TranscriptPage, String> {
    let before = if let Some(cursor) = cursor {
        let (offset, hash) = cursor.split_once(':').ok_or("invalid history cursor")?;
        let offset: usize = offset.parse().map_err(|_| "invalid history cursor")?;
        if offset > items.len() || fingerprint(session, &items[..offset]) != hash {
            return Err(
                "history changed; reconnect or reload this chat before loading older messages"
                    .into(),
            );
        }
        offset
    } else {
        items.len()
    };
    let total_turns = items
        .last()
        .and_then(|i| i.get("turn_index"))
        .and_then(Value::as_u64)
        .map_or(0, |n| n as usize + 1);
    let last_turn = items
        .get(before.saturating_sub(1))
        .and_then(|i| i.get("turn_index"))
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;
    let mut first_turn = last_turn.saturating_sub(turns.clamp(1, 50) - 1);
    let mut start = items[..before]
        .iter()
        .position(|i| turn_of(i) >= first_turn)
        .unwrap_or(before);
    let mut page_items = items[start..before].to_vec();
    for item in &mut page_items {
        if let Some(message) = item.get_mut("message") {
            externalize_images(session, message);
        }
    }
    // Byte budget: drop whole older turns while the page is over it, but
    // always keep the newest turn. Older turns come with the next page.
    let sizes: Vec<usize> = page_items.iter().map(|i| i.to_string().len()).collect();
    let mut total: usize = sizes.iter().sum();
    let mut cut = 0;
    while total > PAGE_BYTES {
        let turn = turn_of(&page_items[cut]);
        let Some(next) = page_items[cut..].iter().position(|i| turn_of(i) != turn) else {
            break;
        };
        total -= sizes[cut..cut + next].iter().sum::<usize>();
        cut += next;
    }
    if cut > 0 {
        page_items.drain(..cut);
        start += cut;
        first_turn = page_items.first().map_or(first_turn, turn_of);
    }
    Ok(TranscriptPage {
        items: page_items,
        next_cursor: (start > 0)
            .then(|| format!("{start}:{}", fingerprint(session, &items[..start]))),
        total_turns,
        first_turn,
    })
}
/// Images at or below this size stay inline; small ones aren't worth a
/// round trip.
const INLINE_IMAGE_MAX: usize = 16 * 1024;

/// Stable key for an image's base64 payload, used in its URL.
pub fn image_key(data: &str) -> String {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    data.hash(&mut hash);
    format!("{:016x}{:x}", hash.finish(), data.len())
}

/// Swap a message's large inline images for URLs the browser fetches (and
/// caches) when they scroll into view. Computer-use screenshots are ~1 MB
/// of base64 each and dominate transcript size; sent inline, a handful of
/// them made opening a chat multi-megabyte. The client renders `url` when
/// `data` is empty. Resend/fork never ship images back, so nothing depends
/// on the client holding the bytes.
pub fn externalize_images(session: &str, message: &mut Value) {
    let Some(images) = message.get_mut("images").and_then(Value::as_array_mut) else {
        return;
    };
    for image in images {
        let Some(data) = image.get("data").and_then(Value::as_str) else {
            continue;
        };
        if data.len() <= INLINE_IMAGE_MAX {
            continue;
        }
        let url = format!("/api/sessions/{session}/images/{}", image_key(data));
        image["url"] = json!(url);
        image["data"] = json!("");
    }
}

pub fn latest_state(lines: &[Value]) -> Vec<Value> {
    let mut state = std::collections::BTreeMap::new();
    for line in lines {
        if let Some(kind) = line.pointer("/frame/type").and_then(Value::as_str) {
            if matches!(
                kind,
                "acp_modes" | "acp_config_options" | "acp_commands" | "acp_usage" | "acp_limits"
            ) {
                state.insert(kind.to_owned(), line.clone());
            }
        }
    }
    state.into_values().collect()
}
#[cfg(test)]
mod tests {
    use super::*;

    fn user_turn(text: &str, image_bytes: usize) -> Vec<mira_core::Message> {
        let mut user = mira_core::Message::user(text);
        if image_bytes > 0 {
            user.images = vec![mira_core::ImageData::png("A".repeat(image_bytes))];
        }
        vec![user, mira_core::Message::assistant("ok")]
    }

    #[test]
    fn large_images_become_urls_and_small_ones_stay_inline() {
        let mut messages = user_turn("big", 64 * 1024);
        messages.extend(user_turn("small", 100));
        let items = interleave(&messages, &[]);
        let page = page("s1", &items, None, 20).unwrap();
        let big = &page.items[0]["message"]["images"][0];
        assert_eq!(big["data"], "");
        let key = image_key(&"A".repeat(64 * 1024));
        assert_eq!(big["url"], format!("/api/sessions/s1/images/{key}"));
        let small = &page.items[2]["message"]["images"][0];
        assert_eq!(small["data"].as_str().unwrap().len(), 100);
        assert!(small.get("url").is_none());
    }

    #[test]
    fn byte_budget_drops_older_turns_but_keeps_the_newest() {
        // Each turn's text alone is ~0.4 MiB, so 20 turns can't fit in one page.
        let mut messages = Vec::new();
        for i in 0..5 {
            messages.extend(user_turn(&format!("{i}{}", "x".repeat(400 * 1024)), 0));
        }
        let items = interleave(&messages, &[]);
        let first = page("s1", &items, None, 20).unwrap();
        assert!(first.items.len() < items.len());
        assert_eq!(first.first_turn, 3);
        assert_eq!(first.items.last().unwrap()["turn_index"], 4);
        // The cursor picks up exactly where the budget cut.
        let older = page("s1", &items, first.next_cursor.as_deref(), 20).unwrap();
        assert_eq!(older.items.last().unwrap()["turn_index"], 2);

        // A single turn over budget is still sent whole.
        let huge = interleave(&user_turn(&"y".repeat(2 << 20), 0), &[]);
        assert_eq!(page("s1", &huge, None, 20).unwrap().items.len(), 2);
    }
    #[test]
    fn steering_keeps_provider_and_native_messages_in_the_existing_turn() {
        let mut steer = mira_core::Message::user("continue differently");
        steer.input_intent = Some("steer".into());
        let messages = vec![
            mira_core::Message::user("first"),
            steer,
            mira_core::Message::assistant("done"),
            mira_core::Message::user("next"),
        ];
        let items = interleave(&messages, &[]);
        assert_eq!(items[1]["turn_index"], 0);
        assert_eq!(items[1]["provider_turn_index"], 0);
        assert_eq!(items[3]["turn_index"], 1);
        assert_eq!(items[3]["provider_turn_index"], 1);
        let native = vec![
            json!({"user":{"text":"first"}}),
            json!({"user":{"text":"steer","input_intent":"steer"}}),
            json!({"user":{"text":"next"}}),
        ];
        let items = interleave(&[], &native);
        assert_eq!(items[1]["turn_index"], 0);
        assert_eq!(items[2]["turn_index"], 1);
    }

    #[test]
    fn pages_preserve_whole_turns_and_validate_rewinds() {
        let messages: Vec<_> = (0..65)
            .flat_map(|n| {
                [
                    mira_core::Message::user(format!("user {n}")),
                    mira_core::Message::assistant(format!("answer {n}")),
                ]
            })
            .collect();
        let mut items = interleave(&messages, &[]);
        let latest = page("chat", &items, None, 20).unwrap();
        assert_eq!(latest.first_turn, 45);
        assert_eq!(latest.items.len(), 40);
        let cursor = latest.next_cursor.unwrap();
        items.push(json!({"turn_index":65,"message":{"role":"user","content":"live"}}));
        let older = page("chat", &items, Some(&cursor), 20).unwrap();
        assert_eq!(older.first_turn, 25);
        assert!(page("other", &items, Some(&cursor), 20).is_err());
        items[0]["message"]["content"] = json!("rewritten");
        assert!(page("chat", &items, Some(&cursor), 20).is_err());
    }
    #[test]
    fn engine_switch_order_matches_live_history() {
        let messages = vec![
            mira_core::Message::user("provider"),
            mira_core::Message::assistant("answer"),
            mira_core::Message::user("provider again"),
        ];
        let lines = vec![
            json!({"driver":"codex","switch":{"harness_len":2,"to":"codex"}}),
            json!({"user":{"text":"native"}}),
            json!({"frame":{"type":"acp_text","text":"native answer"}}),
            json!({"switch":{"harness_len":2,"to":"provider"}}),
        ];
        let items = interleave(&messages, &lines);
        assert_eq!(items[3].pointer("/line/user/text").unwrap(), "native");
        assert_eq!(items.last().unwrap()["provider_turn_index"], 1);
        assert_eq!(items.last().unwrap()["turn_index"], 2);
    }
}
