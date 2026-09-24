//! OAuth for remote MCP servers.
//!
//! Tokens live in `~/.mira/mcp/credentials.json` (mode 0600), one entry
//! per server URL, so a server reached from several places (your config
//! and a plugin, say) shares one sign-in. rmcp does discovery, dynamic
//! client registration, PKCE and refresh; this module stores the result
//! and runs the sign-in flow.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use rmcp::transport::auth::{
    AuthError, AuthorizationManager, AuthorizationRequest, AuthorizationSession, CredentialStore,
    StoredCredentials,
};
use tokio::sync::Mutex;

use crate::spec::Transport;
use crate::state::write_private;

/// Serializes access to the credentials file across every store handle.
static FILE_LOCK: Mutex<()> = Mutex::const_new(());

/// One server URL's slot in the credentials file.
#[derive(Clone)]
pub struct FileCredentialStore {
    path: Arc<PathBuf>,
    key: String,
}

impl FileCredentialStore {
    pub fn new(path: impl Into<PathBuf>, server_url: &str) -> Self {
        Self {
            path: Arc::new(path.into()),
            key: server_url.trim_end_matches('/').to_owned(),
        }
    }

    fn read_all(path: &Path) -> BTreeMap<String, StoredCredentials> {
        std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    fn write_all(path: &Path, all: &BTreeMap<String, StoredCredentials>) -> Result<(), AuthError> {
        let json = serde_json::to_vec_pretty(all)
            .map_err(|e| AuthError::CredentialStoreError(e.to_string()))?;
        write_private(path, &json).map_err(|e| AuthError::CredentialStoreError(e.to_string()))
    }

    /// True when a token (possibly expired but refreshable) is stored.
    pub async fn has_token(&self) -> bool {
        let _g = FILE_LOCK.lock().await;
        Self::read_all(&self.path)
            .get(&self.key)
            .is_some_and(|c| c.token_response.is_some())
    }
}

#[async_trait]
impl CredentialStore for FileCredentialStore {
    async fn load(&self) -> Result<Option<StoredCredentials>, AuthError> {
        let _g = FILE_LOCK.lock().await;
        Ok(Self::read_all(&self.path).remove(&self.key))
    }

    async fn save(&self, credentials: StoredCredentials) -> Result<(), AuthError> {
        let _g = FILE_LOCK.lock().await;
        let mut all = Self::read_all(&self.path);
        all.insert(self.key.clone(), credentials);
        Self::write_all(&self.path, &all)
    }

    async fn clear(&self) -> Result<(), AuthError> {
        let _g = FILE_LOCK.lock().await;
        let mut all = Self::read_all(&self.path);
        if all.remove(&self.key).is_some() {
            Self::write_all(&self.path, &all)?;
        }
        Ok(())
    }
}

/// An authorization manager for `url` that reads and refreshes the
/// stored token, or `None` when there's no stored sign-in.
pub async fn stored_manager(
    credentials: &Path,
    url: &str,
) -> Result<Option<AuthorizationManager>, AuthError> {
    let store = FileCredentialStore::new(credentials, url);
    if !store.has_token().await {
        return Ok(None);
    }
    let mut manager = AuthorizationManager::new(url).await?;
    manager.set_credential_store(store);
    if manager.initialize_from_store().await? {
        Ok(Some(manager))
    } else {
        Ok(None)
    }
}

/// A sign-in in progress: the browser URL to open, and the session that
/// finishes it when the redirect comes back.
pub struct PendingSignIn {
    pub server: String,
    pub auth_url: String,
    /// The `state` parameter, which identifies this sign-in on callback.
    pub state: String,
    pub session: AuthorizationSession,
}

/// Start signing in to a remote server: discover its authorization
/// server (from the 401 challenge when we have one), register a client
/// unless one is configured, and build the authorization URL.
pub async fn begin(
    credentials: &Path,
    server: &str,
    transport: &Transport,
    challenge: Option<&str>,
    redirect_uri: &str,
) -> Result<PendingSignIn, AuthError> {
    let (url, oauth) = match transport {
        Transport::Http { url, oauth, .. } | Transport::Sse { url, oauth, .. } => (url, oauth),
        Transport::Stdio { .. } => {
            return Err(AuthError::AuthorizationFailed(
                "stdio servers don't use OAuth".into(),
            ))
        }
    };
    let mut manager = AuthorizationManager::new(url.as_str()).await?;
    manager.set_credential_store(FileCredentialStore::new(credentials, url));
    let resolution = manager.resolve_metadata_from_challenge(challenge).await?;
    manager.set_metadata(resolution.metadata);

    let mut request = AuthorizationRequest::new(redirect_uri).with_client_name("Mira");
    if let Some(challenge) = challenge {
        request = request.with_challenge(challenge);
    }
    if let Some(o) = oauth {
        if let Some(id) = &o.client_id {
            request = request.with_preregistered_client(id);
            if let Some(secret) = &o.client_secret {
                request = request.with_client_secret(secret);
            }
        }
        if !o.scopes.is_empty() {
            request = request.with_scopes(o.scopes.iter().map(String::as_str));
        }
    }
    let session = AuthorizationSession::new(manager, request)
        .await
        .map_err(|(_, e)| e)?;
    let auth_url = session.get_authorization_url().to_owned();
    let state = url::Url::parse(&auth_url)
        .ok()
        .and_then(|u| {
            u.query_pairs()
                .find(|(k, _)| k == "state")
                .map(|(_, v)| v.into_owned())
        })
        .unwrap_or_default();
    Ok(PendingSignIn {
        server: server.to_owned(),
        auth_url,
        state,
        session,
    })
}

/// Forget the stored sign-in for a server URL.
pub async fn sign_out(credentials: &Path, url: &str) -> Result<(), AuthError> {
    FileCredentialStore::new(credentials, url).clear().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn store_keeps_servers_apart_and_private() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("credentials.json");
        let a = FileCredentialStore::new(&path, "https://a.dev/mcp/");
        let b = FileCredentialStore::new(&path, "https://b.dev/mcp");
        a.save(StoredCredentials::new("client-a".into(), None, vec![], None))
            .await
            .unwrap();
        b.save(StoredCredentials::new("client-b".into(), None, vec![], None))
            .await
            .unwrap();
        // Trailing slash doesn't matter.
        let a2 = FileCredentialStore::new(&path, "https://a.dev/mcp");
        assert_eq!(a2.load().await.unwrap().unwrap().client_id, "client-a");
        a.clear().await.unwrap();
        assert!(a2.load().await.unwrap().is_none());
        assert_eq!(b.load().await.unwrap().unwrap().client_id, "client-b");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }
}
