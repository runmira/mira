//! An account-only Codex app-server connection. It never opens a thread or
//! logs authentication payloads. The CLI owns credentials and callback URLs.
use crate::{
    conn::{AgentCallback, ConnError},
    driver::LaunchConfig,
    process::AgentProcess,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;
use tokio::sync::{mpsc, watch, Mutex};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoginPhase {
    #[default]
    SignedOut,
    Pending,
    Authenticated,
    Failed,
    Cancelled,
    Unsupported,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AccountIdentity {
    pub kind: String,
    pub email: Option<String>,
    pub plan: Option<String>,
}

#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AccountView {
    pub connection_id: String,
    pub phase: LoginPhase,
    pub account: Option<AccountIdentity>,
    pub requires_openai_auth: Option<bool>,
    pub requirements: Option<Value>,
    pub requirements_supported: bool,
    pub login_id: Option<String>,
    pub auth_url: Option<String>,
    pub user_code: Option<String>,
    pub error: Option<String>,
    pub revision: u64,
}

enum Notice {
    Updated,
    Completed {
        id: Option<String>,
        success: bool,
        error: Option<String>,
    },
    Closed,
}
struct Callbacks(mpsc::Sender<Notice>);
#[async_trait::async_trait]
impl AgentCallback for Callbacks {
    async fn on_request(&self, _: &Value, method: &str, _: Value) -> Result<Value, ConnError> {
        Err(ConnError::Unhandled {
            method: method.into(),
        })
    }
    async fn on_notification(&self, method: &str, params: Value) {
        let notice = match method {
            "account/updated" => Notice::Updated,
            "account/login/completed" => Notice::Completed {
                id: params
                    .get("loginId")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                success: params.get("success").and_then(Value::as_bool) == Some(true),
                error: params
                    .get("error")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            },
            _ => return,
        };
        let _ = self.0.send(notice).await;
    }
    async fn on_closed(&self, _: String) {
        let _ = self.0.send(Notice::Closed).await;
    }
}

pub struct AccountClient {
    process: AgentProcess,
    gate: Mutex<()>,
    view: watch::Sender<AccountView>,
    retired: AtomicBool,
    touched: watch::Sender<()>,
}

fn unavailable(error: &str) -> bool {
    error.contains("-32601")
        || error.contains("unknown variant")
        || error.contains("Unknown variant")
}
fn clear_handoff(view: &mut AccountView) {
    view.login_id = None;
    view.auth_url = None;
    view.user_code = None;
}

impl AccountClient {
    pub async fn start(launch: &LaunchConfig) -> Result<Arc<Self>, String> {
        let (tx, mut rx) = mpsc::channel(32);
        let process = AgentProcess::spawn(launch, Arc::new(Callbacks(tx)))
            .await
            .map_err(|e| e.to_string())?;
        let (view, _) = watch::channel(AccountView {
            connection_id: uuid::Uuid::new_v4().to_string(),
            ..Default::default()
        });
        let (touched, mut touches) = watch::channel(());
        let client = Arc::new(Self {
            process,
            gate: Mutex::new(()),
            view,
            retired: AtomicBool::new(false),
            touched,
        });
        client
            .call(
                "initialize",
                crate::appserver::initialize_params("mira-account"),
            )
            .await?;
        let _ = client.process.conn().notify("initialized", json!({})).await;
        client.refresh().await;
        let weak = Arc::downgrade(&client);
        tokio::spawn(async move {
            // Auth connections are bounded even if the settings page disappears.
            let expiry = tokio::time::sleep(Duration::from_secs(600));
            tokio::pin!(expiry);
            loop {
                let notice = tokio::select! {
                    biased;
                    changed = touches.changed() => {
                        if changed.is_err() {break;}
                        expiry.as_mut().reset(tokio::time::Instant::now() + Duration::from_secs(600));
                        continue;
                    }
                    notice = rx.recv() => match notice { Some(n) => n, None => break },
                    _ = &mut expiry => {
                        if let Some(client) = weak.upgrade() {
                            if client.snapshot().phase == LoginPhase::Pending { client.fail("The sign-in connection expired. Start sign-in again."); }
                            client.shutdown().await;
                        }
                        break;
                    }
                };
                let Some(client) = weak.upgrade() else { break };
                let _gate = client.gate.lock().await;
                if client.retired.load(Ordering::SeqCst) {
                    break;
                }
                match notice {
                    Notice::Updated => client.refresh_locked(false).await,
                    Notice::Completed { id, success, error } => {
                        // Completion is not identity or authorization: match the
                        // active attempt, then reread account and requirements.
                        let pending = client.snapshot();
                        if pending.phase != LoginPhase::Pending || id != pending.login_id {
                            continue;
                        }
                        if success {
                            client.refresh_locked(true).await;
                            if client.snapshot().phase == LoginPhase::SignedOut {
                                client.fail(
                                    "Sign-in completed, but Codex has no authenticated account.",
                                );
                            }
                        } else {
                            client.fail(error.as_deref().unwrap_or("Codex sign-in failed."));
                        }
                    }
                    Notice::Closed => {
                        client.fail("The Codex account connection closed. Try again.");
                        client.retired.store(true, Ordering::SeqCst);
                        break;
                    }
                }
            }
        });
        Ok(client)
    }
    async fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        tokio::time::timeout(
            Duration::from_secs(15),
            self.process.conn().request::<Value>(method, params),
        )
        .await
        .map_err(|_| format!("{method} timed out"))?
        .map_err(|e| e.to_string())
    }
    pub fn snapshot(&self) -> AccountView {
        self.view.borrow().clone()
    }
    pub fn subscribe(&self) -> watch::Receiver<AccountView> {
        self.view.subscribe()
    }
    pub fn retired(&self) -> bool {
        self.retired.load(Ordering::SeqCst)
    }
    fn fail(&self, message: &str) {
        self.view.send_modify(|v| {
            clear_handoff(v);
            v.phase = LoginPhase::Failed;
            v.error = Some(message.into());
            v.revision += 1;
        });
    }
    pub async fn refresh(&self) {
        let _gate = self.gate.lock().await;
        if !self.retired() {
            self.refresh_locked(false).await;
        }
    }
    async fn refresh_locked(&self, finish_login: bool) {
        let result = match self
            .call("account/read", json!({"refreshToken":false}))
            .await
        {
            Ok(result) => result,
            Err(error) => {
                self.fail(&error);
                if unavailable(&error) {
                    self.view.send_modify(|v| v.phase = LoginPhase::Unsupported);
                }
                return;
            }
        };
        let requirements = self.call("configRequirements/read", json!({})).await;
        if let Err(error) = &requirements {
            if !unavailable(error) {
                self.fail("Codex account requirements could not be refreshed. Try checking again.");
                return;
            }
        }
        let account = result
            .get("account")
            .filter(|a| a.is_object())
            .map(|a| AccountIdentity {
                kind: a
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .into(),
                email: a.get("email").and_then(Value::as_str).map(str::to_owned),
                plan: a.get("planType").and_then(Value::as_str).map(str::to_owned),
            });
        self.view.send_modify(|v| {
            v.account = account;
            v.requires_openai_auth = result.get("requiresOpenaiAuth").and_then(Value::as_bool);
            v.requirements_supported = requirements.is_ok();
            v.requirements = requirements
                .ok()
                .and_then(|r| r.get("requirements").cloned())
                .filter(|r| !r.is_null());
            // Account updates during a login may still describe the old saved
            // account. Only the matching completion ends a pending attempt.
            if finish_login || v.phase != LoginPhase::Pending {
                if finish_login {
                    clear_handoff(v);
                }
                v.phase = if v.account.is_some() || v.requires_openai_auth == Some(false) {
                    LoginPhase::Authenticated
                } else {
                    LoginPhase::SignedOut
                };
                v.error = None;
            }
            v.revision += 1;
        });
    }
    pub async fn begin(&self, flow: &str) -> Result<AccountView, String> {
        let kind = match flow {
            "browser" => "chatgpt",
            "device" => "chatgptDeviceCode",
            _ => return Err("Unknown sign-in flow".into()),
        };
        let _gate = self.gate.lock().await;
        if self.retired() {
            return Err("The sign-in connection has closed. Start again.".into());
        }
        if self.snapshot().phase == LoginPhase::Pending {
            return Err("A sign-in is already pending. Cancel it before starting another.".into());
        }
        self.touched.send_replace(());
        let result = match self.call("account/login/start", json!({"type":kind})).await {
            Ok(result) => result,
            Err(error) => {
                self.fail(&error);
                if unavailable(&error) {
                    self.view.send_modify(|v| v.phase = LoginPhase::Unsupported);
                }
                return Ok(self.snapshot());
            }
        };
        let handoff = (|| -> Result<(&str, &str), String> {
            let id = result
                .get("loginId")
                .and_then(Value::as_str)
                .filter(|v| !v.is_empty())
                .ok_or("Codex did not return a sign-in identifier")?;
            let url = result
                .get("authUrl")
                .or_else(|| result.get("verificationUrl"))
                .and_then(Value::as_str)
                .ok_or("Codex did not return a sign-in URL")?;
            let parsed =
                url::Url::parse(url).map_err(|_| "Codex returned an invalid sign-in URL")?;
            if parsed.scheme() != "https"
                || !parsed.host_str().is_some_and(|host| {
                    host == "openai.com"
                        || host.ends_with(".openai.com")
                        || host == "chatgpt.com"
                        || host.ends_with(".chatgpt.com")
                })
                || !parsed.username().is_empty()
                || parsed.password().is_some()
            {
                return Err("Codex returned an unsafe sign-in URL".into());
            }
            if flow == "device"
                && !result
                    .get("userCode")
                    .and_then(Value::as_str)
                    .is_some_and(|v| !v.is_empty())
            {
                return Err("Codex did not return a device code".into());
            }
            Ok((id, url))
        })();
        let (id, url) = match handoff {
            Ok(handoff) => handoff,
            Err(error) => {
                self.fail(&error);
                self.shutdown().await;
                return Ok(self.snapshot());
            }
        };
        self.view.send_modify(|v| {
            v.phase = LoginPhase::Pending;
            v.error = None;
            v.login_id = Some(id.into());
            v.auth_url = Some(url.into());
            v.user_code = result
                .get("userCode")
                .and_then(Value::as_str)
                .map(str::to_owned);
            v.revision += 1;
        });
        Ok(self.snapshot())
    }
    pub async fn cancel(&self) -> AccountView {
        let _gate = self.gate.lock().await;
        self.cancel_locked().await
    }
    pub async fn cancel_attempt(
        &self,
        connection_id: &str,
        login_id: &str,
    ) -> Result<AccountView, String> {
        let _gate = self.gate.lock().await;
        let view = self.snapshot();
        if view.phase != LoginPhase::Pending
            || view.connection_id != connection_id
            || view.login_id.as_deref() != Some(login_id)
        {
            return Err(
                "This sign-in was replaced or already finished. Check the current account.".into(),
            );
        }
        Ok(self.cancel_locked().await)
    }
    async fn cancel_locked(&self) -> AccountView {
        let id = self.snapshot().login_id;
        self.retired.store(true, Ordering::SeqCst);
        self.view.send_modify(|v| {
            clear_handoff(v);
            v.phase = LoginPhase::Cancelled;
            v.error = None;
            v.revision += 1;
        });
        if let Some(id) = id {
            let _ = self
                .call("account/login/cancel", json!({"loginId":id}))
                .await;
        }
        // Also close the callback listener on older CLIs or failed cancellation.
        self.process.shutdown(Duration::from_millis(500)).await;
        self.snapshot()
    }
    pub async fn shutdown(&self) {
        self.retired.store(true, Ordering::SeqCst);
        self.view.send_modify(|v| v.revision += 1);
        self.process.shutdown(Duration::from_millis(500)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fake(mode: &str) -> (LaunchConfig, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("codex");
        std::fs::write(&program,r#"#!/bin/sh
signed=0
email=first@example.test
provider=openai
[ "$FAKE_MODE" = saved ] && signed=1
id_of() { printf '%s' "$1" | sed -E 's/^\{"jsonrpc":"2\.0","id":([0-9]+),.*/\1/'; }
while IFS= read -r line; do
  id=$(id_of "$line")
  case "$line" in
    *'"method":"initialize"'*) echo "{\"id\":$id,\"result\":{}}" ;;
    *'"method":"account/read"'*)
      if [ "$signed" = 1 ]; then
        echo "{\"id\":$id,\"result\":{\"account\":{\"type\":\"chatgpt\",\"email\":\"$email\",\"planType\":\"plus\"},\"requiresOpenaiAuth\":true}}"
      else echo "{\"id\":$id,\"result\":{\"account\":null,\"requiresOpenaiAuth\":true}}"; fi ;;
    *'"method":"configRequirements/read"'*)
      if [ "$FAKE_MODE" = legacy ]; then echo "{\"id\":$id,\"error\":{\"code\":-32601,\"message\":\"Method not found\"}}"
      else echo "{\"id\":$id,\"result\":{\"requirements\":{\"allowedModelProviders\":[\"$provider\"]}}}"; fi ;;
    *'"method":"account/login/start"'*)
      if [ "$FAKE_MODE" = legacy ] || { [ "$FAKE_MODE" = device-only ] && echo "$line" | grep -q '"type":"chatgpt"'; }; then
        echo "{\"id\":$id,\"error\":{\"code\":-32601,\"message\":\"Method not found\"}}"; continue
      fi
      if [ "$FAKE_MODE" = complete ]; then
        signed=1
        echo '{"method":"account/login/completed","params":{"loginId":"login-1","success":true}}'
        echo '{"method":"account/updated","params":{}}'
      fi
      if [ "$FAKE_MODE" = unsafe ]; then url=https://example.test/login; else url=https://auth.openai.com/codex/device; fi
      echo "{\"id\":$id,\"result\":{\"type\":\"chatgptDeviceCode\",\"loginId\":\"login-1\",\"authUrl\":\"$url\",\"verificationUrl\":\"$url\",\"userCode\":\"ABCD-EFGH\"}}"
      echo '{"method":"account/login/completed","params":{"loginId":"other-owner","success":true}}'
      if [ "$FAKE_MODE" = failed ]; then echo '{"method":"account/login/completed","params":{"loginId":"login-1","success":false,"error":"Denied"}}'; fi ;;
    *'"method":"account/login/cancel"'*)
      echo "{\"id\":$id,\"result\":{\"status\":\"canceled\"}}"
      echo '{"method":"account/login/completed","params":{"loginId":"login-1","success":true}}' ;;
    *'"method":"fixture/switch"'*)
      signed=1; email=second@example.test; provider=bedrock
      echo "{\"id\":$id,\"result\":{}}"
      echo '{"method":"account/updated","params":{"authMode":"chatgpt"}}' ;;
  esac
