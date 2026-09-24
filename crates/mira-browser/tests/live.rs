//! Live browser check against a real Chromium. Run with
//! `cargo test -p mira-browser --test live -- --ignored` (set
//! MIRA_BROWSER if Chrome isn't in a standard location).

use mira_browser::{Browser, BrowserAction, BrowserOptions};
use serde_json::json;

const PAGE: &str = r#"<html><head><title>Mira test</title></head><body style="height:3000px">
<h1 id=h>Hello</h1>
<button onclick="document.getElementById('h').textContent='Clicked!'">Press me</button>
<form onsubmit="document.getElementById('h').textContent='Sent '+document.getElementById('q').value; return false">
<label for=q>Query</label><input id=q name=q>
</form>
<a href="about:blank" target=_blank>popup</a>
</body></html>"#;

fn act(v: serde_json::Value) -> BrowserAction {
    BrowserAction::from_args(&v).unwrap()
}

fn find_ref(snapshot: &str, needle: &str) -> String {
    let line = snapshot
        .lines()
        .find(|l| l.contains(needle))
        .unwrap_or_else(|| panic!("no `{needle}` in:\n{snapshot}"));
    line[1..line.find(']').unwrap()].to_owned()
}

#[tokio::test]
#[ignore = "needs a Chromium-family browser"]
async fn drives_a_page_end_to_end() {
    let profile = std::env::temp_dir().join(format!("mira-browser-test-{}", std::process::id()));
    let b = Browser::new(BrowserOptions {
        headless: true,
        profile_dir: profile.clone(),
        ..Default::default()
    });
    let url = format!("data:text/html,{}", PAGE.replace('#', "%23"));
    let out = b
        .execute(&act(json!({"action": "navigate", "url": url})))
        .await
        .unwrap();
    assert!(out.text.contains("Page: Mira test"), "{}", out.text);

    let button = find_ref(&out.text, "button \"Press me\"");
    let out = b
        .execute(&act(json!({"action": "click", "ref": button})))
        .await
        .unwrap();
    assert!(out.text.contains("Clicked!"), "{}", out.text);

    let input = find_ref(&out.text, "textbox \"Query\"");
    let out = b
        .execute(&act(
            json!({"action": "type", "ref": input, "text": "rust", "submit": true}),
        ))
        .await
        .unwrap();
    assert!(out.text.contains("Sent rust"), "{}", out.text);

    let out = b
        .execute(&act(json!({"action": "screenshot"})))
        .await
        .unwrap();
    let shot = out.screenshot.expect("image");
    assert!(shot.width > 0 && shot.height > 0);

    // Coordinate click on the button, via screenshot space.
    let pos = b
        .execute(&act(json!({"action": "evaluate", "expression":
            "document.getElementById('h').textContent='Reset'; const r=document.querySelector('button').getBoundingClientRect(); JSON.stringify([r.left+r.width/2, r.top+r.height/2])"})))
        .await
        .unwrap();
    let [x, y]: [f64; 2] = serde_json::from_str(&pos.text).unwrap();
    let out = b
        .execute(&act(json!({"action": "click", "coordinate": [x, y]})))
        .await
        .unwrap();
    assert!(out.text.contains("Clicked!"), "{}", out.text);

    let out = b
        .execute(&act(
            json!({"action": "scroll", "direction": "down", "amount": 5}),
        ))
        .await
        .unwrap();
    assert!(!out.text.contains("scrolled 0/"), "{}", out.text);

    // Popup from target=_blank shows up as a tab.
    let link = find_ref(&out.text, "link \"popup\"");
    b.execute(&act(json!({"action": "click", "ref": link})))
        .await
        .unwrap();
    let tabs = b
        .execute(&act(json!({"action": "list_tabs"})))
        .await
        .unwrap();
    assert!(tabs.text.contains("[1]"), "{}", tabs.text);
    b.execute(&act(json!({"action": "switch_tab", "index": 1})))
        .await
        .unwrap();
    b.execute(&act(json!({"action": "close_tab"})))
        .await
        .unwrap();

    let out = b
        .execute(&act(json!({"action": "key", "key": "ctrl+a"})))
        .await
        .unwrap();
    assert!(out.text.contains("Pressed ctrl+a"));

    b.execute(&act(json!({"action": "close"}))).await.unwrap();
    let _ = std::fs::remove_dir_all(profile);
}
