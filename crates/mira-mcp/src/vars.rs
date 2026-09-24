//! Values for `${VAR}`s in server definitions, saved from the UI or
//! `mira mcp set-var`, so a server that needs a token (a plugin's
//! `${GITHUB_PERSONAL_ACCESS_TOKEN}`, say) works without exporting it in
//! your shell. Kept next to the OAuth credentials, mode 0600.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Result};

use crate::state::write_private;

/// `~/.mira/mcp/variables.json`, beside `credentials.json`.
pub fn path_for(credentials: &Path) -> PathBuf {
    credentials.with_file_name("variables.json")
}

pub fn load(path: &Path) -> BTreeMap<String, String> {
    std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// Set a variable, or with `None` (or an empty value) remove it.
pub fn set(path: &Path, name: &str, value: Option<&str>) -> Result<()> {
    if !valid_name(name) {
        bail!("`{name}` isn't a variable name (letters, digits and `_`)");
    }
    let mut all = load(path);
    match value.map(str::trim).filter(|v| !v.is_empty()) {
        Some(v) => {
            all.insert(name.to_owned(), v.to_owned());
        }
        None => {
            all.remove(name);
        }
    }
    write_private(path, &serde_json::to_vec_pretty(&all)?)
}

pub fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_and_remove() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("variables.json");
        set(&p, "GITHUB_TOKEN", Some(" ghp_x ")).unwrap();
        assert_eq!(load(&p)["GITHUB_TOKEN"], "ghp_x");
        set(&p, "GITHUB_TOKEN", Some("")).unwrap();
        assert!(load(&p).is_empty());
        assert!(set(&p, "not ok", Some("x")).is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&p).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }
}
