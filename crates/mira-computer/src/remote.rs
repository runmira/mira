//! Scoped Mira-to-Mira computer transport. Tokens never enter browser storage or YAML.
use crate::{
    backend::cua::{validate_endpoint, TargetWindow},
    ComputerAction, ComputerController, ComputerError, Delivery, Outcome,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
#[derive(Clone, Serialize, Deserialize)]
pub struct Connection {
    pub id: String,
    pub name: String,
    pub platform: String,
    pub endpoint: String,
    pub device_id: String,
    pub token: String,
}
impl std::fmt::Debug for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connection")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("endpoint", &self.endpoint)
            .finish()
    }
}
pub fn credentials_path() -> PathBuf {
    mira_config::global_path().with_file_name("computer-credentials.json")
}
pub fn load_connections() -> Result<Vec<Connection>, ComputerError> {
    match std::fs::read(credentials_path()) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|_| {
            ComputerError::Remote(
                "Computer credentials could not be read. Re-pair the computer in Settings.".into(),
            )
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
        Err(_) => Err(ComputerError::Remote(
            "Computer credentials are not accessible.".into(),
        )),
    }
}
pub fn save_connections(connections: &[Connection]) -> Result<(), ComputerError> {
    let path = credentials_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|_| {
            ComputerError::Remote("Could not create the computer credential store.".into())
        })?;
    }
    let bytes = serde_json::to_vec(connections)
        .map_err(|_| ComputerError::Remote("Could not encode computer credentials.".into()))?;
    mira_config::write_private(&path, &bytes)
        .map_err(|_| ComputerError::Remote("Could not save computer credentials.".into()))
}
pub fn client() -> Result<reqwest::Client, ComputerError> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(60))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| ComputerError::Remote("Could not initialize the computer connection.".into()))
}
pub async fn json_response<T: serde::de::DeserializeOwned>(
    mut response: reqwest::Response,
) -> Result<T, ComputerError> {
    let status = response.status();
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| ComputerError::Remote(e.without_url().to_string()))?
    {
        if bytes.len() + chunk.len() > 24 * 1024 * 1024 {
            return Err(ComputerError::Remote(
                "Computer response exceeded the size limit.".into(),
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    if !status.is_success() {
        let message = serde_json::from_slice::<serde_json::Value>(&bytes)
            .ok()
            .and_then(|v| v["error"].as_str().map(str::to_owned))
            .unwrap_or_else(|| format!("Computer connection failed ({status})."));
        return Err(ComputerError::Remote(message));
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| ComputerError::Remote("Invalid response from the paired computer.".into()))
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ControlRequest {
    pub action: ComputerAction,
    pub delivery: Delivery,
    pub window: Option<TargetWindow>,
    #[serde(default)]
    pub run_id: String,
}
impl ControlRequest {
    pub fn validate(&self) -> Result<(), ComputerError> {
        if self.run_id.len() > 64
            || !self
                .run_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-')
        {
            return Err(ComputerError::InvalidArgs(
                "Invalid computer run id.".into(),
            ));
        }
        match &self.action {
            ComputerAction::Wait { seconds } | ComputerAction::HoldKey { seconds, .. }
                if !seconds.is_finite() || !(0.0..=30.0).contains(seconds) =>
            {
                return Err(ComputerError::InvalidArgs(
                    "Duration must be between 0 and 30 seconds.".into(),
                ))
            }
            ComputerAction::Type(text) if text.len() > 1024 * 1024 => {
                return Err(ComputerError::InvalidArgs("Text is too long.".into()))
            }
            ComputerAction::Click { count, .. } if !(1..=3).contains(count) => {
                return Err(ComputerError::InvalidArgs(
                    "Click count must be between 1 and 3.".into(),
                ))
            }
            ComputerAction::Scroll { amount, .. } if *amount == 0 || *amount > 100 => {
                return Err(ComputerError::InvalidArgs(
                    "Scroll amount must be between 1 and 100.".into(),
                ))
            }
            _ => {}
        }
        Ok(())
    }
}
pub struct RemoteComputer {
    client: reqwest::Client,
    connection: Connection,
    delivery: Delivery,
    window: Option<TargetWindow>,
    run_id: String,
    blocked: AtomicBool,
}
impl RemoteComputer {
    pub fn new(
        connection: Connection,
        delivery: Delivery,
        window: Option<TargetWindow>,
    ) -> Result<Arc<Self>, ComputerError> {
        validate_endpoint(&connection.endpoint)?;
        Ok(Arc::new(Self {
            client: client()?,
            connection,
            delivery,
            window,
            run_id: uuid::Uuid::new_v4().simple().to_string(),
            blocked: AtomicBool::new(false),
        }))
    }
}
#[async_trait]
impl ComputerController for RemoteComputer {
    async fn execute(&self, action: &ComputerAction) -> Result<Outcome, ComputerError> {
        let reading = matches!(
            action,
            ComputerAction::Screenshot
                | ComputerAction::CursorPosition
                | ComputerAction::Zoom { .. }
                | ComputerAction::Wait { .. }
        );
        if !reading && self.blocked.load(Ordering::SeqCst) {
            return Err(ComputerError::Remote("Computer input is paused after an unconfirmed delivery. Reconnect in Computer settings before continuing.".into()));
        }
        let request = ControlRequest {
            action: action.clone(),
            delivery: self.delivery,
            window: self.window.clone(),
            run_id: self.run_id.clone(),
        };
        request.validate()?;
        let response=self.client.post(format!("{}/api/computer/control",self.connection.endpoint.trim_end_matches('/'))).bearer_auth(&self.connection.token).json(&request).send().await.map_err(|_|{
            if !reading{self.blocked.store(true,Ordering::SeqCst);}ComputerError::Remote("Computer delivery could not be confirmed. Do not repeat the action; reconnect and inspect the screen before continuing.".into())
        })?;
        let may_have_run = response.status().is_success() || response.status().is_server_error();
        let result = json_response(response).await;
        if !reading && may_have_run && result.is_err() {
            self.blocked.store(true, Ordering::SeqCst);
        }
        result
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn remote_requests_reject_unbounded_or_negative_durations_before_input() {
        let r = ControlRequest {
            action: ComputerAction::HoldKey {
                keys: "shift".into(),
                seconds: -1.0,
            },
            delivery: Delivery::Foreground,
            window: None,
            run_id: "test".into(),
        };
        assert!(r.validate().is_err());
        let r = ControlRequest {
            action: ComputerAction::Wait { seconds: 31.0 },
            ..r
        };
        assert!(r.validate().is_err());
    }
}
