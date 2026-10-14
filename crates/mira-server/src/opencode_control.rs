//! Steering side channel to the HTTP server hosted by the same ACP process.
//! Never retry an ambiguous POST: a timeout may mean the inbox accepted it.
use mira_acp::driver::LaunchConfig;
use mira_core::ImageData;
use serde_json::json;

pub struct OpenCodeControl {
    client: reqwest::Client,
    base: String,
    password: String,
    bridge_files: Option<tempfile::TempDir>,
}
impl OpenCodeControl {
    fn probe_command(launch: &LaunchConfig) -> tokio::process::Command {
        let mut command =
            tokio::process::Command::new(mira_acp::which::resolve_for_spawn(&launch.program));
        command.env_clear().kill_on_drop(true);
        for key in mira_sandbox::safe_child_env() {
            if !mira_acp::which::env_denied(key, &launch.env_deny) {
                if let Ok(value) = std::env::var(key) {
                    command.env(key, value);
                }
            }
        }
        if let Some(path) = mira_acp::which::effective_path() {
            command.env("PATH", path);
        }
        for (key, value) in &launch.env {
            if !mira_acp::which::env_denied(key, &launch.env_deny) {
                command.env(key, value);
            }
        }
        command
    }
    pub async fn supported(launch: &LaunchConfig) -> bool {
        let mut command = Self::probe_command(launch);
        command.args(["acp", "--help"]);
        match tokio::time::timeout(std::time::Duration::from_secs(5), command.output()).await {
            Ok(Ok(output)) if output.status.success() => {
                let help = String::from_utf8_lossy(&output.stdout);
                help.contains("--port") && help.contains("--hostname")
            }
            _ => false,
        }
    }
    /// V1 hosts HTTP on ACP flags; V2 offers an in-process plugin prompt API.
    pub async fn prepare(launch: &mut LaunchConfig) -> Result<Option<Self>, String> {
        if Self::supported(launch).await {
            return Self::configure(launch).map(Some);
        }
        let mut command = Self::probe_command(launch);
        command.arg("--version");
        let version =
            match tokio::time::timeout(std::time::Duration::from_secs(5), command.output()).await {
                Ok(Ok(output)) if output.status.success() => {
                    String::from_utf8_lossy(&output.stdout).trim().to_string()
                }
                _ => return Ok(None),
            };
        if !version
            .split_whitespace()
            .any(|part| part.trim_start_matches('v').starts_with("2."))
        {
            return Ok(None);
        }
        let mut control = Self::configure_client()?;
        let dir = tempfile::Builder::new()
            .prefix("mira-opencode-steering-")
            .tempdir()
            .map_err(|e| e.to_string())?;
        let plugin = dir.path().join("index.js");
        let auth = dir.path().join("auth");
        std::fs::write(&plugin, include_str!("opencode_bridge.mjs")).map_err(|e| e.to_string())?;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        {
            use std::io::Write;
            options
                .open(&auth)
                .map_err(|e| e.to_string())?
                .write_all(control.password.as_bytes())
                .map_err(|e| e.to_string())?;
        }
        let mut config: serde_json::Value = serde_json::from_str(
            launch
                .env
                .get("OPENCODE_CONFIG_CONTENT")
                .map(String::as_str)
                .unwrap_or("{}"),
        )
        .map_err(|_| "Invalid OpenCode inline configuration")?;
        let port = reqwest::Url::parse(&control.base)
            .map_err(|e| e.to_string())?
            .port()
            .ok_or("Missing bridge port")?;
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"name":"mira-opencode-steering","type":"module","main":"./index.js"}"#,
        )
        .map_err(|e| e.to_string())?;
        let entry = json!({"package":dir.path(),"options":{"port":port,"authFile":auth}});
        config
            .as_object_mut()
            .ok_or("OpenCode inline configuration must be an object")?
            .entry("plugins")
            .or_insert(json!([]))
            .as_array_mut()
            .ok_or("OpenCode plugins must be an array")?
            .push(entry);
        launch
            .env
            .insert("OPENCODE_CONFIG_CONTENT".into(), config.to_string());
        control.bridge_files = Some(dir);
        Ok(Some(control))
    }
    pub async fn ready(&self) -> bool {
        if self.bridge_files.is_none() {
            return true;
        }
        for _ in 0..10 {
            if let Ok(response) = self
                .client
                .get(format!("{}/health", self.base))
                .basic_auth("opencode", Some(&self.password))
                .timeout(std::time::Duration::from_millis(300))
                .send()
                .await
            {
                if response.status().is_success()
                    && response
                        .json::<serde_json::Value>()
                        .await
                        .ok()
                        .is_some_and(|v| v["bridge"] == "mira.steering.v2")
                {
                    return true;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        false
    }
    fn configure_client() -> Result<Self, String> {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).map_err(|e| e.to_string())?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        Ok(Self {
            client: reqwest::Client::builder()
                .no_proxy()
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .map_err(|e| e.to_string())?,
            base: format!("http://127.0.0.1:{port}"),
            password: uuid::Uuid::new_v4().to_string(),
            bridge_files: None,
        })
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
            bridge_files: None,
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
        if self.bridge_files.is_some() {
            url.set_path("/mira/steer");
        } else {
            url.path_segments_mut()
                .map_err(|_| "invalid control URL")?
                .extend(["api", "session", session, "prompt"]);
        }
        let files: Vec<_> = images
            .iter()
            .map(|i| json!({"uri":format!("data:{};base64,{}",i.media_type,i.data)}))
            .collect();
        let body = json!({"sessionID":session,"id":format!("msg_mira_steer_{id}"), "prompt":{"text":text,"files":files},"delivery":"steer"});
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
        if self.bridge_files.is_none() && response.status() == reqwest::StatusCode::NOT_FOUND {
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

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    #[ignore = "requires an installed OpenCode v2 CLI; does not send a provider prompt"]
    async fn installed_v2_bridge_initializes_inside_the_acp_owned_server() {
        use mira_acp::driver::AcpDriver;
        let binary = std::env::var("MIRA_TEST_OPENCODE_BIN").expect("set MIRA_TEST_OPENCODE_BIN");
        let driver = mira_acp::drivers::OpenCodeDriver;
        let mut launch = driver.resolve(
            &Default::default(),
            mira_acp::driver::PermissionMode::Ask,
            binary.into(),
        );
        let control = OpenCodeControl::prepare(&mut launch)
            .await
            .unwrap()
            .expect("v2 bridge");
        assert!(control.bridge_files.is_some());
        let agent = mira_acp::process::start(mira_acp::process::StartSpec {
            launch: &launch,
            caps: Some(driver.client_caps()),
            files: std::sync::Arc::new(mira_acp::DenyAll),
            terminals: std::sync::Arc::new(mira_acp::DenyAll),
            permissions: std::sync::Arc::new(mira_acp::DenyAll),
            events: std::sync::Arc::new(mira_acp::DenyAll),
            cwd: Some(std::path::Path::new("/tmp")),
            mira_mcp: None,
            resume_session_id: None,
        })
        .await
        .expect("ACP startup");
        let ready = control.ready().await;
        agent.shutdown().await;
        assert!(
            ready,
            "plugin must bind its authenticated control endpoint inside the real v2 server"
        );
    }
}

#[cfg(test)]
mod live_steering_tests {
    use super::*;
    use mira_acp::driver::AcpDriver;
    struct Events(tokio::sync::mpsc::UnboundedSender<mira_acp::events::NormalizedEvent>);
    #[async_trait::async_trait]
    impl mira_acp::EventPort for Events {
        async fn emit(&self, event: mira_acp::events::NormalizedEvent) {
            let _ = self.0.send(event);
        }
    }
    #[tokio::test]
    #[ignore = "requires installed OpenCode v2 and network access to its free model"]
    async fn installed_v2_steers_an_active_prompt_without_cancelling() {
        let binary = std::env::var("MIRA_TEST_OPENCODE_BIN").expect("set MIRA_TEST_OPENCODE_BIN");
        let cwd = tempfile::tempdir().unwrap();
        let driver = mira_acp::drivers::OpenCodeDriver;
        let mut launch = driver.resolve(
            &Default::default(),
            mira_acp::driver::PermissionMode::Ask,
            binary.into(),
        );
        let control = OpenCodeControl::prepare(&mut launch)
            .await
            .unwrap()
            .expect("v2 bridge");
        let (tx, mut events) = tokio::sync::mpsc::unbounded_channel();
        let agent = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            mira_acp::process::start(mira_acp::process::StartSpec {
                launch: &launch,
                caps: Some(driver.client_caps()),
                files: std::sync::Arc::new(mira_acp::DenyAll),
                terminals: std::sync::Arc::new(mira_acp::DenyAll),
                permissions: std::sync::Arc::new(mira_acp::DenyAll),
                events: std::sync::Arc::new(Events(tx)),
                cwd: Some(cwd.path()),
                mira_mcp: None,
                resume_session_id: None,
            }),
        )
        .await
        .unwrap()
        .unwrap();
        let outcome=async {
            assert!(control.ready().await);
            let option=agent.session().config_options().iter().find(|o|o["category"]=="model").and_then(|o|o["id"].as_str()).unwrap_or("model").to_owned();
            agent.session().set_config_option(&agent.session_id().unwrap(),&option,json!("opencode/big-pickle")).await.map_err(|e|e.to_string())?;
            let main=agent.clone();
            let mut turn=tokio::spawn(async move { main.prompt(vec![serde_json::from_value(json!({"type":"text","text":"Explain how a Unix shell pipeline works in about 500 words. Do not call tools."})).unwrap()]).await });
            let seen=tokio::time::timeout(std::time::Duration::from_secs(45),async {
                loop { tokio::select! {
                    result=&mut turn => return Err(format!("initial prompt ended before steering: {result:?}")),
                    event=events.recv()=> match event { Some(e) if matches!(e.event,mira_acp::events::MiraEvent::AssistantText{..})=>return Ok(()),None=>return Err("event stream closed".into()),_=>{} }
                }}
            }).await.map_err(|_|"no streamed reply within 45 seconds".to_owned())?;
            seen?;
            control.steer(&agent.session_id().unwrap(),"live-verification","Stop the explanation and reply exactly MIRA_STEER_OK.",&[]).await?;
            assert!(!turn.is_finished(),"steering must be admitted while the original ACP prompt remains active");
            let result=tokio::time::timeout(std::time::Duration::from_secs(45),&mut turn).await.map_err(|_|"steered prompt timed out".to_owned())?.map_err(|e|e.to_string())?.map_err(|e|e.to_string())?;
            assert!(!result.was_cancelled,"native steering must not cancel the original prompt");
            let mut text=String::new();while let Ok(event)=events.try_recv(){if let mira_acp::events::MiraEvent::AssistantText{text:chunk,..}=event.event{text.push_str(&chunk);}}
            assert!(text.contains("MIRA_STEER_OK"),"steering instruction must reach the model");
            Ok::<(),String>(())
        }.await;
        agent.shutdown().await;
        outcome.unwrap();
    }
}
