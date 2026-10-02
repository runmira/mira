//! `GET /api/unfurl?url=…` — a link's preview (title, description, image,
//! site name) from its Open Graph tags, for the hover card on links in a
//! transcript.
//!
//! The URLs come from model output, so this must not become a way to make
//! Mira's server talk to things the page can't: only `http(s)`, only public
//! addresses (every resolved address is checked, so a public name that
//! resolves to a private one is refused), no redirects followed blindly
//! (each hop is checked again), a small byte cap and a short timeout.
//! Results are cached for the life of the process.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::Duration;

use axum::extract::Query;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};

const MAX_BYTES: usize = 512 * 1024;
const MAX_HOPS: usize = 4;

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct Preview {
    pub url: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub image: Option<String>,
    pub site_name: Option<String>,
}

#[derive(Deserialize)]
pub struct UnfurlQuery {
    pub url: String,
}

static CACHE: Mutex<Option<HashMap<String, Option<Preview>>>> = Mutex::new(None);

pub async fn unfurl(Query(q): Query<UnfurlQuery>) -> Response {
    if let Some(hit) = CACHE
        .lock()
        .ok()
        .and_then(|c| c.as_ref().and_then(|m| m.get(&q.url).cloned()))
    {
        return respond(hit);
    }
    let found = fetch(&q.url).await.ok();
    if let Ok(mut c) = CACHE.lock() {
        let map = c.get_or_insert_with(HashMap::new);
        if map.len() > 500 {
            map.clear();
        }
        map.insert(q.url.clone(), found.clone());
    }
    respond(found)
}

fn respond(p: Option<Preview>) -> Response {
    match p {
        Some(p) => Json(p).into_response(),
        None => StatusCode::NO_CONTENT.into_response(),
    }
}

/// Loopback, private, link-local, CGNAT, multicast, unspecified — anything
/// that isn't the public internet.
fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => {
            let o = v.octets();
            !(v.is_loopback()
                || v.is_private()
                || v.is_link_local()
                || v.is_broadcast()
                || v.is_unspecified()
                || v.is_multicast()
                || o[0] == 0
                || (o[0] == 100 && (64..128).contains(&o[1])))
        }
        IpAddr::V6(v) => {
            if let Some(v4) = v.to_ipv4_mapped() {
                return is_public(IpAddr::V4(v4));
            }
            let seg = v.segments();
            !(v.is_loopback()
                || v.is_unspecified()
                || v.is_multicast()
                || (seg[0] & 0xfe00) == 0xfc00
                || (seg[0] & 0xffc0) == 0xfe80)
        }
    }
}

async fn check_public(url: &reqwest::Url) -> Result<(), String> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err("only http(s)".into());
    }
    let host = url.host_str().ok_or("no host")?.to_string();
    let port = url.port_or_known_default().unwrap_or(443);
    let addrs: Vec<_> = tokio::net::lookup_host((host.as_str(), port))
        .await
        .map_err(|e| e.to_string())?
        .collect();
    if addrs.is_empty() || !addrs.iter().all(|a| is_public(a.ip())) {
        return Err("not a public address".into());
    }
    Ok(())
}

async fn fetch(raw: &str) -> Result<Preview, String> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(6))
        .user_agent("Mozilla/5.0 (compatible; MiraLinkPreview/1.0)")
        .build()
        .map_err(|e| e.to_string())?;
    let mut url = reqwest::Url::parse(raw).map_err(|e| e.to_string())?;
    for _ in 0..MAX_HOPS {
        check_public(&url).await?;
        let mut resp = client
            .get(url.clone())
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if resp.status().is_redirection() {
            let next = resp
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|l| l.to_str().ok())
                .ok_or("redirect without location")?;
            url = url.join(next).map_err(|e| e.to_string())?;
            continue;
        }
        if !resp.status().is_success() {
            return Err(format!("status {}", resp.status()));
        }
        let html_ok = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|c| c.to_str().ok())
            .is_some_and(|c| c.contains("html"));
        if !html_ok {
            return Err("not a page".into());
        }
        let mut body = Vec::new();
        while let Some(chunk) = resp.chunk().await.map_err(|e| e.to_string())? {
            body.extend_from_slice(&chunk);
            if body.len() >= MAX_BYTES {
                break;
            }
        }
        let mut p = parse(&String::from_utf8_lossy(&body), &url);
        p.url = url.to_string();
        if p.title.is_none() && p.description.is_none() {
            return Err("no preview".into());
        }
        return Ok(p);
    }
    Err("too many redirects".into())
}

