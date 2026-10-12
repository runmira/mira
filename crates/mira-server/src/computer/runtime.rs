use super::grants::Grants;
use mira_computer::{
    backend::cua::{CuaBackend, CuaOptions, TargetWindow},
    Computer, ComputerController, ComputerError, ComputerOptions, Delivery,
};
use mira_tools::{Tool, ToolSource};
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};
use tokio::sync::{Mutex, RwLock as AsyncRwLock};

struct ActiveController {
    current: AsyncRwLock<Option<Arc<dyn ComputerController>>>,
    input: Arc<Mutex<()>>,
}
#[async_trait::async_trait]
impl ComputerController for ActiveController {
    async fn execute(
        &self,
        action: &mira_computer::ComputerAction,
    ) -> Result<mira_computer::Outcome, ComputerError> {
        let _input = self.input.lock().await;
        let current = self.current.read().await;
        current
            .as_ref()
            .ok_or_else(|| {
                ComputerError::Remote(
                    "Computer use is disabled or disconnected. Check Computer settings.".into(),
                )
            })?
            .execute(action)
            .await
    }
}

pub struct Runtime {
    active: Arc<ActiveController>,
    tool: Arc<dyn Tool>,
    pub config: RwLock<mira_config::ComputerUseConfig>,
    pub changes: Mutex<()>,
    pub grants: Grants,
    pub local_options: CuaOptions,
    input: Arc<Mutex<()>>,
    shared: Mutex<HashMap<String, Arc<Computer>>>,
    #[cfg(test)]
    test_backend: Option<Arc<dyn mira_computer::ComputerBackend>>,
}
impl ToolSource for Runtime {
    fn tools(&self) -> Vec<Arc<dyn Tool>> {
        if self
            .config
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .enabled()
        {
            vec![self.tool.clone()]
        } else {
            vec![]
        }
    }
}
impl Runtime {
    pub async fn new(config: mira_config::ComputerUseConfig) -> Result<Arc<Self>, String> {
        let input = Arc::new(Mutex::new(()));
        let active = Arc::new(ActiveController {
            current: AsyncRwLock::new(None),
            input: input.clone(),
        });
        let runtime = Arc::new(Self {
            tool: Arc::new(mira_tools::builtin::computer::ComputerTool::new(
                active.clone(),
                None,
            )),
            active,
            config: RwLock::new(config.clone()),
            changes: Mutex::new(()),
            grants: Grants::load(
                mira_config::global_path().with_file_name("computer-grants.json"),
            )?,
            local_options: CuaOptions::default().token_from_env("CUA_ENV_TOKEN"),
            input,
            shared: Mutex::new(HashMap::new()),
            #[cfg(test)]
            test_backend: None,
        });
        if config.enabled() {
            match mira_tools::builtin::computer_controller(&config, None).await {
                Ok(controller) => *runtime.active.current.write().await = Some(controller),
                Err(e) => tracing::warn!(error = %e, "computer unavailable; no local fallback"),
            }
        }
        Ok(runtime)
    }
    #[cfg(test)]
    pub fn ephemeral(backend: Arc<dyn mira_computer::ComputerBackend>) -> Arc<Self> {
        let input = Arc::new(Mutex::new(()));
        let active = Arc::new(ActiveController {
            current: AsyncRwLock::new(None),
            input: input.clone(),
        });
        Arc::new(Self {
            tool: Arc::new(mira_tools::builtin::computer::ComputerTool::new(
                active.clone(),
                None,
            )),
            active,
            config: RwLock::new(Default::default()),
            changes: Mutex::new(()),
            grants: Grants::ephemeral(),
            local_options: Default::default(),
            input,
            shared: Mutex::new(HashMap::new()),
            test_backend: Some(backend),
        })
    }
    pub async fn apply(
        &self,
        config: mira_config::ComputerUseConfig,
        controller: Option<Arc<dyn ComputerController>>,
    ) {
        *self.active.current.write().await = controller;
        *self.config.write().unwrap_or_else(|e| e.into_inner()) = config;
    }
    pub async fn local_cua(
        &self,
        delivery: Delivery,
        window: Option<TargetWindow>,
    ) -> Result<CuaBackend, ComputerError> {
        CuaBackend::connect(&CuaOptions {
            delivery,
            window,
            ..self.local_options.clone()
        })
        .await
    }
    pub async fn shared_control(
        &self,
        token: &str,
        request: mira_computer::remote::ControlRequest,
    ) -> Result<mira_computer::Outcome, ComputerError> {
        request.validate()?;
        let _input = self.input.lock().await;
        let grant = self
            .grants
            .authenticate(token)
            .ok_or_else(|| ComputerError::Remote("Computer pairing has been revoked.".into()))?;
        let device = grant.id;
        if grant.backend == "native"
            && (request.window.is_some()
                || (request.delivery != Delivery::Foreground
                    && !matches!(
                        request.action,
                        mira_computer::ComputerAction::Screenshot
                            | mira_computer::ComputerAction::CursorPosition
                            | mira_computer::ComputerAction::Zoom { .. }
                            | mira_computer::ComputerAction::Wait { .. }
                    )))
        {
            return Err(ComputerError::Unsupported(
                "This computer supports foreground input only. Select Foreground to continue."
                    .into(),
            ));
        }
        let key = format!(
            "{device}:{}:{}:{}:{}",
            request.run_id,
            request.delivery,
            request.window.as_ref().map(|w| w.id.as_str()).unwrap_or(""),
            request.window.as_ref().map(|w| w.epoch).unwrap_or(0)
        );
        let mut shared = self.shared.lock().await;
        let computer = if let Some(computer) = shared.get(&key) {
            computer.clone()
        } else {
            if shared.len() >= 64 {
                shared.clear();
            }
            #[cfg(test)]
            let injected = self.test_backend.clone();
            #[cfg(not(test))]
            let injected: Option<Arc<dyn mira_computer::ComputerBackend>> = None;
            let backend: Arc<dyn mira_computer::ComputerBackend> = if let Some(backend) = injected {
                backend
            } else if grant.backend == "native" {
                Arc::from(mira_computer::detect()?)
            } else {
                Arc::new(self.local_cua(request.delivery, request.window).await?)
            };
            #[cfg(test)]
            let backend = self.test_backend.clone().unwrap_or(backend);
            let options = ComputerOptions {
                settle: if cfg!(test) {
                    std::time::Duration::ZERO
                } else {
                    std::time::Duration::from_millis(600)
                },
                ..ComputerOptions::default()
            };
            let computer = Arc::new(Computer::new(backend, options));
            shared.insert(key, computer.clone());
            computer
        };
        drop(shared);
        computer.execute(&request.action).await
    }
    pub async fn revoke(&self, id: &str) -> Result<(), String> {
        let _input = self.input.lock().await;
        self.grants.revoke(id)?;
        self.shared
            .lock()
            .await
            .retain(|key, _| !key.starts_with(&format!("{id}:")));
        Ok(())
    }
}
