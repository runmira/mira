//! Read the active github.com CLI login privately; never export or log its token.
use std::path::Path;

pub(crate) async fn cli_token() -> Option<String> {
    let program = mira_acp::which::resolve_for_spawn(Path::new("gh"));
    cli_token_from(&program).await
}

/// The `gh` login, asked for at most every few minutes: spawning `gh` costs
/// more than many GitHub requests do.
pub(crate) async fn cached_cli_token() -> Option<String> {
    const FRESH: std::time::Duration = std::time::Duration::from_secs(300);
    if let Some((at, token)) = cli_cache()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
    {
        if at.elapsed() < FRESH {
            return token;
        }
    }
    let token = cli_token().await;
    *cli_cache().lock().unwrap_or_else(|e| e.into_inner()) =
        Some((std::time::Instant::now(), token.clone()));
    token
}

/// GitHub rejected the cached login too: ask `gh` again next time.
pub(crate) fn forget_cli_token() {
    *cli_cache().lock().unwrap_or_else(|e| e.into_inner()) = None;
}

type CliCache = std::sync::Mutex<Option<(std::time::Instant, Option<String>)>>;

fn cli_cache() -> &'static CliCache {
    static CACHE: std::sync::OnceLock<CliCache> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(None))
}

/// Keys GitHub answered 401 to during this run, by hash. A key saved in
/// Settings then stops costing a failed request before every fallback;
/// saving a new key changes the hash, so it's tried again.
pub(crate) fn mark_rejected(token: &str) {
    rejected()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(token_hash(token));
}

/// Keys GitHub accepted when probed, so the status check asks once.
pub(crate) fn mark_accepted(token: &str) {
    accepted()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(token_hash(token));
}

pub(crate) fn is_accepted(token: &str) -> bool {
    accepted()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains(&token_hash(token))
}

fn accepted() -> &'static std::sync::Mutex<std::collections::HashSet<u64>> {
    static ACCEPTED: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<u64>>> =
        std::sync::OnceLock::new();
    ACCEPTED.get_or_init(Default::default)
}

/// A short, stable name for a key that isn't the key: the UI remembers
/// which one it was told about without ever holding it.
pub(crate) fn fingerprint(token: &str) -> String {
    format!("{:016x}", token_hash(token))
}

pub(crate) fn is_rejected(token: &str) -> bool {
    rejected()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains(&token_hash(token))
}

fn rejected() -> &'static std::sync::Mutex<std::collections::HashSet<u64>> {
    static REJECTED: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<u64>>> =
        std::sync::OnceLock::new();
    REJECTED.get_or_init(Default::default)
}

fn token_hash(token: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    token.hash(&mut hasher);
    hasher.finish()
}

async fn cli_token_from(program: &Path) -> Option<String> {
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        tokio::process::Command::new(program)
            .args(["auth", "token", "--hostname", "github.com"])
            // A key exported from Mira's settings must not mask the saved CLI
            // login when recovering from that very key's rejection.
            .env_remove("GH_TOKEN")
            .env_remove("GITHUB_TOKEN")
            .env_remove("GH_ENTERPRISE_TOKEN")
            .env_remove("GITHUB_ENTERPRISE_TOKEN")
            .env("GH_PROMPT_DISABLED", "1")
            .kill_on_drop(true)
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    if !output.status.success() {
        return None;
    }
    let token = String::from_utf8(output.stdout).ok()?.trim().to_string();
    (!token.is_empty() && !token.chars().any(char::is_whitespace)).then_some(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rejected_key_is_remembered_by_value_not_kept() {
        assert!(!is_rejected("key-a"));
        mark_rejected("key-a");
        assert!(is_rejected("key-a"));
        assert!(
            !is_rejected("key-b"),
            "a newly saved key gets its own chance"
        );
    }
    #[tokio::test]
    async fn cli_login_is_private_noninteractive_and_host_bound() {
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("gh");
        std::fs::write(
            &program,
            r#"#!/bin/sh
[ "$1 $2 $3 $4" = 'auth token --hostname github.com' ] || exit 1
[ -z "$GITHUB_TOKEN$GH_TOKEN$GH_ENTERPRISE_TOKEN$GITHUB_ENTERPRISE_TOKEN" ] || exit 2
[ "$GH_PROMPT_DISABLED" = 1 ] || exit 3
printf 'fake-saved-login\n'
"#,
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        assert_eq!(
            cli_token_from(&program).await.as_deref(),
            Some("fake-saved-login")
        );
        std::fs::write(&program, "#!/bin/sh\nprintf 'sign in required'\nexit 1\n").unwrap();
        assert!(cli_token_from(&program).await.is_none());
    }
}