/// Open Graph / Twitter / `<title>` from a page's head.
fn parse(html: &str, base: &reqwest::Url) -> Preview {
    let head = &html[..html.find("</head>").unwrap_or(html.len().min(200_000))];
    let mut meta: HashMap<String, String> = HashMap::new();
    let lower = head.to_ascii_lowercase();
    let mut at = 0;
    while let Some(i) = lower[at..].find("<meta") {
        let start = at + i;
        let end = lower[start..]
            .find('>')
            .map(|e| start + e)
            .unwrap_or(lower.len());
        let tag = &head[start..end];
        let key = attr(tag, "property").or_else(|| attr(tag, "name"));
        if let (Some(k), Some(v)) = (key, attr(tag, "content")) {
            meta.entry(k.to_ascii_lowercase()).or_insert(v);
        }
        at = end;
    }
    let title_tag = lower.find("<title").and_then(|i| {
        let open = i + lower[i..].find('>')? + 1;
        let close = open + lower[open..].find("</title")?;
        Some(head[open..close].to_string())
    });
    let get = |keys: &[&str]| keys.iter().find_map(|k| meta.get(*k).cloned());
    let clean = |s: String| {
        let s = decode_entities(s.trim());
        (!s.is_empty()).then(|| s.chars().take(300).collect::<String>())
    };
    Preview {
        url: String::new(),
        title: get(&["og:title", "twitter:title"])
            .or(title_tag)
            .and_then(clean),
        description: get(&["og:description", "twitter:description", "description"]).and_then(clean),
        image: get(&["og:image", "twitter:image"])
            .and_then(|i| base.join(i.trim()).ok())
            .filter(|u| u.scheme() == "https")
            .map(|u| u.to_string()),
        site_name: get(&["og:site_name"]).and_then(clean),
    }
}

/// The value of `name="…"` (or single-quoted) in a tag.
fn attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find(name) {
        let i = from + i;
        let before_ok = i == 0 || lower.as_bytes()[i - 1].is_ascii_whitespace();
        let rest = lower[i + name.len()..].trim_start();
        if before_ok && rest.starts_with('=') {
            let off = tag.len() - rest.len() + 1;
            let v = tag[off..].trim_start();
            let q = v.chars().next()?;
            if q == '"' || q == '\'' {
                let body = &v[1..];
                return body.find(q).map(|e| body[..e].to_string());
            }
            return Some(
                v.split_whitespace()
                    .next()?
                    .trim_end_matches('/')
                    .to_string(),
            );
        }
        from = i + name.len();
    }
    None
}

fn decode_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_addresses_are_refused() {
        for ip in [
            "127.0.0.1",
            "10.1.2.3",
            "192.168.1.1",
            "172.16.0.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "::1",
            "fd00::1",
            "fe80::1",
            "::ffff:127.0.0.1",
        ] {
            assert!(!is_public(ip.parse().unwrap()), "{ip} must be refused");
        }
        for ip in ["1.1.1.1", "140.82.112.3", "2606:4700::1111"] {
            assert!(is_public(ip.parse().unwrap()), "{ip} is public");
        }
    }

    #[tokio::test]
    async fn localhost_urls_never_fetch() {
        for u in [
            "http://localhost:8787/api/settings",
            "http://127.0.0.1/",
            "file:///etc/passwd",
            "http://[::1]/",
        ] {
            assert!(fetch(u).await.is_err(), "{u}");
        }
    }

    #[test]
    fn open_graph_and_title_are_read() {
        let html = r#"<html><head><title>Fallback &amp; co</title>
            <meta property="og:title" content="Linear – Plan &amp; build">
            <meta name='description' content='Issue tracking'>
            <meta property="og:image" content="/og.png" />
            <meta property="og:site_name" content="Linear"></head><body>x</body></html>"#;
        let base = reqwest::Url::parse("https://linear.app/x").unwrap();
        let p = parse(html, &base);
        assert_eq!(p.title.as_deref(), Some("Linear – Plan & build"));
        assert_eq!(p.description.as_deref(), Some("Issue tracking"));
        assert_eq!(p.image.as_deref(), Some("https://linear.app/og.png"));
        assert_eq!(p.site_name.as_deref(), Some("Linear"));
        let bare = parse("<head><title> Just a title </title></head>", &base);
        assert_eq!(bare.title.as_deref(), Some("Just a title"));
    }
}
