//! Computer-only credentials: separate from full Mira device access.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::Mutex,
    time::{Duration, Instant},
};
#[derive(Clone, Serialize, Deserialize)]
struct Grant {
    id: String,
    name: String,
    hash: String,
    created_at: i64,
    #[serde(default = "cua_backend")]
    backend: String,
}
fn cua_backend() -> String {
    "cua".into()
}
#[derive(Clone, Serialize)]
pub struct GrantView {
    pub id: String,
    pub name: String,
    pub created_at: i64,
    pub backend: String,
}
struct Code {
    value: String,
    until: Instant,
    attempts: usize,
    backend: String,
}
#[derive(Default)]
struct Inner {
    grants: Vec<Grant>,
    code: Option<Code>,
    failures: VecDeque<Instant>,
}
pub struct Grants {
    path: Option<PathBuf>,
    inner: Mutex<Inner>,
}
fn hash(value: &str) -> String {
    Sha256::digest(value.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn equals(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0, |d, (a, b)| d | (a ^ b)) == 0
}
fn view(g: &Grant) -> GrantView {
    GrantView {
        id: g.id.clone(),
        name: g.name.clone(),
        created_at: g.created_at,
        backend: g.backend.clone(),
    }
}
impl Grants {
    pub fn load(path: PathBuf) -> Result<Self, String> {
        let grants = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|_| "Computer access records could not be read")?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => vec![],
            Err(_) => return Err("Computer access records are not accessible".into()),
        };
        Ok(Self {
            path: Some(path),
            inner: Mutex::new(Inner {
                grants,
                ..Default::default()
            }),
        })
    }
    pub fn ephemeral() -> Self {
        Self {
            path: None,
            inner: Mutex::new(Inner::default()),
        }
    }
    fn save(&self, grants: &[Grant]) -> Result<(), String> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|_| "Could not create computer access records")?;
        }
        mira_config::write_private(
            path,
            &serde_json::to_vec(grants).map_err(|_| "Could not encode computer access records")?,
        )
        .map_err(|_| "Could not save computer access records".into())
    }
    #[cfg(test)]
    pub fn open(&self) -> (String, i64) {
        self.open_for("cua")
    }
    pub fn open_for(&self, backend: &str) -> (String, i64) {
        let value = uuid::Uuid::new_v4().simple().to_string()[..10].to_ascii_uppercase();
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).code = Some(Code {
            value: value.clone(),
            until: Instant::now() + Duration::from_secs(600),
            attempts: 0,
            backend: backend.into(),
        });
        (value, chrono::Utc::now().timestamp() + 600)
    }
    pub fn open_backend(&self) -> Option<String> {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .code
            .as_ref()
            .filter(|c| Instant::now() < c.until)
            .map(|c| c.backend.clone())
    }
    pub fn is_open(&self) -> bool {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .code
            .as_ref()
            .is_some_and(|c| Instant::now() < c.until)
    }
    pub fn close(&self, code: &str) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if inner
            .code
            .as_ref()
            .is_some_and(|open| equals(open.value.as_bytes(), code.as_bytes()))
        {
            inner.code = None;
        }
    }
    pub fn claim(&self, value: &str, name: &str) -> Result<(GrantView, String), String> {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        while inner
            .failures
            .front()
            .is_some_and(|t| now.duration_since(*t) > Duration::from_secs(600))
        {
            inner.failures.pop_front();
        }
        if inner.failures.len() >= 10 {
            return Err("Too many pairing attempts. Try again in 10 minutes.".into());
        }
        let code = inner
            .code
            .as_mut()
            .ok_or("That pairing link has expired or was already used.")?;
        if now >= code.until {
            inner.code = None;
            return Err("That pairing link has expired.".into());
        }
        if !equals(
            value.trim().to_ascii_uppercase().as_bytes(),
            code.value.as_bytes(),
        ) {
            code.attempts += 1;
            if code.attempts >= 5 {
                inner.code = None;
            }
            inner.failures.push_back(now);
            return Err("That pairing code is not correct.".into());
        }
        let backend = code.backend.clone();
        if inner.grants.len() >= 32 {
            return Err("Remove an existing computer before pairing another.".into());
        }
        let token = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let grant = Grant {
            id: uuid::Uuid::new_v4().simple().to_string(),
            name: name.trim().chars().take(60).collect(),
            hash: hash(&token),
            created_at: chrono::Utc::now().timestamp(),
            backend,
        };
        let mut next = inner.grants.clone();
        next.push(grant.clone());
        self.save(&next)?;
        inner.grants = next;
        inner.code = None;
        Ok((view(&grant), token))
    }
    pub fn authenticate(&self, token: &str) -> Option<GrantView> {
        if token.len() != 64 {
            return None;
        }
        let digest = hash(token);
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .grants
            .iter()
            .find(|g| equals(digest.as_bytes(), g.hash.as_bytes()))
            .map(view)
    }
    pub fn list(&self) -> Vec<GrantView> {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .grants
            .iter()
            .map(view)
            .collect()
    }
    pub fn revoke(&self, id: &str) -> Result<(), String> {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let next = inner
            .grants
            .iter()
            .filter(|g| g.id != id)
            .cloned()
            .collect::<Vec<_>>();
        self.save(&next)?;
        inner.grants = next;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn codes_are_single_use_and_credentials_are_scoped_and_revocable() {
        let g = Grants::ephemeral();
        let (code, _) = g.open();
        let (device, token) = g.claim(&code, "Laptop").unwrap();
        assert!(g.claim(&code, "Other").is_err());
        assert!(g.authenticate(&token).is_some());
        assert!(g.authenticate("invalid").is_none());
        g.revoke(&device.id).unwrap();
        assert!(g.authenticate(&token).is_none());
    }
    #[test]
    fn wrong_guesses_close_the_code() {
        let g = Grants::ephemeral();
        let (code, _) = g.open();
        for _ in 0..5 {
            assert!(g.claim("wrong", "attacker").is_err());
        }
        assert!(g.claim(&code, "Laptop").is_err());
    }
    #[test]
    fn persistence_failure_cannot_issue_or_revoke_a_credential() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("records.json");
        let g = Grants::load(path.clone()).unwrap();
        let (code, _) = g.open();
        let (device, token) = g.claim(&code, "Laptop").unwrap();
        assert!(Grants::load(path.clone())
            .unwrap()
            .authenticate(&token)
            .is_some());
        let stored = std::fs::read_to_string(&path).unwrap();
        assert!(!stored.contains(&token));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(g.revoke(&device.id).is_err());
        assert!(g.authenticate(&token).is_some());
        let (second, _) = g.open();
        assert!(g.claim(&second, "Other").is_err());
        assert_eq!(g.list().len(), 1);
    }
    #[test]
    fn closing_an_old_sheet_does_not_cancel_a_new_code_and_backends_remain_bound() {
        let g = Grants::ephemeral();
        let (old, _) = g.open_for("native");
        let (new, _) = g.open_for("cua");
        g.close(&old);
        let (view, _) = g.claim(&new, "Laptop").unwrap();
        assert_eq!(view.backend, "cua");
    }
    #[test]
    fn expired_codes_cannot_grant_access() {
        let g = Grants::ephemeral();
        let (code, _) = g.open();
        g.inner.lock().unwrap().code.as_mut().unwrap().until =
            Instant::now() - Duration::from_secs(1);
        assert!(g.claim(&code, "Laptop").is_err());
        assert!(g.list().is_empty());
    }
}
