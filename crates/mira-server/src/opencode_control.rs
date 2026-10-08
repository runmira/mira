//! Steering side channel to the HTTP server hosted by the same ACP process.
//! Never retry an ambiguous POST: a timeout may mean the inbox accepted it.
use mira_acp::driver::LaunchConfig;
use mira_core::ImageData;
use serde_json::json;

pub struct OpenCodeControl {
    client: reqwest::Client,
    base: String,
    password: String,
}
impl OpenCodeControl {
    pub async fn supported(launch: &LaunchConfig) -> bool {
        let mut command = tokio::process::Command::new(&launch.program);
        command
            .args(["acp", "--help"])
            .envs(&launch.env)
            .kill_on_drop(true);
        for key in &launch.env_deny {
            command.env_remove(key);
        }
        match tokio::time::timeout(std::time::Duration::from_secs(5), command.output()).await {
            Ok(Ok(output)) if output.status.success() => {
                let help = String::from_utf8_lossy(&output.stdout);
                help.contains("--port") && help.contains("--hostname")
            }
            _ => false,
        }
    }
    pub fn configure(launch: &mut LaunchConfig) -> Result<Self, String> {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).map_err(|e| e.to_string())?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let password = uuid::Uuid::new_v4().to_string();
        launch.args.extend([
            "--hostname".into(),
            "127.0.0.1".into(),
            "--port".into(),
            port.to_string(),
        ]);
        launch
            .env
            .insert("OPENCODE_SERVER_PASSWORD".into(), password.clone());
        launch
            .env
            .insert("OPENCODE_SERVER_USERNAME".into(), "opencode".into());
        launch.secret_env.push("OPENCODE_SERVER_PASSWORD".into());
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            client,
            base: format!("http://127.0.0.1:{port}"),
            password,
        })
    }
    pub async fn steer(
        &self,
        session: &str,
        id: &str,
        text: &str,
        images: &[ImageData],
    ) -> Result<(), String> {
        let mut url = reqwest::Url::parse(&self.base).map_err(|e| e.to_string())?;
        url.path_segments_mut()
            .map_err(|_| "invalid control URL")?
            .extend(["api", "session", session, "prompt"]);
        let files: Vec<_> = images
            .iter()
            .map(|i| json!({"uri":format!("data:{};base64,{}",i.media_type,i.data)}))
            .collect();
        let body = json!({"id":format!("msg_mira_steer_{id}"), "prompt":{"text":text,"files":files},"delivery":"steer"});
        let response = self
            .client
            .post(url)
            .basic_auth("opencode", Some(&self.password))
            .json(&body)
            .send()
            .await
            .map_err(|_| {
                "Steering delivery could not be confirmed. Check the transcript before retrying."
                    .to_string()
            })?;
        // Only a definitive missing route permits the older API fallback.
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            let mut legacy = reqwest::Url::parse(&self.base).map_err(|e| e.to_string())?;
            legacy
                .path_segments_mut()
                .map_err(|_| "invalid control URL")?
                .extend(["session", session, "prompt_async"]);
            let mut parts = vec![json!({"type":"text", "text":text})];
            parts.extend(images.iter().map(|i| json!({"type":"file", "mime":i.media_type,"url":format!("data:{};base64,{}",i.media_type,i.data)})));
            let response = self.client.post(legacy).basic_auth("opencode",Some(&self.password))
                .json(&json!({"messageID":format!("msg_mira_steer_{id}"),"parts":parts})).send().await
                .map_err(|_| "Steering delivery could not be confirmed. Check the transcript before retrying.".to_string())?;
            if !response.status().is_success() {
                return Err(format!("OpenCode refused steering ({})", response.status()));
            }
        } else if !response.status().is_success() {
            return Err(format!("OpenCode refused steering ({})", response.status()));
        }
        Ok(())
    }
}