done
"#).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let mut launch = crate::appserver::launch(program);
        launch.env.insert("FAKE_MODE".into(), mode.into());
        (launch, dir)
    }
    async fn wait_for(client: &AccountClient, phase: LoginPhase) {
        let mut events = client.subscribe();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if client.snapshot().phase == phase {
                    break;
                }
                events.changed().await.unwrap();
            }
        })
        .await
        .expect("account phase");
    }
    #[tokio::test]
    async fn completion_before_start_response_is_matched_and_refreshes_identity() {
        let (launch, _dir) = fake("complete");
        let client = AccountClient::start(&launch).await.unwrap();
        assert_eq!(client.snapshot().phase, LoginPhase::SignedOut);
        assert_eq!(
            client.begin("browser").await.unwrap().phase,
            LoginPhase::Pending
        );
        wait_for(&client, LoginPhase::Authenticated).await;
        let view = client.snapshot();
        assert_eq!(
            view.account.unwrap().email.as_deref(),
            Some("first@example.test")
        );
        assert!(view.auth_url.is_none() && view.login_id.is_none() && view.user_code.is_none());
        assert!(view.requirements_supported);
        client.shutdown().await;
    }
    #[tokio::test]
    async fn pending_state_survives_reconnect_and_late_cancel_completion_is_ignored() {
        let (launch, _dir) = fake("pending");
        let client = AccountClient::start(&launch).await.unwrap();
        client.begin("device").await.unwrap();
        let reconnect = client.subscribe().borrow().clone();
        assert_eq!(reconnect.phase, LoginPhase::Pending);
        assert_eq!(reconnect.user_code.as_deref(), Some("ABCD-EFGH"));
        assert!(client.begin("browser").await.is_err());
        let view = client.cancel().await;
        assert_eq!(view.phase, LoginPhase::Cancelled);
        assert!(view.login_id.is_none() && view.auth_url.is_none() && view.user_code.is_none());
        client.refresh().await;
        assert_eq!(client.snapshot().phase, LoginPhase::Cancelled);
        assert!(client.retired());
    }
    #[tokio::test]
    async fn stale_cancellation_cannot_cancel_another_connection_or_attempt() {
        let (launch, _dir) = fake("pending");
        let client = AccountClient::start(&launch).await.unwrap();
        let pending = client.begin("browser").await.unwrap();
        assert!(client
            .cancel_attempt("old-connection", "login-1")
            .await
            .is_err());
        assert!(client
            .cancel_attempt(&pending.connection_id, "old-login")
            .await
            .is_err());
        assert_eq!(client.snapshot().phase, LoginPhase::Pending);
        let cancelled = client
            .cancel_attempt(&pending.connection_id, "login-1")
            .await
            .unwrap();
        assert_eq!(cancelled.phase, LoginPhase::Cancelled);
    }

    #[tokio::test]
    async fn failed_login_and_unsupported_flows_are_explicit() {
        let (launch, _dir) = fake("failed");
        let client = AccountClient::start(&launch).await.unwrap();
        client.begin("browser").await.unwrap();
        wait_for(&client, LoginPhase::Failed).await;
        assert_eq!(client.snapshot().error.as_deref(), Some("Denied"));
        client.shutdown().await;
        let (launch, _dir) = fake("device-only");
        let client = AccountClient::start(&launch).await.unwrap();
        assert_eq!(
            client.begin("browser").await.unwrap().phase,
            LoginPhase::Unsupported
        );
        assert_eq!(
            client.begin("device").await.unwrap().phase,
            LoginPhase::Pending
        );
        client.cancel().await;
        let (launch, _dir) = fake("legacy");
        let client = AccountClient::start(&launch).await.unwrap();
        assert!(!client.snapshot().requirements_supported);
        assert_eq!(
            client.begin("device").await.unwrap().phase,
            LoginPhase::Unsupported
        );
        client.shutdown().await;
    }
    #[tokio::test]
    async fn saved_credentials_and_account_switches_refresh_requirements() {
        let (launch, _dir) = fake("saved");
        let client = AccountClient::start(&launch).await.unwrap();
        assert_eq!(client.snapshot().phase, LoginPhase::Authenticated);
        assert_eq!(
            client.snapshot().requirements.unwrap()["allowedModelProviders"],
            json!(["openai"])
        );
        let mut events = client.subscribe();
        client.call("fixture/switch", json!({})).await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if client
                    .snapshot()
                    .account
                    .as_ref()
                    .and_then(|a| a.email.as_deref())
                    == Some("second@example.test")
                {
                    break;
                }
                events.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
        assert_eq!(
            client.snapshot().requirements.unwrap()["allowedModelProviders"],
            json!(["bedrock"])
        );
        client.shutdown().await;
    }
    #[tokio::test]
    async fn unsafe_handoffs_close_the_login_connection() {
        let (launch, _dir) = fake("unsafe");
        let client = AccountClient::start(&launch).await.unwrap();
        let view = client.begin("browser").await.unwrap();
        assert_eq!(view.phase, LoginPhase::Failed);
        assert!(view.auth_url.is_none());
        assert!(client.retired());
    }
}

#[cfg(test)]
mod installed_tests {
    use super::*;
    #[tokio::test]
    #[ignore = "requires installed Codex CLI; reads saved account only, never starts login"]
    async fn installed_codex_reads_saved_account_without_a_login() {
        let launch = crate::appserver::launch(crate::which::resolve_for_spawn(
            std::path::Path::new("codex"),
        ));
        let client = AccountClient::start(&launch).await.unwrap();
        assert!(matches!(
            client.snapshot().phase,
            LoginPhase::SignedOut | LoginPhase::Authenticated
        ));
        assert!(client.snapshot().login_id.is_none());
        assert!(client.snapshot().auth_url.is_none());
        client.shutdown().await;
    }
}
