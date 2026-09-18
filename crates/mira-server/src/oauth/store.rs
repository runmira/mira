//! Persistent OAuth token store — `~/.mira/auth/<provider>.json`.
//!
//! Providers whose access is granted by OAuth (rather than a
//! long-lived user-pasted API key) also need to remember:
//!
//!   - the current short-lived API key,
//!   - when it expires,
//!   - the long-lived refresh token used to mint a new one.
//!
//! Yaml is the wrong home for that — it's user-edited and the refresh
//! token needs to survive between sessions without leaking into config
//! diffs. This module owns a per-provider JSON file, mode 0600 on Unix
//! (owner-only), containing everything the refresh task needs to keep
//! the running provider healthy without prompting the user again.
//!
//! The API key ALSO lives in yaml so the standard provider-build path
//! (`ProviderConfig::resolved_api_key`) picks it up without special
//! casing. The store is the source of truth for refresh; yaml holds a
//! mirror of the current key value that the refresh loop keeps in sync.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenBundle {
    /// The short-lived OpenAI API key returned by the token-exchange
    /// step. This is what actual API calls authenticate with.
    pub api_key: String,
    /// Long-lived refresh token used to mint a new access + api-key
    /// pair when the current one is close to expiry.
    pub refresh_token: String,
    /// The access_token from the OAuth flow. Not used for API calls
    /// (the api_key is), but kept around because the token-exchange
    /// step needs a fresh access_token, and refresh returns one.
    pub access_token: String,
    /// The id_token from the OAuth flow. Carries the ChatGPT account
    /// id used for workspace validation on refresh.
    pub id_token: String,
    /// Unix seconds when `api_key` / `access_token` stop working.
    /// Refresh should fire at least a few minutes before this.
    pub expires_at: u64,
    /// Unix seconds this bundle was written. Diagnostic only.
    pub obtained_at: u64,
}

impl TokenBundle {
    pub fn is_near_expiry(&self, buffer_secs: u64) -> bool {
        let now = now_secs();
        self.expires_at <= now + buffer_secs
    }
}

/// Absolute path to the per-provider token file, rooted at the given
/// home directory. Split from [`token_path`] so tests can point at a
/// sandbox root without mutating the global `$HOME` (which races with
/// every other test that also touches `$HOME`).
pub fn token_path_at(home: &std::path::Path, provider: &str) -> PathBuf {
    home.join(".mira")
        .join("auth")
        .join(format!("{provider}.json"))
}

/// Absolute path to the per-provider token file under the real user
/// `$HOME`. Production callers use this; tests use [`token_path_at`]
/// via [`load_at`] / [`save_at`] to stay hermetic.
pub fn token_path(provider: &str) -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    token_path_at(&home, provider)
}

/// Read the current bundle for `provider`. `Ok(None)` when the file
/// doesn't exist yet — that's the "user hasn't signed in" case, not an
/// error. Any other IO / parse failure surfaces as `Err`.
pub fn load(provider: &str) -> Result<Option<TokenBundle>> {
    load_at(&token_path(provider))
}

fn load_at(path: &std::path::Path) -> Result<Option<TokenBundle>> {
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let bundle: TokenBundle =
        serde_json::from_str(&raw).with_context(|| format!("parse {}", path.display()))?;
    Ok(Some(bundle))
}

/// Atomically save `bundle` for `provider`. Writes to a sibling
/// `.tmp` file first, then renames — a crash mid-write can't leave a
/// half-written token file that the next boot would silently accept.
/// Sets mode 0600 on Unix so the file is only readable by its owner.
pub fn save(provider: &str, bundle: &TokenBundle) -> Result<()> {
    save_at(&token_path(provider), bundle)
}

fn save_at(path: &std::path::Path, bundle: &TokenBundle) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("token path has no parent"))?;
    fs::create_dir_all(parent).with_context(|| format!("mkdir {}", parent.display()))?;

    let tmp = parent.join(format!(
        "{}.tmp",
        path.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("provider.json")
    ));
    let json = serde_json::to_string_pretty(bundle)?;
    {
        let mut f = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&tmp)
            .with_context(|| format!("open {}", tmp.display()))?;
        f.write_all(json.as_bytes())
            .with_context(|| format!("write {}", tmp.display()))?;
        f.sync_all().ok();
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(0o600);
        fs::set_permissions(&tmp, perms).ok();
    }

    fs::rename(&tmp, &path)
        .with_context(|| format!("rename {} -> {}", tmp.display(), path.display()))?;
    Ok(())
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn near_expiry_thresholds() {
        let b = TokenBundle {
            api_key: "k".into(),
            refresh_token: "r".into(),
            access_token: "a".into(),
            id_token: "i".into(),
            expires_at: now_secs() + 100,
            obtained_at: now_secs(),
        };
        assert!(b.is_near_expiry(200), "should fire when buffer > remaining");
        assert!(
            !b.is_near_expiry(50),
            "should stay quiet when buffer < remaining"
        );
    }

    #[test]
    fn roundtrip_atomic_write() {
        // Hermetic — pass an explicit root instead of mutating $HOME
        // (which would race with every other test in the workspace
        // that reads home / cache paths).
        let temp = tempfile::tempdir().unwrap();
        let path = token_path_at(temp.path(), "openai_test");

        let bundle = TokenBundle {
            api_key: "sk-test".into(),
            refresh_token: "rt".into(),
            access_token: "at".into(),
            id_token: "id".into(),
            expires_at: 12345,
            obtained_at: 999,
        };
        save_at(&path, &bundle).unwrap();
        let loaded = load_at(&path).unwrap().unwrap();
        assert_eq!(loaded.api_key, "sk-test");
        assert_eq!(loaded.refresh_token, "rt");
        assert_eq!(loaded.expires_at, 12345);
    }

    #[test]
    fn load_returns_none_when_missing() {
        let temp = tempfile::tempdir().unwrap();
        let path = token_path_at(temp.path(), "nope_provider");
        assert!(load_at(&path).unwrap().is_none());
    }
}
